use anchor_lang::prelude::*;
use anchor_lang::system_program;

declare_id!("4iuo8xEfhLrqhosoqYitcgUGofAJVe6xqhz4WnRbjk4C");

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
        let clock = Clock::get()?;

        auction_info.seller = ctx.accounts.seller.key();
        auction_info.highest_bidder = ctx.accounts.seller.key();
        auction_info.end_time = clock.slot + duration_slots;
        auction_info.highest_bid = starting_bid;
        auction_info.object = auctioned_object;

        Ok(())
    }

    pub fn bid(
        ctx: Context<BidCtx>,
        auctioned_object: String,
        amount_to_deposit: u64,
    ) -> Result<()> {
        let clock = Clock::get()?;
        
        // First, get all necessary data from auction_info without mutable borrow
        let seller = ctx.accounts.auction_info.seller;
        let end_time = ctx.accounts.auction_info.end_time;
        let current_highest_bid = ctx.accounts.auction_info.highest_bid;
        let current_highest_bidder = ctx.accounts.auction_info.highest_bidder;

        // Validate before mutable borrow
        require!(clock.slot < end_time, AuctionError::AuctionEnded);
        require!(amount_to_deposit > current_highest_bid, AuctionError::BidTooLow);

        // Transfer new bid to auction PDA using CPI
        let transfer_ix = system_program::Transfer {
            from: ctx.accounts.bidder.to_account_info(),
            to: ctx.accounts.auction_info.to_account_info(),
        };
        let cpi_context = CpiContext::new(
            ctx.accounts.system_program.to_account_info(),
            transfer_ix,
        );
        system_program::transfer(cpi_context, amount_to_deposit)?;

        // Now update auction_info with mutable borrow
        let auction_info = &mut ctx.accounts.auction_info;
        auction_info.highest_bidder = ctx.accounts.bidder.key();
        auction_info.highest_bid = amount_to_deposit;

        // Refund previous bidder if not seller using direct lamport manipulation
        if current_highest_bidder != seller {
            // Get current lamport values
            let auction_lamports = ctx.accounts.auction_info.to_account_info().lamports();
            let bidder_lamports = ctx.accounts.current_highest_bidder.lamports();
            
            // Calculate new values with safe arithmetic
            let new_auction_lamports = auction_lamports
                .checked_sub(current_highest_bid)
                .ok_or(AuctionError::InsufficientFunds)?;
                
            let new_bidder_lamports = bidder_lamports
                .checked_add(current_highest_bid)
                .ok_or(AuctionError::Overflow)?;

            // Set new lamport values
            **ctx.accounts.auction_info.to_account_info().try_borrow_mut_lamports()? = new_auction_lamports;
            **ctx.accounts.current_highest_bidder.try_borrow_mut_lamports()? = new_bidder_lamports;
        }

        Ok(())
    }

    pub fn end(ctx: Context<EndCtx>, auctioned_object: String) -> Result<()> {
        let clock = Clock::get()?;
        
        // First, get all necessary data from auction_info without mutable borrow
        let end_time = ctx.accounts.auction_info.end_time;
        let highest_bid = ctx.accounts.auction_info.highest_bid;

        // Validate before mutable borrow
        require!(clock.slot >= end_time, AuctionError::AuctionNotEnded);

        // Get rent requirements
        let rent = Rent::get()?;
        let auction_account_info = ctx.accounts.auction_info.to_account_info();
        let rent_minimum = rent.minimum_balance(auction_account_info.data_len());
        
        // Get current lamport values
        let auction_lamports = auction_account_info.lamports();
        let seller_lamports = ctx.accounts.seller.lamports();
        
        // Calculate available funds (total minus rent)
        let available_funds = auction_lamports.checked_sub(rent_minimum)
            .ok_or(AuctionError::InsufficientFunds)?;
        
        // Transfer available funds to seller (this should be the highest bid)
        let new_auction_lamports = auction_lamports
            .checked_sub(available_funds)
            .ok_or(AuctionError::InsufficientFunds)?;
            
        let new_seller_lamports = seller_lamports
            .checked_add(available_funds)
            .ok_or(AuctionError::Overflow)?;

        // Set new lamport values
        **auction_account_info.try_borrow_mut_lamports()? = new_auction_lamports;
        **ctx.accounts.seller.try_borrow_mut_lamports()? = new_seller_lamports;

        // Close auction account and return rent to seller
        let auction_info_account = &mut ctx.accounts.auction_info;
        auction_info_account.close(ctx.accounts.seller.to_account_info())?;

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
        space = 8 + 32 + 32 + 8 + 8 + 4 + auctioned_object.len(),
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
        bump,
        constraint = auction_info.object == auctioned_object
    )]
    pub auction_info: Account<'info, AuctionInfo>,
    /// CHECK: Validated in bid logic
    #[account(
        mut,
        constraint = current_highest_bidder.key() == auction_info.highest_bidder
    )]
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
        seeds = [auctioned_object.as_bytes()],
        bump,
        constraint = auction_info.seller == seller.key(),
        constraint = auction_info.object == auctioned_object
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
    #[msg("Auction has already ended")]
    AuctionEnded,
    #[msg("Auction has not ended yet")]
    AuctionNotEnded,
    #[msg("Bid amount is too low")]
    BidTooLow,
    #[msg("Insufficient funds in auction account")]
    InsufficientFunds,
    #[msg("Lamport addition would overflow")]
    Overflow,
}