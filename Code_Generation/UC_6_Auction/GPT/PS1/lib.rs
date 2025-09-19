use anchor_lang::prelude::*;
use anchor_lang::system_program;

declare_id!("8W2ByTE2SGze5yPDabKqPXVYLCikYCxqjZE47i5F2zMV");

const MAX_OBJECT_LEN: usize = 200;
const VAULT_SEED: &[u8] = b"vault";

#[program]
pub mod anchor_auction {
    use super::*;

    /// Start an auction. Seller must sign.
    pub fn start(
        ctx: Context<StartCtx>,
        auctioned_object: String,
        duration_slots: u64,
        starting_bid: u64,
    ) -> Result<()> {
        require!(
            auctioned_object.len() <= MAX_OBJECT_LEN,
            AuctionError::ObjectTooLong
        );

        // initialize AuctionInfo stored in auction_info PDA
        let auction_info = &mut ctx.accounts.auction_info;
        let clock = Clock::get()?; // get current slot
        auction_info.seller = *ctx.accounts.seller.key;
        auction_info.highest_bidder = *ctx.accounts.seller.key; // seller placeholder
        auction_info.end_time = clock.slot.saturating_add(duration_slots);
        auction_info.highest_bid = starting_bid;
        auction_info.object = auctioned_object;

        // auction_vault is created by Anchor (init in StartCtx). No funds are moved here.
        Ok(())
    }

    /// Place a bid. Bidder must sign. Refund previous highest bidder immediately (if they are not the seller placeholder).
    pub fn bid(
        ctx: Context<BidCtx>,
        auctioned_object: String,
        amount_to_deposit: u64,
    ) -> Result<()> {
        require!(
            auctioned_object.len() <= MAX_OBJECT_LEN,
            AuctionError::ObjectTooLong
        );

        let auction_info = &mut ctx.accounts.auction_info;
        let bidder = &ctx.accounts.bidder;
        let prev_high_acc = &ctx.accounts.current_highest_bidder;
        let clock = Clock::get()?;

        // Verify auction_info PDA matches provided auctioned_object
        let (expected_auction_pda, _auction_bump) =
            Pubkey::find_program_address(&[auctioned_object.as_bytes()], ctx.program_id);
        require!(expected_auction_pda == auction_info.key(), AuctionError::InvalidPDA);

        // Verify vault PDA
        let (expected_vault_pda, vault_bump) =
            Pubkey::find_program_address(&[auctioned_object.as_bytes(), VAULT_SEED], ctx.program_id);
        require!(
            expected_vault_pda == ctx.accounts.auction_vault.key(),
            AuctionError::InvalidVaultPDA
        );

        // Auction must be live
        require!(clock.slot <= auction_info.end_time, AuctionError::AuctionEnded);

        // Bidder must sign
        require!(bidder.is_signer, AuctionError::MissingBidderSignature);

        // Provided prev_high_acc must match stored highest bidder
        require!(
            prev_high_acc.key() == auction_info.highest_bidder,
            AuctionError::PrevHighestBidderMismatch
        );

        // New bid must be strictly greater
        require!(amount_to_deposit > auction_info.highest_bid, AuctionError::BidTooLow);

        // Transfer new bid amount from bidder -> auction_vault
        system_program::transfer(
            CpiContext::new(
                ctx.accounts.system_program.to_account_info(),
                system_program::Transfer {
                    from: bidder.to_account_info(),
                    to: ctx.accounts.auction_vault.to_account_info(),
                },
            ),
            amount_to_deposit,
        )?;

        // Refund previous highest bidder (if previous highest bidder was a real bidder, not the seller placeholder)
        if auction_info.highest_bidder != auction_info.seller && auction_info.highest_bid > 0 {
            let vault_seeds: &[&[u8]] = &[
                auctioned_object.as_bytes(),
                VAULT_SEED,
                &[vault_bump],
            ];
            let signer_seeds: &[&[&[u8]]] = &[&vault_seeds[..]];

            system_program::transfer(
                CpiContext::new_with_signer(
                    ctx.accounts.system_program.to_account_info(),
                    system_program::Transfer {
                        from: ctx.accounts.auction_vault.to_account_info(),
                        to: prev_high_acc.to_account_info(),
                    },
                    signer_seeds,
                ),
                auction_info.highest_bid,
            )?;
        }

        // Update auction state
        auction_info.highest_bid = amount_to_deposit;
        auction_info.highest_bidder = *bidder.key;

        Ok(())
    }

