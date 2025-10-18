use anchor_lang::prelude::*;
use anchor_lang::system_program;
use anchor_lang::system_program::Transfer;

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
        auction.highest_bidder = *ctx.accounts.seller.key; // sentinel until a real deposit arrives
        auction.end_time = clock
            .slot
            .checked_add(duration_slots)
            .ok_or(ErrorCode::Overflow)?;
        auction.highest_bid = starting_bid; // reflect starting bid in state (expected by tests)
        auction.min_bid = starting_bid;     // keep min_bid for validation
        auction.object = auctioned_object;

        Ok(())
    }

    pub fn bid(
        ctx: Context<BidCtx>,
        auctioned_object: String,
        amount_to_deposit: u64,
    ) -> Result<()> {
        // 1) take AccountInfos before mutably borrowing auction_info
        let auction_info_ai = ctx.accounts.auction_info.to_account_info();
        let system_program_ai = ctx.accounts.system_program.to_account_info();
        let bidder_ai = ctx.accounts.bidder.to_account_info();
        let prev_bidder_ai = ctx.accounts.current_highest_bidder.to_account_info();

        // 2) mutably borrow the auction state
        let auction = &mut ctx.accounts.auction_info;

        // 3) validate auction active
        let clock = Clock::get()?;
        require!(clock.slot <= auction.end_time, ErrorCode::AuctionEnded);

        // 4) validate deposit respects min_bid and improves highest_bid
        require!(
            amount_to_deposit >= auction.min_bid,
            ErrorCode::BidBelowMinimum
        );
        require!(
            amount_to_deposit > auction.highest_bid,
            ErrorCode::BidTooLow
        );

        // 5) transfer lamports from bidder into PDA
        system_program::transfer(
            CpiContext::new(
                system_program_ai.clone(),
                Transfer {
                    from: bidder_ai.clone(),
                    to: auction_info_ai.clone(),
                },
            ),
            amount_to_deposit,
        )?;

        // 6) refund previous highest bidder if they were a real bidder
        let prev_amount = auction.highest_bid;
        let prev_bidder_pk = auction.highest_bidder;
        if prev_amount > 0 && prev_bidder_pk != auction.seller {
            // adjust lamports directly: auction_info is mutable and owned by program,
            // current_highest_bidder is a system account
            **auction_info_ai.try_borrow_mut_lamports()? -= prev_amount;
            **prev_bidder_ai.try_borrow_mut_lamports()? += prev_amount;
        }

        // 7) update state
        auction.highest_bid = amount_to_deposit;
        auction.highest_bidder = *ctx.accounts.bidder.key;

        Ok(())
    }

    pub fn end(
        ctx: Context<EndCtx>,
        _auctioned_object: String,
    ) -> Result<()> {
        let auction = &ctx.accounts.auction_info;

        // ensure auction finished
        let clock = Clock::get()?;
        require!(
            clock.slot >= auction.end_time,
            ErrorCode::AuctionOngoing
        );

        // No manual lamport math here. `close = seller` on the account
        // will transfer the PDA's entire lamport balance (winning bid + rent)
        // to the seller and deallocate the account.
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
        space = 8  // discriminator
            + 32   // seller
            + 32   // highest_bidder
            + 8    // end_time
            + 8    // highest_bid
            + 8    // min_bid
            + 4    // string prefix
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
        bump,
    )]
    pub auction_info: Account<'info, AuctionInfo>,

    /// CHECK: must equal auction_info.highest_bidder when used; mutable so we can credit lamports
    #[account(mut, address = auction_info.highest_bidder)]
    pub current_highest_bidder: SystemAccount<'info>,

    pub system_program: Program<'info, System>,
}

#[derive(Accounts)]
#[instruction(auctioned_object: String)]
pub struct EndCtx<'info> {
    #[account(mut)]
    pub seller: Signer<'info>,

    #[account(
        mut,
        seeds = [auctioned_object.as_ref()],
        bump,
        has_one = seller,
        close = seller,
    )]
    pub auction_info: Account<'info, AuctionInfo>,
}

#[account]
pub struct AuctionInfo {
    pub seller: Pubkey,
    pub highest_bidder: Pubkey,
    pub end_time: u64,
    pub highest_bid: u64,
    // additional stored field: minimum (starting) bid that is NOT deposited
    pub min_bid: u64,
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
    #[msg("Bid is below minimum starting bid")]
    BidBelowMinimum,
}
