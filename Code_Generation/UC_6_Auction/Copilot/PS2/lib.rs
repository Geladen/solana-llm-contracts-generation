use anchor_lang::prelude::*;
use anchor_lang::system_program;

declare_id!("7iub8X1kHopiHdaZm1DHSuMXtg1fvgxXFiaBcEEf9fGX");

#[program]
pub mod auction_copilot {
    use super::*;

    pub fn start(
        ctx: Context<StartCtx>,
        auctioned_object: String,
        duration_slots: u64,
        starting_bid: u64,
    ) -> Result<()> {
        // 1) compute end slot
        let clock = Clock::get()?;
        let end_slot = clock
            .slot
            .checked_add(duration_slots)
            .ok_or(AuctionError::Overflow)?;

        // 2) initialize on‐chain state
        {
            let info = &mut ctx.accounts.auction_info;
            info.seller = *ctx.accounts.seller.key;
            info.highest_bidder = *ctx.accounts.seller.key;
            info.end_time = end_slot;
            info.highest_bid = starting_bid;
            info.object = auctioned_object.clone();
        }

        // 3) escrow the starting bid into the auction PDA
        let cpi_accounts = system_program::Transfer {
            from: ctx.accounts.seller.to_account_info(),
            to: ctx.accounts.auction_info.to_account_info(),
        };
        let cpi_ctx = CpiContext::new(
            ctx.accounts.system_program.to_account_info(),
            cpi_accounts,
        );
        system_program::transfer(cpi_ctx, starting_bid)?;

        Ok(())
    }

    pub fn bid(
        ctx: Context<BidCtx>,
        _auctioned_object: String,
        amount_to_deposit: u64,
    ) -> Result<()> {
        // 1) read current state (immutable)
        let clock = Clock::get()?;
        let end_time = ctx.accounts.auction_info.end_time;
        require!(clock.slot < end_time, AuctionError::AuctionEnded);

        let current_highest = ctx.accounts.auction_info.highest_bid;
        require!(
            amount_to_deposit > current_highest,
            AuctionError::BidTooLow
        );

        let prev_bidder = ctx.accounts.auction_info.highest_bidder;
        let seller_key = ctx.accounts.auction_info.seller;

        // 2) deposit new bid via CPI
        {
            let cpi_accounts = system_program::Transfer {
                from: ctx.accounts.bidder.to_account_info(),
                to: ctx.accounts.auction_info.to_account_info(),
            };
            let cpi_ctx = CpiContext::new(
                ctx.accounts.system_program.to_account_info(),
                cpi_accounts,
            );
            system_program::transfer(cpi_ctx, amount_to_deposit)?;
        }

        // 3) refund previous bidder if they weren’t the seller
        if prev_bidder != seller_key {
            require_keys_eq!(
                ctx.accounts.current_highest_bidder.key(),
                prev_bidder,
                AuctionError::InvalidHighestBidderAccount
            );
            let refund_amount = current_highest;

            // debit the PDA’s lamports
            {
                let pda_ai = ctx.accounts.auction_info.to_account_info();
                let mut lamports = pda_ai.lamports.borrow_mut();
                **lamports = lamports
                    .checked_sub(refund_amount)
                    .ok_or(AuctionError::Overflow)?;
            }

            // credit back to previous bidder
            {
                let mut prior_lam =
                    ctx.accounts.current_highest_bidder.lamports.borrow_mut();
                **prior_lam = prior_lam.checked_add(refund_amount).unwrap();
            }
        }

        // 4) finally update the highest bid & bidder (mutable borrow only here)
        {
            let info = &mut ctx.accounts.auction_info;
            info.highest_bid = amount_to_deposit;
            info.highest_bidder = *ctx.accounts.bidder.key;
        }

        Ok(())
    }

    pub fn end(ctx: Context<EndCtx>, _auctioned_object: String) -> Result<()> {
        // 1) ensure auction time has passed
        let clock = Clock::get()?;
        let end_time = ctx.accounts.auction_info.end_time;
        require!(
            clock.slot >= end_time,
            AuctionError::AuctionNotEnded
        );

        // 2) payout the winning bid via direct lamport math
        let payout = ctx.accounts.auction_info.highest_bid;

        // debit the PDA
        {
            let pda_ai = ctx.accounts.auction_info.to_account_info();
            let mut lamports = pda_ai.lamports.borrow_mut();
            **lamports = lamports
                .checked_sub(payout)
                .ok_or(AuctionError::Overflow)?;
        }

        // credit seller
        {
            let seller_ai = ctx.accounts.seller.to_account_info();
            let mut sell_lam = seller_ai.lamports.borrow_mut();
            **sell_lam = sell_lam.checked_add(payout).unwrap();
        }

        // 3) Anchor will close the auction_info account (close = seller),
        //    returning its remaining rent‐exempt lamports to seller.
        Ok(())
    }
}

//
// ACCOUNT CONTEXTS
//

#[derive(Accounts)]
#[instruction(auctioned_object: String)]
pub struct StartCtx<'info> {
    /// Seller signs to create and fund the auction
    #[account(mut)]
    pub seller: Signer<'info>,

    /// PDA storing auction state and escrow
    #[account(
        init,
        payer = seller,
        seeds = [auctioned_object.as_bytes()],
        bump,
        space = 8 + AuctionInfo::LEN
    )]
    pub auction_info: Account<'info, AuctionInfo>,

    pub system_program: Program<'info, System>,
}

#[derive(Accounts)]
#[instruction(auctioned_object: String)]
pub struct BidCtx<'info> {
    /// Bidder signs and funds their bid
    #[account(mut)]
    pub bidder: Signer<'info>,

    /// PDA storing auction state and lamports
    #[account(
        mut,
        seeds = [auctioned_object.as_bytes()],
        bump
    )]
    pub auction_info: Account<'info, AuctionInfo>,

    /// CHECK: must equal the prior highest bidder when refunding
    #[account(mut)]
    pub current_highest_bidder: AccountInfo<'info>,

    pub system_program: Program<'info, System>,
}

#[derive(Accounts)]
#[instruction(auctioned_object: String)]
pub struct EndCtx<'info> {
    /// Seller signs to end the auction and collect proceeds
    #[account(mut)]
    pub seller: Signer<'info>,

    /// PDA storing auction state; closed here with `close = seller`
    #[account(
        mut,
        seeds = [auctioned_object.as_bytes()],
        bump,
        has_one = seller,
        close = seller
    )]
    pub auction_info: Account<'info, AuctionInfo>,

    pub system_program: Program<'info, System>,
}

//
// STATE & ERRORS
//

#[account]
pub struct AuctionInfo {
    pub seller: Pubkey,
    pub highest_bidder: Pubkey,
    pub end_time: u64,
    pub highest_bid: u64,
    pub object: String,
}

impl AuctionInfo {
    pub const MAX_OBJECT_LEN: usize = 64;
    pub const LEN: usize =
        32 + 32 + 8 + 8 + 4 + Self::MAX_OBJECT_LEN;
}

#[error_code]
pub enum AuctionError {
    #[msg("Auction has already ended.")]
    AuctionEnded,
    #[msg("Auction has not ended yet.")]
    AuctionNotEnded,
    #[msg("Bid is too low.")]
    BidTooLow,
    #[msg("Arithmetic overflow.")]
    Overflow,
    #[msg("Invalid highest bidder account.")]
    InvalidHighestBidderAccount,
}