    /// End the auction. Only the seller stored in auction_info may call this after auction end_time.
    /// Transfers the vault balance to the seller and closes auction_info to seller (returning rent).
    pub fn end(ctx: Context<EndCtx>, auctioned_object: String) -> Result<()> {
        require!(
            auctioned_object.len() <= MAX_OBJECT_LEN,
            AuctionError::ObjectTooLong
        );

        let auction_info = &ctx.accounts.auction_info;
        let seller = &ctx.accounts.seller;
        let clock = Clock::get()?;

        // Verify PDAs
        let (expected_auction_pda, _a_bump) =
            Pubkey::find_program_address(&[auctioned_object.as_bytes()], ctx.program_id);
        require!(expected_auction_pda == auction_info.key(), AuctionError::InvalidPDA);

        let (expected_vault_pda, vault_bump) =
            Pubkey::find_program_address(&[auctioned_object.as_bytes(), VAULT_SEED], ctx.program_id);
        require!(
            expected_vault_pda == ctx.accounts.auction_vault.key(),
            AuctionError::InvalidVaultPDA
        );

        // Only seller may end
        require!(seller.is_signer, AuctionError::MissingSellerSignature);
        require!(seller.key() == auction_info.seller, AuctionError::NotAuctionSeller);

        // Auction must have ended
        require!(clock.slot > auction_info.end_time, AuctionError::AuctionNotEnded);

        // Drain vault lamports to seller (signed by vault PDA)
        let vault_lamports = ctx.accounts.auction_vault.to_account_info().lamports();
        if vault_lamports > 0 {
            // signer seeds for vault PDA
            let vault_seeds: &[&[u8]] = &[
                auctioned_object.as_bytes(),
                VAULT_SEED,
                &[vault_bump],
            ];
            let signer_seeds: &[&[&[u8]]] = &[&vault_seeds[..]];

            // Transfer all lamports from vault -> seller
            system_program::transfer(
                CpiContext::new_with_signer(
                    ctx.accounts.system_program.to_account_info(),
                    system_program::Transfer {
                        from: ctx.accounts.auction_vault.to_account_info(),
                        to: seller.to_account_info(),
                    },
                    signer_seeds,
                ),
                vault_lamports,
            )?;
        }

        // auction_info will be closed to seller by Anchor because of `close = seller` in EndCtx.
        Ok(())
    }
}

/// Start accounts
#[derive(Accounts)]
pub struct StartCtx<'info> {
    #[account(mut)]
    pub seller: Signer<'info>,

    #[account(
        init,
        payer = seller,
        space = 8 + 128,
        seeds = [auctioned_object.as_bytes()],
        bump
    )]
    pub auction_info: Account<'info, AuctionInfo>,

    #[account(
        init,
        payer = seller,
        seeds = [auctioned_object.as_bytes(), b"vault"],
        bump
    )]
    /// CHECK: Auction vault is a PDA to hold funds
    pub auction_vault: AccountInfo<'info>,
}


/// Bid accounts
#[derive(Accounts)]
pub struct BidCtx<'info> {
    #[account(mut)]
    pub bidder: Signer<'info>,

    #[account(mut)]
    pub auction_info: Account<'info, AuctionInfo>,

    #[account(mut)]
    /// CHECK: Auction vault PDA to hold lamports
    pub auction_vault: AccountInfo<'info>,
}

/// End accounts
#[derive(Accounts)]
pub struct EndCtx<'info> {
    #[account(mut)]
    pub seller: Signer<'info>,

    #[account(mut, has_one = seller, close = seller)]
    pub auction_info: Account<'info, AuctionInfo>,

    #[account(mut)]
    /// CHECK: Auction vault PDA to hold lamports
    pub auction_vault: AccountInfo<'info>,
}


/// AuctionInfo layout (exact fields required by your prompt)
#[account]
pub struct AuctionInfo {
    pub seller: Pubkey,
    pub highest_bidder: Pubkey,
    pub end_time: u64,
    pub highest_bid: u64,
    pub object: String,
}

impl AuctionInfo {
    // 8 discriminator + seller (32) + highest_bidder (32) + end_time (8) + highest_bid (8) + string (4 len prefix) + MAX_OBJECT_LEN
    pub const LEN: usize = 8 + 32 + 32 + 8 + 8 + 4 + MAX_OBJECT_LEN;
}

#[error_code]
pub enum AuctionError {
    #[msg("Object name too long")]
    ObjectTooLong,
    #[msg("Auction already ended")]
    AuctionEnded,
    #[msg("Auction has not ended yet")]
    AuctionNotEnded,
    #[msg("Bid must be strictly greater than current highest bid")]
    BidTooLow,
    #[msg("Invalid PDA for given auctioned_object")]
    InvalidPDA,
    #[msg("Invalid vault PDA for given auctioned_object")]
    InvalidVaultPDA,
    #[msg("Missing seller signature")]
    MissingSellerSignature,
    #[msg("Missing bidder signature")]
    MissingBidderSignature,
    #[msg("Provided current_highest_bidder account does not match stored highest bidder")]
    PrevHighestBidderMismatch,
    #[msg("Only the seller can end this auction")]
    NotAuctionSeller,
}
