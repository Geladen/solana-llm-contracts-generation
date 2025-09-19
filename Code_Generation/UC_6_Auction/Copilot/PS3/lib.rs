use anchor_lang::prelude::*;
use anchor_lang::solana_program::{
    program::invoke,
    system_instruction,
};


declare_id!("32ZGtf1q1qFhsmrtzxcfNfCCBR17uW6g94bMAYPah8Z3");

const MAX_OBJECT_LEN: usize = 64;
const AUCTION_INFO_LEN: usize = 8    // discriminator
    + 32   // seller
    + 32   // highest_bidder
    + 8    // end_time
    + 8    // highest_bid
    + 4    // string prefix for object
    + MAX_OBJECT_LEN;

#[program]
pub mod auction {
    use super::*;

    // 1) Start auction
    pub fn start(
        ctx: Context<StartCtx>,
        auctioned_object: String,
        duration_slots: u64,
        starting_bid: u64,
    ) -> Result<()> {
        let auction = &mut ctx.accounts.auction_info;
        let now = Clock::get()?.slot;

        auction.seller         = *ctx.accounts.seller.key;
        auction.highest_bid    = starting_bid;
        auction.highest_bidder = Pubkey::default();    // NO bidder yet
        auction.end_time       = now
            .checked_add(duration_slots)
            .ok_or(AuctionError::NumericalOverflow)?;
        auction.object         = auctioned_object;
        Ok(())
    }

    // 2) Place a bid
    pub fn bid(
        ctx: Context<BidCtx>,
        auctioned_object: String,
        amount_to_deposit: u64,
    ) -> Result<()> {
        // Clone AccountInfos *before* any &mut borrow
        let bidder_ai     = ctx.accounts.bidder.to_account_info().clone();
        let auction_ai    = ctx.accounts.auction_info.to_account_info().clone();
        let prev_bid_ai   = ctx.accounts.current_highest_bidder.to_account_info().clone();
        let system_ai     = ctx.accounts.system_program.to_account_info().clone();

        // 1: Validate timing + floor + object seed
        {
            let auction = &mut ctx.accounts.auction_info;
            require!(
                auction.object == auctioned_object,
                AuctionError::InvalidObjectSeed
            );
            let now = Clock::get()?.slot;
            require!(
                now <= auction.end_time,
                AuctionError::AuctionEnded
            );
            require!(
                amount_to_deposit > auction.highest_bid,
                AuctionError::BidTooLow
            );
        }

        // 2: Transfer new bid → PDA
        invoke(
            &system_instruction::transfer(
                bidder_ai.key,
                auction_ai.key,
                amount_to_deposit,
            ),
            &[ bidder_ai.clone(), auction_ai.clone(), system_ai.clone() ],
        )?;

        // 3: Refund *only* if there was a real previous bidder
        let previous_bid;
        let previous_key;
        {
            let auction = &mut ctx.accounts.auction_info;
            previous_bid = auction.highest_bid;
            previous_key = auction.highest_bidder;
        }

        if previous_key != Pubkey::default() {
            require!(
                &previous_key == prev_bid_ai.key,
                AuctionError::InvalidPreviousBidder
            );
            // direct lamports math
            **auction_ai.try_borrow_mut_lamports()? -= previous_bid;
            **prev_bid_ai.try_borrow_mut_lamports()? -= 0; // no‐op on data
            **prev_bid_ai.try_borrow_mut_lamports()? += previous_bid;
        }

        // 4: Finally update state
        {
            let auction = &mut ctx.accounts.auction_info;
            auction.highest_bid    = amount_to_deposit;
            auction.highest_bidder = *ctx.accounts.bidder.key;
        }

        Ok(())
    }

    // 3) End auction
    pub fn end(ctx: Context<EndCtx>, auctioned_object: String) -> Result<()> {
        let auction = &ctx.accounts.auction_info;

        // seed check
        require!(
            auction.object == auctioned_object,
            AuctionError::InvalidObjectSeed
        );
        // only seller
        require!(
            *ctx.accounts.seller.key == auction.seller,
            AuctionError::Unauthorized
        );
        // only after end
        let now = Clock::get()?.slot;
        require!(
            now > auction.end_time,
            AuctionError::AuctionNotEnded
        );

        // `close = seller` drains everything (highest‐bid + rent) back to seller
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
        space = AUCTION_INFO_LEN,
        seeds = [auctioned_object.as_bytes()],
        bump
    )]
    pub auction_info: Account<'info, AuctionInfo>,

    pub system_program: Program<'info, System>,
}

#[derive(Accounts)]
#[instruction(auctioned_object: String)]
pub struct BidCtx<'info> {
    #[account(mut)]
    pub bidder: Signer<'info>,

    #[account(
        mut,
        seeds = [auctioned_object.as_bytes()],
        bump
    )]
    pub auction_info: Account<'info, AuctionInfo>,

    /// CHECK: must match auction_info.highest_bidder before refund
    #[account(mut)]
    pub current_highest_bidder: UncheckedAccount<'info>,

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
        close = seller
    )]
    pub auction_info: Account<'info, AuctionInfo>,
}

#[account]
pub struct AuctionInfo {
    pub seller: Pubkey,
    pub highest_bidder: Pubkey,
    pub end_time: u64,
    pub highest_bid: u64,
    pub object: String,
}

#[error_code]
pub enum AuctionError {
    #[msg("Bid must exceed current highest bid")]
    BidTooLow,

    #[msg("Auction has not ended yet")]
    AuctionNotEnded,

    #[msg("Auction has already ended")]
    AuctionEnded,

    #[msg("Numerical overflow")]
    NumericalOverflow,

    #[msg("Object seed does not match auction state")]
    InvalidObjectSeed,

    #[msg("Provided bidder does not match stored highest bidder")]
    InvalidPreviousBidder,

    #[msg("Signer is not the auction seller")]
    Unauthorized,
}
