use anchor_lang::prelude::*;
use anchor_lang::solana_program::{program::invoke, system_instruction};

declare_id!("AX5CtfUGeU69G7uto6x1HSiPS4h7rUjRAGoUq4xJr5qB");

/// Maximum bytes allowed for the auctioned object string.
/// Adjust as needed, but keep consistent with space calculation below.
const MAX_OBJECT_LEN: usize = 128;

/// Space calculation for the AuctionInfo PDA:
/// 8 discriminator + 32 seller + 32 highest_bidder + 8 end_time + 8 highest_bid
/// + 4 (string len prefix) + MAX_OBJECT_LEN
const AUCTION_INFO_SPACE: usize = 8 + 32 + 32 + 8 + 8 + 4 + MAX_OBJECT_LEN;

#[program]
pub mod auction {
    use super::*;

    /// Start an auction. Creates the auction PDA and deposits the `starting_bid`
    /// from the seller into the auction PDA so the seller is the initial (refundable) highest bidder.
    pub fn start(
        ctx: Context<StartCtx>,
        auctioned_object: String,
        duration_slots: u64,
        starting_bid: u64,
    ) -> Result<()> {
        // Validate object length
        require!(
            auctioned_object.len() <= MAX_OBJECT_LEN,
            AuctionError::ObjectTooLong
        );

        // Initialize auction state
        let clock = Clock::get()?;
        let end_time = clock.slot.checked_add(duration_slots).ok_or(AuctionError::Overflow)?;

        let auction = &mut ctx.accounts.auction_info;
        auction.seller = ctx.accounts.seller.key();
        auction.highest_bidder = ctx.accounts.seller.key(); // seller is initial highest bidder
        auction.end_time = end_time;
        auction.highest_bid = starting_bid;
        auction.object = auctioned_object.clone();

        // If starting_bid > 0, transfer lamports from seller to PDA
        if starting_bid > 0 {
            // system transfer from seller -> auction_pda
            let ix = system_instruction::transfer(
                &ctx.accounts.seller.key(),
                &ctx.accounts.auction_info.key(),
                starting_bid,
            );
            // Pass the seller and auction_info and system_program AccountInfos
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

    /// Place a bid. Caller must be the bidder signer and must pass the current highest bidder account
    /// (the account which will be refunded). The bid amount must be greater than current highest_bid.
pub fn bid(
    ctx: Context<BidCtx>,
    auctioned_object: String,
    amount_to_deposit: u64,
) -> Result<()> {
    require!(auctioned_object.len() <= MAX_OBJECT_LEN, AuctionError::ObjectTooLong);

    // Immutable borrow only
    {
        let auction = &ctx.accounts.auction_info;

        require!(auction.object == auctioned_object, AuctionError::ObjectMismatch);
        let clock = Clock::get()?;
        require!(clock.slot < auction.end_time, AuctionError::AuctionEnded);
        require!(amount_to_deposit > auction.highest_bid, AuctionError::BidTooLow);
        require_keys_eq!(
            auction.highest_bidder,
            ctx.accounts.current_highest_bidder.key(),
            AuctionError::InvalidPreviousHighestBidder
        );
    }

    // First transfer new bid into PDA
    let transfer_ix = system_instruction::transfer(
        &ctx.accounts.bidder.key(),
        &ctx.accounts.auction_info.key(),
        amount_to_deposit,
    );
    invoke(
        &transfer_ix,
        &[
            ctx.accounts.bidder.to_account_info(),
            ctx.accounts.auction_info.to_account_info(),
            ctx.accounts.system_program.to_account_info(),
        ],
    )?;

    // Refund old highest bidder (need previous amount and key)
    let prev_amount = ctx.accounts.auction_info.highest_bid;
    if prev_amount > 0 {
        **ctx.accounts.auction_info.to_account_info().try_borrow_mut_lamports()? -= prev_amount;
        **ctx.accounts.current_highest_bidder.to_account_info().try_borrow_mut_lamports()? += prev_amount;
    }

    // Now re-borrow mutably to update state
    let auction = &mut ctx.accounts.auction_info;
    auction.highest_bidder = ctx.accounts.bidder.key();
    auction.highest_bid = amount_to_deposit;

    Ok(())
}


    /// End the auction. Only the seller may call this. Must only be called after auction end_time.
    /// Transfers the highest bid from PDA to seller, then closes the PDA (rent returned to seller).
    pub fn end(ctx: Context<EndCtx>, auctioned_object: String) -> Result<()> {
        // Validate name len
        require!(
            auctioned_object.len() <= MAX_OBJECT_LEN,
            AuctionError::ObjectTooLong
        );

        let auction = &mut ctx.accounts.auction_info;

        // Check object matches PDA's stored object
        require!(
            auction.object == auctioned_object,
            AuctionError::ObjectMismatch
        );

        // Only seller can end
        require_keys_eq!(auction.seller, ctx.accounts.seller.key(), AuctionError::Unauthorized);

        // Check auction is ended by slot
        let clock = Clock::get()?;
        require!(
            clock.slot >= auction.end_time,
            AuctionError::AuctionNotEndedYet
        );

        // Transfer highest bid to seller (direct lamport move since auction_info is program-owned)
        let payout = auction.highest_bid;
        if payout > 0 {
            **ctx
                .accounts
                .auction_info
                .to_account_info()
                .try_borrow_mut_lamports()? -= payout;
            **ctx.accounts.seller.to_account_info().try_borrow_mut_lamports()? += payout;
        }

        // At function exit, because of `close = seller` in the account constraints,
        // the auction_info account will be closed and any remaining SOL (rent-exempt) will be
        // returned to seller automatically.

        Ok(())
    }
}

/// Auction state stored in PDA
#[account]
pub struct AuctionInfo {
    pub seller: Pubkey,
    pub highest_bidder: Pubkey,
    pub end_time: u64,
    pub highest_bid: u64,
    pub object: String,
}

/// Start context: seller creates the PDA (seeds = [auctioned_object.as_ref()])
#[derive(Accounts)]
#[instruction(auctioned_object: String)]
pub struct StartCtx<'info> {
    #[account(mut)]
    pub seller: Signer<'info>,

    #[account(
        init,
        payer = seller,
        space = AUCTION_INFO_SPACE,
        seeds = [auctioned_object.as_ref()],
        bump
    )]
    pub auction_info: Account<'info, AuctionInfo>,

    pub system_program: Program<'info, System>,
}

