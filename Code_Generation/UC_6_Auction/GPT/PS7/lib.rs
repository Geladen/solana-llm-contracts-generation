use anchor_lang::prelude::*;
use anchor_lang::solana_program::{
    program::{invoke, invoke_signed},
    system_instruction,
};

declare_id!("FhWLPxhuXJvM3HAqUiBgrSe6XfuxTqdKJxjCqu7WQs9G");

#[program]
pub mod auction{
    use super::*;

    pub fn start(
        ctx: Context<StartCtx>,
        auctioned_object: String,
        duration_slots: u64,
        starting_bid: u64,
    ) -> Result<()> {
        let clock = Clock::get()?;

        if auctioned_object.len() > AuctionInfo::MAX_OBJECT_LEN {
            return err!(ErrorCode::ObjectNameTooLong);
        }

        let end_slot = clock
            .slot
            .checked_add(duration_slots)
            .ok_or(ErrorCode::Overflow)?;

        let auction_info = &mut ctx.accounts.auction_info;
        auction_info.seller = *ctx.accounts.seller.key;
        auction_info.highest_bidder = *ctx.accounts.seller.key;
        auction_info.end_time = end_slot;
        auction_info.highest_bid = starting_bid;
        auction_info.object = auctioned_object.clone();

        // Transfer starting bid from seller -> PDA if >0
        if starting_bid > 0 {
            let ix = system_instruction::transfer(
                ctx.accounts.seller.key,
                ctx.accounts.auction_info.to_account_info().key,
                starting_bid,
            );
            invoke(
                &ix,
                &[
                    ctx.accounts.seller.to_account_info(),
                    ctx.accounts.auction_info.to_account_info(),
                    ctx.accounts.system_program.to_account_info(),
                ],
            )?;
        }

        Ok(())
    }

    pub fn bid(
        ctx: Context<BidCtx>,
        auctioned_object: String,
        amount_to_deposit: u64,
        bump: u8, // <- pass bump from client
    ) -> Result<()> {
        let clock = Clock::get()?;

        // Validate auction object
        if ctx.accounts.auction_info.object != auctioned_object {
            return err!(ErrorCode::InvalidAuctionObject);
        }

        if clock.slot >= ctx.accounts.auction_info.end_time {
            return err!(ErrorCode::AuctionEnded);
        }

        if amount_to_deposit <= ctx.accounts.auction_info.highest_bid {
            return err!(ErrorCode::BidTooLow);
        }

        let bidder_lamports = **ctx.accounts.bidder.to_account_info().lamports.borrow();
        if bidder_lamports < amount_to_deposit {
            return err!(ErrorCode::InsufficientFunds);
        }

        if ctx.accounts.current_highest_bidder.key()
            != ctx.accounts.auction_info.highest_bidder
        {
            return err!(ErrorCode::InvalidCurrentHighestBidder);
        }

        // Prepare AccountInfo references before mutable borrow
        let bidder_ai = ctx.accounts.bidder.to_account_info();
        let auction_ai = ctx.accounts.auction_info.to_account_info();
        let prev_high_bidder_ai = ctx.accounts.current_highest_bidder.to_account_info();
        let system_program_ai = ctx.accounts.system_program.to_account_info();

        // Transfer new bid from bidder -> PDA
        let ix_deposit =
            system_instruction::transfer(bidder_ai.key, auction_ai.key, amount_to_deposit);
        invoke(
            &ix_deposit,
            &[bidder_ai.clone(), auction_ai.clone(), system_program_ai.clone()],
        )?;

        // Refund previous highest bidder if any
        let previous_bid = ctx.accounts.auction_info.highest_bid;
        if previous_bid > 0 {
            let ix_refund =
                system_instruction::transfer(auction_ai.key, prev_high_bidder_ai.key, previous_bid);
            let refund_accounts = &[auction_ai.clone(), prev_high_bidder_ai.clone(), system_program_ai.clone()];

            // PDA signer seeds
            let seeds: &[&[u8]] = &[auctioned_object.as_bytes(), &[bump]];
            invoke_signed(&ix_refund, refund_accounts, &[seeds])?;

        }

        // Update auction info
        let auction_info = &mut ctx.accounts.auction_info;
        auction_info.highest_bid = amount_to_deposit;
        auction_info.highest_bidder = *ctx.accounts.bidder.key;

        Ok(())
    }

    pub fn end(ctx: Context<EndCtx>, auctioned_object: String) -> Result<()> {
        let clock = Clock::get()?;
        let auction_info = &ctx.accounts.auction_info;

        if auction_info.object != auctioned_object {
            return err!(ErrorCode::InvalidAuctionObject);
        }

        if clock.slot < auction_info.end_time {
            return err!(ErrorCode::AuctionNotEnded);
        }

        // `close = seller` on auction_info automatically transfers lamports and closes PDA
        Ok(())
    }
}

/// Accounts

#[derive(Accounts)]
#[instruction(auctioned_object: String)]
pub struct StartCtx<'info> {
    #[account(mut)]
    pub seller: Signer<'info>,

    #[account(
        init,
        payer = seller,
        space = AuctionInfo::LEN,
        seeds = [auctioned_object.as_bytes()],
        bump
    )]
    pub auction_info: Account<'info, AuctionInfo>,

    pub system_program: Program<'info, System>,
}

#[derive(Accounts)]
#[instruction(auctioned_object: String)]
pub struct BidCtx<'info> {
    /// Bidder paying the deposit (must sign)
    #[account(mut)]
    pub bidder: Signer<'info>,  // <-- MUST be Signer

    /// The Auction PDA being bid on
    #[account(mut, seeds = [auctioned_object.as_bytes()], bump)]
    pub auction_info: Account<'info, AuctionInfo>,

    /// Current highest bidder to refund
    /// CHECK: validated at runtime against auction_info.highest_bidder
    #[account(mut)]
    pub current_highest_bidder: AccountInfo<'info>,

    /// System program for CPI transfers
    pub system_program: Program<'info, System>,
}


#[derive(Accounts)]
#[instruction(auctioned_object: String)]
pub struct EndCtx<'info> {
    #[account(mut)]
    pub seller: Signer<'info>,

    #[account(
        mut,
        seeds = [auctioned_object.as_bytes()],
        bump,
        has_one = seller,
        close = seller
    )]
    pub auction_info: Account<'info, AuctionInfo>,
}

/// Auction state

#[account]
pub struct AuctionInfo {
    pub seller: Pubkey,
    pub highest_bidder: Pubkey,
    pub end_time: u64,
    pub highest_bid: u64,
    pub object: String,
}

impl AuctionInfo {
    pub const MAX_OBJECT_LEN: usize = 256;
    pub const LEN: usize = 8 + 32 + 32 + 8 + 8 + 4 + Self::MAX_OBJECT_LEN;
}

/// Errors

#[error_code]
pub enum ErrorCode {
    #[msg("Auction object name is too long.")]
    ObjectNameTooLong,
    #[msg("Insufficient funds to complete requested transfer.")]
    InsufficientFunds,
    #[msg("Integer overflow/underflow.")]
    Overflow,
    #[msg("Bid must be strictly greater than current highest bid.")]
    BidTooLow,
    #[msg("Auction has already ended.")]
    AuctionEnded,
    #[msg("Auction has not ended yet.")]
    AuctionNotEnded,
    #[msg("Provided current_highest_bidder account does not match the on-chain highest bidder.")]
    InvalidCurrentHighestBidder,
    #[msg("Provided auctioned_object does not match PDA data.")]
    InvalidAuctionObject,
    #[msg("Failed to retrieve PDA bump from context.")]
    InvalidBump,
}
