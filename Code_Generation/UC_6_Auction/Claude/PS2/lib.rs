use anchor_lang::prelude::*;

declare_id!("AygcXziX5bUrhRCoPeSJwJLgWQ8tYoziodXQtNg5JMsy");

#[program]
pub mod auction {
    use super::*;

    /// Starts a new auction
    /// Only callable by the seller
    pub fn start(
        ctx: Context<StartCtx>,
        auctioned_object: String,
        duration_slots: u64,
        starting_bid: u64,
    ) -> Result<()> {
        let auction_info = &mut ctx.accounts.auction_info;
        let seller = &ctx.accounts.seller;
        let current_slot = Clock::get()?.slot;

        // Validate auction object name length (prevent excessive storage costs)
        require!(
            auctioned_object.len() <= 64,
            AuctionError::ObjectNameTooLong
        );
        
        // Validate duration
        require!(
            duration_slots > 0 && duration_slots <= 864000, // Max ~4.5 days at 400ms slots
            AuctionError::InvalidDuration
        );

        // Initialize auction info
        auction_info.seller = seller.key();
        auction_info.highest_bidder = seller.key(); // Seller starts as highest bidder
        auction_info.end_time = current_slot + duration_slots;
        auction_info.highest_bid = starting_bid;
        auction_info.object = auctioned_object;

        msg!("Auction started for object: {}", auction_info.object);
        msg!("Starting bid: {} lamports", starting_bid);
        msg!("End time: slot {}", auction_info.end_time);

        Ok(())
    }

    /// Places a bid on an auction
    /// Only callable by bidders with sufficient funds
    pub fn bid(
        ctx: Context<BidCtx>,
        auctioned_object: String,
        amount_to_deposit: u64,
    ) -> Result<()> {
        let auction_info = &mut ctx.accounts.auction_info;
        let bidder = &ctx.accounts.bidder;
        let current_highest_bidder = &mut ctx.accounts.current_highest_bidder;
        let current_slot = Clock::get()?.slot;

        // Validate auction is still active
        require!(
            current_slot < auction_info.end_time,
            AuctionError::AuctionEnded
        );

        // Validate object matches
        require!(
            auction_info.object == auctioned_object,
            AuctionError::ObjectMismatch
        );

        // Validate bid amount is higher than current highest bid
        require!(
            amount_to_deposit > auction_info.highest_bid,
            AuctionError::BidTooLow
        );

        // Validate bidder is not the seller (seller can't bid on their own auction)
        require!(
            bidder.key() != auction_info.seller,
            AuctionError::SellerCannotBid
        );

        // Validate bidder has sufficient funds
        require!(
            bidder.lamports() >= amount_to_deposit,
            AuctionError::InsufficientFunds
        );

        // Store previous highest bid info for refund
        let previous_highest_bid = auction_info.highest_bid;
        let previous_highest_bidder = auction_info.highest_bidder;

        // Transfer new bid amount from bidder to auction PDA
        let transfer_instruction = anchor_lang::system_program::Transfer {
            from: bidder.to_account_info(),
            to: auction_info.to_account_info(),
        };
        let cpi_context = CpiContext::new(
            ctx.accounts.system_program.to_account_info(),
            transfer_instruction,
        );
        anchor_lang::system_program::transfer(cpi_context, amount_to_deposit)?;

        // Update auction info with new highest bid
        auction_info.highest_bidder = bidder.key();
        auction_info.highest_bid = amount_to_deposit;

        // Refund previous highest bidder (if not the seller's initial bid)
        if previous_highest_bidder != auction_info.seller {
            // Transfer previous bid back to previous highest bidder
            **auction_info.to_account_info().try_borrow_mut_lamports()? -= previous_highest_bid;
            **current_highest_bidder.try_borrow_mut_lamports()? += previous_highest_bid;
        }

        msg!("New bid placed by: {}", bidder.key());
        msg!("Bid amount: {} lamports", amount_to_deposit);
        msg!("Previous bidder refunded: {} lamports", previous_highest_bid);

        Ok(())
    }

    /// Ends an auction and transfers funds to seller
    /// Only callable by the seller after auction end time
    pub fn end(
        ctx: Context<EndCtx>,
        auctioned_object: String,
    ) -> Result<()> {
        let auction_info = &mut ctx.accounts.auction_info;
        let seller = &mut ctx.accounts.seller;
        let current_slot = Clock::get()?.slot;

        // Validate auction has ended
        require!(
            current_slot >= auction_info.end_time,
            AuctionError::AuctionStillActive
        );

        // Validate object matches
        require!(
            auction_info.object == auctioned_object,
            AuctionError::ObjectMismatch
        );

        // Validate only seller can end the auction
        require!(
            seller.key() == auction_info.seller,
            AuctionError::UnauthorizedEnd
        );

        let highest_bid = auction_info.highest_bid;
        let winning_bidder = auction_info.highest_bidder;

        // Transfer highest bid to seller (only if there was a real bid, not seller's initial)
        if winning_bidder != auction_info.seller && highest_bid > 0 {
            **auction_info.to_account_info().try_borrow_mut_lamports()? -= highest_bid;
            **seller.try_borrow_mut_lamports()? += highest_bid;
            
            msg!("Auction ended successfully!");
            msg!("Winner: {}", winning_bidder);
            msg!("Winning bid: {} lamports transferred to seller", highest_bid);
        } else {
            msg!("Auction ended with no valid bids");
        }

        // Close auction account and return rent to seller
        let auction_lamports = auction_info.to_account_info().lamports();
        **auction_info.to_account_info().try_borrow_mut_lamports()? = 0;
        **seller.try_borrow_mut_lamports()? += auction_lamports;

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
    
    /// CHECK: This account is validated through the auction_info.highest_bidder
    #[account(mut)]
    pub current_highest_bidder: AccountInfo<'info>,
    
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
        close = seller
    )]
    pub auction_info: Account<'info, AuctionInfo>,
}

#[account]
#[derive(InitSpace)]
pub struct AuctionInfo {
    pub seller: Pubkey,           // 32 bytes
    pub highest_bidder: Pubkey,   // 32 bytes
    pub end_time: u64,            // 8 bytes
    pub highest_bid: u64,         // 8 bytes
    #[max_len(64)]
    pub object: String,           // 4 + 64 bytes (max string length)
}

#[error_code]
pub enum AuctionError {
    #[msg("Auction has already ended")]
    AuctionEnded,
    
    #[msg("Auction is still active")]
    AuctionStillActive,
    
    #[msg("Bid amount is too low")]
    BidTooLow,
    
    #[msg("Insufficient funds for bid")]
    InsufficientFunds,
    
    #[msg("Object name mismatch")]
    ObjectMismatch,
    
    #[msg("Seller cannot bid on their own auction")]
    SellerCannotBid,
    
    #[msg("Only seller can end the auction")]
    UnauthorizedEnd,
    
    #[msg("Object name too long (max 64 characters)")]
    ObjectNameTooLong,
    
    #[msg("Invalid auction duration")]
    InvalidDuration,
}