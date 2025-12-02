use anchor_lang::prelude::*;
use anchor_lang::system_program;
use pyth_sdk_solana::load_price_feed_from_account_info;
use std::str::FromStr;

declare_id!("AcD1XuELc7SdNcLtdvQx5rYhZ5j5iHycZntYA61ubemp");

#[program]
pub mod price_betting {
    use super::*;

    /// Initialize a new bet with specified parameters
    pub fn init(ctx: Context<InitCtx>, delay: u64, wager: u64, rate: u64) -> Result<()> {
        let bet_info = &mut ctx.accounts.bet_info;
        let owner = &ctx.accounts.owner;
        
        // Set bet parameters
        bet_info.owner = owner.key();
        bet_info.player = Pubkey::default(); // Will be set when player joins
        bet_info.wager = wager;
        bet_info.deadline = Clock::get()?.unix_timestamp as u64 + delay;
        bet_info.rate = rate;

        // Transfer wager from owner to bet PDA
        system_program::transfer(
            CpiContext::new(
                ctx.accounts.system_program.to_account_info(),
                system_program::Transfer {
                    from: owner.to_account_info(),
                    to: bet_info.to_account_info(),
                },
            ),
            wager,
        )?;

        msg!("Bet initialized with wager: {}, rate: {}, deadline: {}", wager, rate, bet_info.deadline);
        Ok(())
    }

    /// Join an existing bet by matching the wager
    pub fn join(ctx: Context<JoinCtx>) -> Result<()> {
        let bet_info = &mut ctx.accounts.bet_info;
        let player = &ctx.accounts.player;

        // Ensure no player has joined yet
        require!(bet_info.player == Pubkey::default(), BettingError::BetAlreadyJoined);

        // Ensure deadline hasn't passed
        require!(
            (Clock::get()?.unix_timestamp as u64) < bet_info.deadline,
            BettingError::DeadlinePassed
        );

        // Set player
        bet_info.player = player.key();

        // Transfer matching wager from player to bet PDA
        system_program::transfer(
            CpiContext::new(
                ctx.accounts.system_program.to_account_info(),
                system_program::Transfer {
                    from: player.to_account_info(),
                    to: bet_info.to_account_info(),
                },
            ),
            bet_info.wager,
        )?;

        msg!("Player {} joined bet with wager: {}", player.key(), bet_info.wager);
        Ok(())
    }

    /// Claim winnings if price condition is met
    pub fn win(ctx: Context<WinCtx>) -> Result<()> {
        let bet_info = &ctx.accounts.bet_info;
        let player = &ctx.accounts.player;
        let price_feed = &ctx.accounts.price_feed;

        // Ensure deadline hasn't passed
        let current_time = Clock::get()?.unix_timestamp as u64;
        require!(
            current_time < bet_info.deadline,
            BettingError::DeadlinePassed
        );

        // Validate and load price feed
        let price_feed_data = load_price_feed_from_account_info(price_feed)
            .map_err(|_| BettingError::InvalidPriceFeed)?;

        // Check price feed staleness (5 minutes max)
        let price_timestamp = price_feed_data.get_price_unchecked().publish_time;
        require!(
            current_time - (price_timestamp as u64) < 300, // 5 minutes
            BettingError::StalePriceFeed
        );

        // Get current price
        let price = price_feed_data.get_price_unchecked();

        // Check if price condition is met (current price >= bet rate)
        let current_price = price.price as u64;
        require!(current_price >= bet_info.rate, BettingError::NoWin);

        // Get the entire PDA balance (includes wagers + rent)
        let total_pot = bet_info.to_account_info().lamports();

        // Transfer entire PDA balance to player
        **bet_info.to_account_info().try_borrow_mut_lamports()? = 0;
        **player.to_account_info().try_borrow_mut_lamports()? += total_pot;

        msg!("Player won! Current price: {}, Target rate: {}, Pot: {}", 
             current_price, bet_info.rate, total_pot);
        Ok(())
    }

