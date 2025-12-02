use anchor_lang::prelude::*;

declare_id!("Cz3aneYmJ8ptPAVLMsyN31xjoi2WnYVta55agFZQtMMp");

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

        // Initialize auction info
        auction_info.seller = seller.key();
        auction_info.highest_bidder = seller.key(); // Seller is initial highest bidder
        auction_info.end_time = clock.slot + duration_slots;
        auction_info.highest_bid = starting_bid;
        auction_info.object = auctioned_object;

        msg!("Auction started for object: {}, ending at slot: {}", 
             auction_info.object, auction_info.end_time);

        Ok(())
    }

    pub fn bid(
        ctx: Context<BidCtx>,
        auctioned_object: String,
        amount_to_deposit: u64,
    ) -> Result<()> {
        let auction_info = &mut ctx.accounts.auction_info;
        let bidder = &mut ctx.accounts.bidder;
        let current_highest_bidder = &mut ctx.accounts.current_highest_bidder;
        let clock = Clock::get()?;

        // Check if auction has ended
        require!(clock.slot < auction_info.end_time, AuctionError::AuctionEnded);

        // Check if bid amount is higher than current highest bid
        require!(
            amount_to_deposit > auction_info.highest_bid,
            AuctionError::BidTooLow
        );

        // Transfer new bid amount from bidder to auction PDA
        let transfer_instruction = anchor_lang::solana_program::system_instruction::transfer(
            &bidder.key(),
            &auction_info.key(),
            amount_to_deposit,
        );
        anchor_lang::solana_program::program::invoke(
            &transfer_instruction,
            &[
                bidder.to_account_info(),
                auction_info.to_account_info(),
                ctx.accounts.system_program.to_account_info(),
            ],
        )?;

        // Refund previous highest bidder (if it's not the seller/initial state)
        if auction_info.highest_bidder != auction_info.seller && auction_info.highest_bid > 0 {
            **auction_info.to_account_info().try_borrow_mut_lamports()? -= auction_info.highest_bid;
            **current_highest_bidder.try_borrow_mut_lamports()? += auction_info.highest_bid;
        }

        // Update auction info with new highest bid
        auction_info.highest_bidder = bidder.key();
        auction_info.highest_bid = amount_to_deposit;

        msg!("New highest bid: {} lamports from {}", amount_to_deposit, bidder.key());

        Ok(())
    }

    pub fn end(
        ctx: Context<EndCtx>,
        auctioned_object: String,
    ) -> Result<()> {
        let auction_info = &mut ctx.accounts.auction_info;
        let seller = &mut ctx.accounts.seller;
        let clock = Clock::get()?;

        // Check if auction has ended
        require!(clock.slot >= auction_info.end_time, AuctionError::AuctionNotEnded);

        // Transfer all lamports from auction PDA to seller
        let auction_lamports = auction_info.to_account_info().lamports();
        
        **auction_info.to_account_info().try_borrow_mut_lamports()? = 0;
        **seller.try_borrow_mut_lamports()? += auction_lamports;

        msg!("Auction ended. Winner: {}, Amount: {} lamports", 
             auction_info.highest_bidder, auction_info.highest_bid);

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
    /// CHECK: This account is the current highest bidder and will receive refund
    #[account(mut)]
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
        bump
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
    #[max_len(100)]
    pub object: String,           // 4 + 100 bytes (max length)
}

#[error_code]
pub enum AuctionError {
    #[msg("Auction has already ended")]
    AuctionEnded,
    #[msg("Bid amount is too low")]
    BidTooLow,
    #[msg("Auction has not ended yet")]
    AuctionNotEnded,
    #[msg("Only the seller can end the auction")]
    UnauthorizedSeller,
}