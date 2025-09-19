use anchor_lang::prelude::*;
use anchor_lang::system_program;
use anchor_lang::system_program::Transfer;
use anchor_lang::solana_program::{
    program::invoke_signed,
    system_instruction,
};

declare_id!("48hUQmvrqfSE9GmCXoV5mn84zhe9NALaiDEbDjTkLwdk");

const MAX_OBJECT_LENGTH: usize = 64;

#[program]
pub mod auction {
    use super::*;

    pub fn start(
        ctx: Context<StartCtx>,
        auctioned_object: String,
        duration_slots: u64,
        starting_bid: u64,
    ) -> Result<()> {
        let auction = &mut ctx.accounts.auction_info;
        let clock = Clock::get()?;

        require!(duration_slots > 0, ErrorCode::InvalidDuration);
        auction.seller = *ctx.accounts.seller.key;
        auction.highest_bidder = *ctx.accounts.seller.key;
        auction.end_time = clock
            .slot
            .checked_add(duration_slots)
            .ok_or(ErrorCode::Overflow)?;
        auction.highest_bid = starting_bid;
        auction.object = auctioned_object.clone();

        Ok(())
    }

pub fn bid(
    ctx: Context<BidCtx>,
    auctioned_object: String,
    amount_to_deposit: u64,
) -> Result<()> {
    // 1) Extract AccountInfos before any &mut borrow
    let auction_info_ai = ctx.accounts.auction_info.to_account_info();
    let system_program_ai = ctx.accounts.system_program.to_account_info();
    let bidder_ai = ctx.accounts.bidder.to_account_info();
    let prev_bidder_ai = ctx.accounts.current_highest_bidder.to_account_info();

    // 2) Mutably borrow the auction state
    let auction = &mut ctx.accounts.auction_info;

    // 3) Ensure auction is still live
    let clock = Clock::get()?;
    require!(clock.slot <= auction.end_time, ErrorCode::AuctionEnded);

    // 4) New bid must exceed the old one
    require!(amount_to_deposit > auction.highest_bid, ErrorCode::BidTooLow);

    // 5) Transfer the new bid from bidder into the auction PDA
    system_program::transfer(
        CpiContext::new(
            system_program_ai.clone(),
            Transfer {
                from: bidder_ai.clone(),
                to:   auction_info_ai.clone(),
            },
        ),
        amount_to_deposit,
    )?;

    // 6) Refund the previous highest bidder by adjusting lamports directly
    let prev_amount = auction.highest_bid;
    if prev_amount > 0 {
        **auction_info_ai
            .try_borrow_mut_lamports()? -= prev_amount;
        **prev_bidder_ai
            .try_borrow_mut_lamports()? += prev_amount;
    }

    // 7) Update on‐chain state
    auction.highest_bid    = amount_to_deposit;
    auction.highest_bidder = *ctx.accounts.bidder.key;

    Ok(())
}

pub fn end(
  ctx: Context<EndCtx>,
  _auctioned_object: String,
) -> Result<()> {
  let auction = &ctx.accounts.auction_info;
  let clock   = Clock::get()?;

  // 1) Auction must have elapsed
  require!(
    clock.slot >= auction.end_time,
    ErrorCode::AuctionOngoing
  );

  // 2) No manual lamport moves here!
  //    Returning Ok lets Anchor:
  //      - transfer ALL lamports in the PDA (winning bid + rent reserve)
  //        to `seller`
  //      - deallocate the account
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
        space = 8  // Discriminator
             + 32 // seller
             + 32 // highest_bidder
             + 8  // end_time
             + 8  // highest_bid
             + 4  // string prefix
             + MAX_OBJECT_LENGTH,
        seeds = [auctioned_object.as_ref()],
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
        seeds = [auctioned_object.as_ref()],
        bump,             // ← this makes ctx.bumps.auction_info available
    )]
    pub auction_info: Account<'info, AuctionInfo>,

    /// CHECK: must match auction_info.highest_bidder
    #[account(mut, address = auction_info.highest_bidder)]
    pub current_highest_bidder: SystemAccount<'info>,

    pub system_program: Program<'info, System>,
}


#[derive(Accounts)]
#[instruction(auctioned_object: String)]
pub struct EndCtx<'info> {
  /// The seller who started the auction
  #[account(mut)]
  pub seller: Signer<'info>,

  /// PDA holding both the state and all bid lamports
  #[account(
    mut,
    seeds = [auctioned_object.as_ref()],
    bump,
    has_one = seller,
    close = seller    // <-- THIS sends *all* lamports (bids + rent) to `seller`
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
pub enum ErrorCode {
    #[msg("Duration must be greater than zero")]
    InvalidDuration,
    #[msg("Overflow calculating end time")]
    Overflow,
    #[msg("Auction has already ended")]
    AuctionEnded,
    #[msg("Bid must exceed the current highest bid")]
    BidTooLow,
    #[msg("Auction is still ongoing")]
    AuctionOngoing,
}