/// Bid context: bidder places a bid. The caller must pass the current highest bidder's account
/// as `current_highest_bidder` (this account will receive the immediate refund).
#[derive(Accounts)]
#[instruction(auctioned_object: String)]
pub struct BidCtx<'info> {
    #[account(mut)]
    pub bidder: Signer<'info>,

    #[account(mut, seeds = [auctioned_object.as_ref()], bump)]
    pub auction_info: Account<'info, AuctionInfo>,

    #[account(mut)]
    /// CHECK: verified against auction_info.highest_bidder in handler
    pub current_highest_bidder: UncheckedAccount<'info>,

    pub system_program: Program<'info, System>,
}

/// End context: seller closes auction PDA. Seeds = [auctioned_object.as_ref()]
#[derive(Accounts)]
#[instruction(auctioned_object: String)]
pub struct EndCtx<'info> {
    #[account(mut)]
    pub seller: Signer<'info>,

    #[account(mut, seeds = [auctioned_object.as_ref()], bump, close = seller)]
    pub auction_info: Account<'info, AuctionInfo>,
}

#[error_code]
pub enum AuctionError {
    #[msg("The auctioned object string is too long.")]
    ObjectTooLong,
    #[msg("Auction object seed mismatch.")]
    ObjectMismatch,
    #[msg("The auction has already ended.")]
    AuctionEnded,
    #[msg("Bid amount is too low.")]
    BidTooLow,
    #[msg("Provided previous highest bidder account does not match stored highest bidder.")]
    InvalidPreviousHighestBidder,
    #[msg("Overflow in time calculation.")]
    Overflow,
    #[msg("Only the seller can perform this operation.")]
    Unauthorized,
    #[msg("Auction has not ended yet; cannot call end.")]
    AuctionNotEndedYet,
}