    /// Owner can claim funds back after deadline if player hasn't won
    pub fn timeout(ctx: Context<TimeoutCtx>) -> Result<()> {
        let bet_info = &ctx.accounts.bet_info;
        let owner = &ctx.accounts.owner;

        // Ensure caller is the bet owner
        require!(bet_info.owner == owner.key(), BettingError::UnauthorizedOwner);

        // Ensure deadline has passed
        require!(
            (Clock::get()?.unix_timestamp as u64) >= bet_info.deadline,
            BettingError::DeadlineNotPassed
        );

        // Get the entire PDA balance (includes wagers + rent)
        let total_pot = bet_info.to_account_info().lamports();

        // Transfer entire PDA balance back to owner
        **bet_info.to_account_info().try_borrow_mut_lamports()? = 0;
        **owner.to_account_info().try_borrow_mut_lamports()? += total_pot;

        msg!("Owner reclaimed pot after timeout: {}", total_pot);
        Ok(())
    }
}

#[derive(Accounts)]
pub struct InitCtx<'info> {
    #[account(mut)]
    pub owner: Signer<'info>,
    
    #[account(
        init,
        payer = owner,
        space = 8 + std::mem::size_of::<OracleBetInfo>(),
        seeds = [owner.key().as_ref()],
        bump
    )]
    pub bet_info: Account<'info, OracleBetInfo>,
    
    pub system_program: Program<'info, System>,
}

#[derive(Accounts)]
pub struct JoinCtx<'info> {
    #[account(mut)]
    pub player: Signer<'info>,
    
    /// CHECK: Referenced for validation but not modified
    pub owner: UncheckedAccount<'info>,
    
    #[account(
        mut,
        seeds = [owner.key().as_ref()],
        bump
    )]
    pub bet_info: Account<'info, OracleBetInfo>,
    
    pub system_program: Program<'info, System>,
}

#[derive(Accounts)]
pub struct WinCtx<'info> {
    #[account(mut)]
    pub player: Signer<'info>,
    
    /// CHECK: Referenced for PDA seed derivation
    pub owner: AccountInfo<'info>,
    
    #[account(
        mut,
        seeds = [owner.key().as_ref()],
        bump,
        constraint = bet_info.player == player.key() @ BettingError::UnauthorizedPlayer
    )]
    pub bet_info: Account<'info, OracleBetInfo>,
    
    /// CHECK: Pyth price feed account - validated in instruction logic
    pub price_feed: AccountInfo<'info>,
    
    pub system_program: Program<'info, System>,
}

#[derive(Accounts)]
pub struct TimeoutCtx<'info> {
    #[account(mut)]
    pub owner: Signer<'info>,
    
    #[account(
        mut,
        seeds = [owner.key().as_ref()],
        bump
    )]
    pub bet_info: Account<'info, OracleBetInfo>,
    
    pub system_program: Program<'info, System>,
}

#[account]
pub struct OracleBetInfo {
    pub owner: Pubkey,
    pub player: Pubkey,
    pub wager: u64,
    pub deadline: u64,
    pub rate: u64,
}

#[error_code]
pub enum BettingError {
    #[msg("Bet has already been joined by another player")]
    BetAlreadyJoined,
    
    #[msg("Deadline has passed")]
    DeadlinePassed,
    
    #[msg("Deadline has not passed yet")]
    DeadlineNotPassed,
    
    #[msg("Only the bet owner can perform this action")]
    UnauthorizedOwner,
    
    #[msg("Only the registered player can perform this action")]
    UnauthorizedPlayer,
    
    #[msg("Invalid price feed account")]
    InvalidPriceFeed,
    
    #[msg("Price feed data is too stale")]
    StalePriceFeed,
    
    #[msg("Unable to get current price from feed")]
    InvalidPrice,
    
    #[msg("Price condition not met - player cannot win")]
    NoWin,
}