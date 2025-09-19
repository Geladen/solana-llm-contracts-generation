use anchor_lang::prelude::*;
use anchor_lang::system_program;

declare_id!("EBrumJ2gteq2gzmfcVrMqyakDy8KzboVrPhz4JET6S8r");

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

        // Initialize auction data
        auction_info.seller = ctx.accounts.seller.key();
        auction_info.highest_bidder = ctx.accounts.seller.key();
        auction_info.end_time = clock.slot + duration_slots;
        auction_info.highest_bid = starting_bid;
        auction_info.object = auctioned_object;

        Ok(())
    }

    pub fn bid(ctx: Context<BidCtx>, auctioned_object: String, amount_to_deposit: u64) -> Result<()> {
        // First, get all necessary data without holding mutable references
        let clock = Clock::get()?;
        let auction_info = &ctx.accounts.auction_info;
        
        // Validate auction state
        require!(clock.slot < auction_info.end_time, AuctionError::AuctionEnded);
        require!(
            amount_to_deposit > auction_info.highest_bid,
            AuctionError::BidTooLow
        );

        // Store current state before updating
        let previous_bid_amount = auction_info.highest_bid;
        let previous_bidder = auction_info.highest_bidder;
        let seller = auction_info.seller;

        // Transfer new bid amount using CPI
        let cpi_context = CpiContext::new(
            ctx.accounts.system_program.to_account_info(),
            system_program::Transfer {
                from: ctx.accounts.bidder.to_account_info(),
                to: ctx.accounts.auction_info.to_account_info(),
            },
        );
        system_program::transfer(cpi_context, amount_to_deposit)?;

        // Refund previous bidder if not seller using direct lamport manipulation
        if previous_bidder != seller {
            let auction_info_lamports = **ctx.accounts.auction_info.to_account_info().try_borrow_mut_lamports()?;
            let previous_bidder_lamports = **ctx.accounts.current_highest_bidder.try_borrow_mut_lamports()?;
            
            **ctx.accounts.auction_info.to_account_info().try_borrow_mut_lamports()? = auction_info_lamports - previous_bid_amount;
            **ctx.accounts.current_highest_bidder.try_borrow_mut_lamports()? = previous_bidder_lamports + previous_bid_amount;
        }

        // Now update auction state after all transfers are complete
        let auction_info = &mut ctx.accounts.auction_info;
        auction_info.highest_bid = amount_to_deposit;
        auction_info.highest_bidder = ctx.accounts.bidder.key();

        Ok(())
    }

    pub fn end(ctx: Context<EndCtx>, auctioned_object: String) -> Result<()> {
        // First get data without mutable reference
        let clock = Clock::get()?;
        let auction_info = &ctx.accounts.auction_info;
        
        // Validate auction can be ended
        require!(clock.slot >= auction_info.end_time, AuctionError::AuctionActive);

        // Only transfer funds if there are valid bids
        if auction_info.highest_bidder != auction_info.seller {
            let highest_bid = auction_info.highest_bid;
            
            // Transfer funds to seller using direct lamport manipulation
            let auction_info_lamports = **ctx.accounts.auction_info.to_account_info().try_borrow_mut_lamports()?;
            let seller_lamports = **ctx.accounts.seller.to_account_info().try_borrow_mut_lamports()?;
            
            **ctx.accounts.auction_info.to_account_info().try_borrow_mut_lamports()? = auction_info_lamports - highest_bid;
            **ctx.accounts.seller.to_account_info().try_borrow_mut_lamports()? = seller_lamports + highest_bid;
        }

        // Close account and return rent to seller (handled by Anchor's close attribute)
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
    /// CHECK: Validated through auction_info.highest_bidder
    #[account(mut, address = auction_info.highest_bidder)]
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
        has_one = seller,
        close = seller // This handles rent return
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
    #[max_len(50)]
    pub object: String,
}

#[error_code]
pub enum AuctionError {
    #[msg("Auction has already ended")]
    AuctionEnded,
    #[msg("Auction is still active")]
    AuctionActive,
    #[msg("Bid amount is too low")]
    BidTooLow,
    #[msg("No valid bids placed")]
    NoBids,
}