use anchor_lang::prelude::*;
use anchor_lang::system_program;

declare_id!("CvNn9tY3o6KZcvD4Mq92u4YrHtkaMR4WvdzfWNmkCCAp");

#[program]
pub mod auction {
    use super::*;

    pub fn start(
        ctx: Context<StartCtx>,
        auctioned_object: String,
        duration_slots: u64,
        starting_bid: u64,
    ) -> Result<()> {
        let auction_info = &mut ctx.accounts.auction_info;
        let seller = &ctx.accounts.seller;
        let clock = Clock::get()?;

        // Validate inputs
        require!(!auctioned_object.is_empty(), AuctionError::EmptyObject);
        require!(duration_slots > 0, AuctionError::InvalidDuration);

        // Initialize auction state
        auction_info.seller = seller.key();
        auction_info.highest_bidder = seller.key(); // Seller starts as highest bidder
        auction_info.end_time = clock.slot + duration_slots;
        auction_info.highest_bid = starting_bid;
        auction_info.object = auctioned_object;

        msg!("Auction started for object: {}", auction_info.object);
        msg!("Starting bid: {} lamports", starting_bid);
        msg!("Duration: {} slots", duration_slots);

        Ok(())
    }

    pub fn bid(
        ctx: Context<BidCtx>,
        auctioned_object: String,
        amount_to_deposit: u64,
    ) -> Result<()> {
        let auction_info = &mut ctx.accounts.auction_info;
        let bidder = &ctx.accounts.bidder;
        let current_highest_bidder = &mut ctx.accounts.current_highest_bidder;
        let system_program = &ctx.accounts.system_program;
        let clock = Clock::get()?;

        // Validate auction is still active
        require!(clock.slot < auction_info.end_time, AuctionError::AuctionEnded);

        // Validate bid amount
        require!(
            amount_to_deposit > auction_info.highest_bid,
            AuctionError::BidTooLow
        );

        // Prevent seller from bidding on their own auction
        require!(
            bidder.key() != auction_info.seller,
            AuctionError::SellerCannotBid
        );

        // Prevent bidding against yourself
        require!(
            bidder.key() != auction_info.highest_bidder,
            AuctionError::AlreadyHighestBidder
        );

        // Transfer new bid amount from bidder to auction PDA
        let transfer_ix = system_program::Transfer {
            from: bidder.to_account_info(),
            to: auction_info.to_account_info(),
        };
        system_program::transfer(
            CpiContext::new(system_program.to_account_info(), transfer_ix),
            amount_to_deposit,
        )?;

        // Refund the previous highest bidder if not the seller
        if auction_info.highest_bidder != auction_info.seller {
            **auction_info.to_account_info().try_borrow_mut_lamports()? -= auction_info.highest_bid;
            **current_highest_bidder.try_borrow_mut_lamports()? += auction_info.highest_bid;
        }

        // Update auction state
        auction_info.highest_bidder = bidder.key();
        auction_info.highest_bid = amount_to_deposit;

        msg!("New highest bid: {} lamports from {}", amount_to_deposit, bidder.key());

        Ok(())
    }

    pub fn end(ctx: Context<EndCtx>, auctioned_object: String) -> Result<()> {
        let auction_info = &mut ctx.accounts.auction_info;
        let seller = &mut ctx.accounts.seller;
        let clock = Clock::get()?;

        // Validate auction has ended
        require!(clock.slot >= auction_info.end_time, AuctionError::AuctionNotEnded);

        // Transfer highest bid to seller (if there was a winning bid)
        if auction_info.highest_bidder != auction_info.seller {
            **auction_info.to_account_info().try_borrow_mut_lamports()? -= auction_info.highest_bid;
            **seller.try_borrow_mut_lamports()? += auction_info.highest_bid;
        }

        msg!("Auction ended for object: {}", auction_info.object);
        msg!("Winning bid: {} lamports", auction_info.highest_bid);
        msg!("Winner: {}", auction_info.highest_bidder);

        // Note: Account closure happens automatically due to close constraint

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
        space = 8 + AuctionInfo::INIT_SPACE,
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
        bump
    )]
    pub auction_info: Account<'info, AuctionInfo>,
    
    /// CHECK: This is the current highest bidder who will receive refund
    #[account(
        mut,
        constraint = current_highest_bidder.key() == auction_info.highest_bidder
            @ AuctionError::InvalidHighestBidder
    )]
    pub current_highest_bidder: UncheckedAccount<'info>,
    
    pub system_program: Program<'info, System>,
}

#[derive(Accounts)]
#[instruction(auctioned_object: String)]
pub struct EndCtx<'info> {
    #[account(
        mut,
        constraint = seller.key() == auction_info.seller @ AuctionError::UnauthorizedSeller
    )]
    pub seller: Signer<'info>,
    
    #[account(
        mut,
        seeds = [auctioned_object.as_ref()],
        bump,
        close = seller
    )]
    pub auction_info: Account<'info, AuctionInfo>,
}

#[account]
#[derive(InitSpace)]
pub struct AuctionInfo {
    pub seller: Pubkey,
    pub highest_bidder: Pubkey,
    pub end_time: u64,
    pub highest_bid: u64,
    #[max_len(200)]
    pub object: String,
}

#[error_code]
pub enum AuctionError {
    #[msg("Auctioned object cannot be empty")]
    EmptyObject,
    #[msg("Duration must be greater than 0")]
    InvalidDuration,
    #[msg("Auction has already ended")]
    AuctionEnded,
    #[msg("Bid amount must be higher than current highest bid")]
    BidTooLow,
    #[msg("Seller cannot bid on their own auction")]
    SellerCannotBid,
    #[msg("You are already the highest bidder")]
    AlreadyHighestBidder,
    #[msg("Auction has not ended yet")]
    AuctionNotEnded,
    #[msg("Only the seller can end the auction")]
    UnauthorizedSeller,
    #[msg("Invalid highest bidder account provided")]
    InvalidHighestBidder,
}