#![allow(unexpected_cfgs)]
use anchor_lang::prelude::*;
use anchor_lang::solana_program::system_instruction;

declare_id!("8W2ByTE2SGze5yPDabKqPXVYLCikYCxqjZE47i5F2zMV");

#[program]
pub mod auction_gpt {
    use super::*;

    pub fn start(
        ctx: Context<StartCtx>,
        auctioned_object: String,
        duration_slots: u64,
        starting_bid: u64,
    ) -> Result<()> {
        let auction = &mut ctx.accounts.auction_info;
        let clock = Clock::get()?;

        auction.seller = *ctx.accounts.seller.key;
        auction.highest_bidder = *ctx.accounts.seller.key;
        auction.highest_bid = starting_bid;
        auction.object = auctioned_object.clone();
        auction.end_time = clock.slot + duration_slots;

        Ok(())
    }

    pub fn bid(
        ctx: Context<BidCtx>,
        auctioned_object: String,
        amount_to_deposit: u64,
    ) -> Result<()> {
        let auction = &mut ctx.accounts.auction_info;
        let bidder = &ctx.accounts.bidder;

        let clock = Clock::get()?;
        require!(clock.slot <= auction.end_time, AuctionError::AuctionEnded);
        require!(amount_to_deposit > auction.highest_bid, AuctionError::BidTooLow);

        let previous_bidder = auction.highest_bidder;
        let previous_amount = auction.highest_bid;

        // Transfer new bid from bidder to auction vault
        anchor_lang::solana_program::program::invoke(
            &system_instruction::transfer(
                &bidder.key(),
                &ctx.accounts.auction_vault.key(),
                amount_to_deposit,
            ),
            &[
                bidder.to_account_info(),
                ctx.accounts.auction_vault.to_account_info(),
            ],
        )?;

        // Refund previous highest bidder if not seller
        if previous_bidder != auction.seller {
            // Compute bump dynamically
            let (_vault_pda, vault_bump) =
                Pubkey::find_program_address(&[auctioned_object.as_bytes(), b"vault"], ctx.program_id);
            let vault_seeds: &[&[u8]] = &[
                auctioned_object.as_bytes(),
                b"vault",
                &[vault_bump],
            ];

            anchor_lang::solana_program::program::invoke_signed(
                &system_instruction::transfer(
                    &ctx.accounts.auction_vault.key(),
                    &previous_bidder,
                    previous_amount,
                ),
                &[ctx.accounts.auction_vault.to_account_info()],
                &[vault_seeds],
            )?;
        }

        auction.highest_bid = amount_to_deposit;
        auction.highest_bidder = *bidder.key;

        Ok(())
    }

    pub fn end(ctx: Context<EndCtx>, auctioned_object: String) -> Result<()> {
        let auction = &ctx.accounts.auction_info;
        let clock = Clock::get()?;

        require!(ctx.accounts.seller.key() == auction.seller, AuctionError::NotSeller);
        require!(clock.slot > auction.end_time, AuctionError::AuctionNotEnded);

        // Transfer highest bid to seller
        if auction.highest_bid > 0 {
            let (_vault_pda, vault_bump) =
                Pubkey::find_program_address(&[auctioned_object.as_bytes(), b"vault"], ctx.program_id);
            let vault_seeds: &[&[u8]] = &[
                auctioned_object.as_bytes(),
                b"vault",
                &[vault_bump],
            ];

            anchor_lang::solana_program::program::invoke_signed(
                &system_instruction::transfer(
                    &ctx.accounts.auction_vault.key(),
                    &ctx.accounts.seller.key(),
                    auction.highest_bid,
                ),
                &[ctx.accounts.auction_vault.to_account_info()],
                &[vault_seeds],
            )?;
        }

        // Close vault and refund remaining lamports to seller
        **ctx.accounts.seller.lamports.borrow_mut() += ctx.accounts.auction_vault.lamports();
        **ctx.accounts.auction_vault.lamports.borrow_mut() = 0;

        Ok(())
    }
}

#[derive(Accounts)]
#[instruction(auctioned_object: String)]
pub struct StartCtx<'info> {
    #[account(mut)]
    pub seller: Signer<'info>,

    #[account(
        init,
        payer = seller,
        space = 8 + 32 + 32 + 8 + 8 + 64,
        seeds = [auctioned_object.as_bytes()],
        bump
    )]
    pub auction_info: Account<'info, AuctionInfo>,

    /// CHECK: Vault PDA to hold lamports; safe
    #[account(
        init,
        payer = seller,
        seeds = [auctioned_object.as_bytes(), b"vault"],
        bump,
        space = 0
    )]
    pub auction_vault: UncheckedAccount<'info>,

    /// System program is required for init
    pub system_program: Program<'info, System>,
}


#[derive(Accounts)]
#[instruction(auctioned_object: String)]
pub struct BidCtx<'info> {
    #[account(mut)]
    pub bidder: Signer<'info>,

    #[account(mut, seeds = [auctioned_object.as_bytes()], bump)]
    pub auction_info: Account<'info, AuctionInfo>,

    /// CHECK: PDA vault to hold lamports; safe
    #[account(mut, seeds = [auctioned_object.as_bytes(), b"vault"], bump)]
    pub auction_vault: UncheckedAccount<'info>,
}

#[derive(Accounts)]
#[instruction(auctioned_object: String)]
pub struct EndCtx<'info> {
    #[account(mut)]
    pub seller: Signer<'info>,

    #[account(mut, has_one = seller, seeds = [auctioned_object.as_bytes()], bump, close = seller)]
    pub auction_info: Account<'info, AuctionInfo>,

    /// CHECK: PDA vault to hold lamports; safe
    #[account(mut, seeds = [auctioned_object.as_bytes(), b"vault"], bump)]
    pub auction_vault: UncheckedAccount<'info>,
}

#[account]
pub struct AuctionInfo {
    pub seller: Pubkey,
    pub highest_bidder: Pubkey,
    pub end_time: u64,
    pub highest_bid: u64,
    pub object: String, // max 64 bytes
}

#[error_code]
pub enum AuctionError {
    #[msg("Bid too low.")]
    BidTooLow,
    #[msg("Auction already ended.")]
    AuctionEnded,
    #[msg("Auction has not ended yet.")]
    AuctionNotEnded,
    #[msg("Only seller can call this.")]
    NotSeller,
}
