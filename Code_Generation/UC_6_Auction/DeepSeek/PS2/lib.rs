use anchor_lang::prelude::*;
use anchor_lang::system_program;

declare_id!("8iXnt3BhRxs4hPtooEC6uSG53usY5gXwBEEvG29sRHQ6");

#[program]
pub mod auction_house {
    use super::*;

    pub fn start(
        ctx: Context<StartCtx>,
        auctioned_object: String,
        duration_slots: u64,
        starting_bid: u64,
    ) -> Result<()> {
        let auction_info = &mut ctx.accounts.auction_info;
        auction_info.seller = ctx.accounts.seller.key();
        auction_info.highest_bidder = ctx.accounts.seller.key();
        auction_info.end_time = Clock::get()?.slot + duration_slots;
        auction_info.highest_bid = starting_bid; // Store the starting bid
        auction_info.object = auctioned_object;

        Ok(())
    }

    pub fn bid(
        ctx: Context<BidCtx>,
        auctioned_object: String,
        amount_to_deposit: u64,
    ) -> Result<()> {
        // First, validate all conditions before making any changes
        require!(
            Clock::get()?.slot < ctx.accounts.auction_info.end_time,
            ErrorCode::AuctionEnded
        );
        require!(
            amount_to_deposit > ctx.accounts.auction_info.highest_bid,
            ErrorCode::BidTooLow
        );
        require!(
            ctx.accounts.current_highest_bidder.key() == ctx.accounts.auction_info.highest_bidder,
            ErrorCode::InvalidHighestBidder
        );

        // Transfer new bid amount to PDA
        let cpi_context = CpiContext::new(
            ctx.accounts.system_program.to_account_info(),
            system_program::Transfer {
                from: ctx.accounts.bidder.to_account_info(),
                to: ctx.accounts.auction_info.to_account_info(),
            },
        );
        system_program::transfer(cpi_context, amount_to_deposit)?;

        // Store values for refund before mutable borrow
        let previous_bid = ctx.accounts.auction_info.highest_bid;
        let previous_bidder = ctx.accounts.auction_info.highest_bidder;

        // Refund previous bidder if not seller (initial state) and if there was a previous bid
        if previous_bid > 0 && previous_bidder != ctx.accounts.auction_info.seller {
            let rent = Rent::get()?;
            let min_rent = rent.minimum_balance(std::mem::size_of::<AuctionInfo>());
            let auction_info_account = ctx.accounts.auction_info.to_account_info();
            let current_balance = auction_info_account.lamports();
            
            require!(
                current_balance >= min_rent + amount_to_deposit,
                ErrorCode::InsufficientFunds
            );

            // Use direct lamport manipulation to avoid borrow conflicts
            **auction_info_account.try_borrow_mut_lamports()? -= previous_bid;
            **ctx
                .accounts
                .current_highest_bidder
                .try_borrow_mut_lamports()? += previous_bid;
        }

        // Now update auction state (single mutable borrow at the end)
        let auction_info = &mut ctx.accounts.auction_info;
        auction_info.highest_bid = amount_to_deposit;
        auction_info.highest_bidder = ctx.accounts.bidder.key();

        Ok(())
    }

    pub fn end(ctx: Context<EndCtx>, _auctioned_object: String) -> Result<()> {
        let auction_info = &mut ctx.accounts.auction_info;
        
        // Validate auction has ended
        require!(
            Clock::get()?.slot >= auction_info.end_time,
            ErrorCode::AuctionNotEnded
        );
        
        // Validate caller is seller
        require!(
            ctx.accounts.seller.key() == auction_info.seller,
            ErrorCode::UnauthorizedSeller
        );

        // Calculate proceeds (handle case where no bids were placed)
        let rent = Rent::get()?;
        let min_rent = rent.minimum_balance(std::mem::size_of::<AuctionInfo>());
        let auction_info_account = ctx.accounts.auction_info.to_account_info();
        let current_balance = auction_info_account.lamports();
        
        // Ensure we don't underflow - if balance is less than min_rent, proceeds is 0
        let proceeds = if current_balance > min_rent {
            current_balance - min_rent
        } else {
            0
        };

        // Only transfer if there are proceeds
        if proceeds > 0 {
            **auction_info_account.try_borrow_mut_lamports()? -= proceeds;
            **ctx.accounts.seller.try_borrow_mut_lamports()? += proceeds;
        }

        // Close account and return rent
        ctx.accounts.auction_info.close(ctx.accounts.seller.to_account_info())?;

        Ok(())
    }
}

#[account]
pub struct AuctionInfo {
    pub seller: Pubkey,
    pub highest_bidder: Pubkey,
    pub end_time: u64,
    pub highest_bid: u64,
    pub object: String,
}

#[derive(Accounts)]
#[instruction(auctioned_object: String, duration_slots: u64, starting_bid: u64)]
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
        bump
    )]
    pub auction_info: Account<'info, AuctionInfo>,
    /// CHECK: Validated in handler logic
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

#[error_code]
pub enum ErrorCode {
    #[msg("Auction has not ended yet")]
    AuctionNotEnded,
    #[msg("Bid must be higher than current highest bid")]
    BidTooLow,
    #[msg("Only seller can perform this action")]
    UnauthorizedSeller,
    #[msg("Insufficient funds in auction account")]
    InsufficientFunds,
    #[msg("Provided highest bidder account does not match current highest bidder")]
    InvalidHighestBidder,
    #[msg("Auction has already ended")]
    AuctionEnded,
}