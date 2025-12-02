use anchor_lang::prelude::*;
use anchor_lang::solana_program::clock::Clock;
use anchor_lang::solana_program::sysvar::rent::Rent;
use pyth_sdk_solana::load_price_feed_from_account_info;
use std::str::FromStr;


declare_id!("3FYgUoJjMxr2KNS8t9BgnHWmzGWM4kUXpWhw6rcK9qxV");

#[program]
pub mod price_bet {
    use super::*;

    /// Initialize a new bet
    /// Only the owner can call this function
    pub fn init(ctx: Context<InitCtx>, delay: u64, wager: u64, rate: u64) -> Result<()> {
        require!(wager > 0, BettingError::InvalidWager);
        require!(rate > 0, BettingError::InvalidRate);

        let clock = Clock::get()?;
        let current_time = clock.unix_timestamp as u64;
        
        let bet_info = &mut ctx.accounts.bet_info;
        bet_info.owner = ctx.accounts.owner.key();
        bet_info.player = Pubkey::default(); // Will be set when player joins
        bet_info.wager = wager;
        bet_info.deadline = current_time + delay;
        bet_info.rate = rate;

        // Transfer wager from owner to bet PDA
        let transfer_ix = anchor_lang::solana_program::system_instruction::transfer(
            &ctx.accounts.owner.key(),
            &bet_info.key(),
            wager,
        );

        anchor_lang::solana_program::program::invoke(
            &transfer_ix,
            &[
                ctx.accounts.owner.to_account_info(),
                bet_info.to_account_info(),
                ctx.accounts.system_program.to_account_info(),
            ],
        )?;

        emit!(BetInitialized {
            owner: ctx.accounts.owner.key(),
            wager,
            deadline: bet_info.deadline,
            rate,
        });

        Ok(())
    }

    /// Join an existing bet
    /// Only a player can call this function with matching wager
    pub fn join(ctx: Context<JoinCtx>) -> Result<()> {
        let bet_info = &mut ctx.accounts.bet_info;
        
        require!(bet_info.player == Pubkey::default(), BettingError::BetAlreadyJoined);
        require!(ctx.accounts.player.key() != bet_info.owner, BettingError::OwnerCannotJoin);

        let clock = Clock::get()?;
        let current_time = clock.unix_timestamp as u64;
        require!(current_time < bet_info.deadline, BettingError::BetExpired);

        bet_info.player = ctx.accounts.player.key();

        // Transfer matching wager from player to bet PDA
        let transfer_ix = anchor_lang::solana_program::system_instruction::transfer(
            &ctx.accounts.player.key(),
            &bet_info.key(),
            bet_info.wager,
        );

        anchor_lang::solana_program::program::invoke(
            &transfer_ix,
            &[
                ctx.accounts.player.to_account_info(),
                bet_info.to_account_info(),
                ctx.accounts.system_program.to_account_info(),
            ],
        )?;

        emit!(BetJoined {
            player: ctx.accounts.player.key(),
            owner: bet_info.owner,
        });

        Ok(())
    }

    /// Player wins if current price is above the bet rate
    /// Only the player can call this function
    pub fn win(ctx: Context<WinCtx>) -> Result<()> {
        let bet_info = &ctx.accounts.bet_info;
        
        require!(bet_info.player == ctx.accounts.player.key(), BettingError::NotPlayer);
        require!(bet_info.player != Pubkey::default(), BettingError::BetNotJoined);

        let clock = Clock::get()?;
        let current_time = clock.unix_timestamp as u64;
        require!(current_time >= bet_info.deadline, BettingError::BetNotExpired);

        // Validate Pyth price feed
        let price_feed_account = &ctx.accounts.price_feed;
        
        // Verify the price feed account owner is Pyth
        let pyth_program_id = Pubkey::from_str("FsJ3A3u2vn5cTVofAjvy6y5kwABJAqYWpe4975bi2epH")
            .map_err(|_| BettingError::InvalidPythProgram)?;
        require!(
            price_feed_account.owner == &pyth_program_id,
            BettingError::InvalidPriceFeeedOwner
        );

        // Load and validate price feed
        let price_feed = load_price_feed_from_account_info(price_feed_account)
            .map_err(|_| BettingError::InvalidPriceFeed)?;

        // Check price staleness (must be updated within last 60 seconds)
        let max_staleness = 60;
        let price_age = current_time - price_feed.get_price_unchecked().publish_time as u64;
        require!(price_age <= max_staleness, BettingError::PriceTooStale);

        // Get current price
        let current_price = price_feed
            .get_price_unchecked()
            .price;
        
        require!(current_price > 0, BettingError::InvalidPrice);

        // Convert rate to same format as Pyth price (Pyth uses different exponent)
        let current_price_scaled = current_price as u64;
        
        // Player wins if current price is above bet rate
        require!(current_price_scaled > bet_info.rate, BettingError::BetLost);

        // Get current lamport balance of bet_info PDA and transfer everything
        let bet_info_lamports = bet_info.to_account_info().lamports();
        
        // Transfer all funds to player (this effectively closes the account)
        **bet_info.to_account_info().try_borrow_mut_lamports()? = 0;
        **ctx.accounts.player.try_borrow_mut_lamports()? += bet_info_lamports;

        emit!(BetWon {
            player: ctx.accounts.player.key(),
            amount: bet_info_lamports,
            winning_price: current_price_scaled,
            bet_rate: bet_info.rate,
        });

        Ok(())
    }

    /// Owner can reclaim funds if bet expires without a player or after timeout
    /// Only the owner can call this function
    pub fn timeout(ctx: Context<TimeoutCtx>) -> Result<()> {
        let bet_info = &ctx.accounts.bet_info;
        
        require!(bet_info.owner == ctx.accounts.owner.key(), BettingError::NotOwner);

        let clock = Clock::get()?;
        let current_time = clock.unix_timestamp as u64;
        require!(current_time >= bet_info.deadline, BettingError::BetNotExpired);

        // Get current lamport balance of bet_info PDA and transfer everything
        let bet_info_lamports = bet_info.to_account_info().lamports();
        
        // Transfer all funds back to owner (this effectively closes the account)
        **bet_info.to_account_info().try_borrow_mut_lamports()? = 0;
        **ctx.accounts.owner.try_borrow_mut_lamports()? += bet_info_lamports;

        emit!(BetTimedOut {
            owner: ctx.accounts.owner.key(),
            amount: bet_info_lamports,
        });

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
        space = 8 + OracleBetInfo::INIT_SPACE,
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
    
    /// CHECK: Referenced for validation only
    pub owner: AccountInfo<'info>,
    
    #[account(
        mut,
        seeds = [owner.key().as_ref()],
        bump,
        constraint = bet_info.owner == owner.key() @ BettingError::InvalidOwner
    )]
    pub bet_info: Account<'info, OracleBetInfo>,
    
    pub system_program: Program<'info, System>,
}

#[derive(Accounts)]
pub struct WinCtx<'info> {
    #[account(mut)]
    pub player: Signer<'info>,
    
    /// CHECK: Referenced for validation only
    pub owner: AccountInfo<'info>,
    
    #[account(
        mut,
        seeds = [owner.key().as_ref()],
        bump,
        constraint = bet_info.owner == owner.key() @ BettingError::InvalidOwner
    )]
    pub bet_info: Account<'info, OracleBetInfo>,
    
    /// CHECK: Pyth price feed account, validated in instruction
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
        bump,
        constraint = bet_info.owner == owner.key() @ BettingError::InvalidOwner
    )]
    pub bet_info: Account<'info, OracleBetInfo>,
    
    pub system_program: Program<'info, System>,
}

#[account]
#[derive(InitSpace)]
pub struct OracleBetInfo {
    pub owner: Pubkey,
    pub player: Pubkey,
    pub wager: u64,
    pub deadline: u64,
    pub rate: u64,
}

#[event]
pub struct BetInitialized {
    pub owner: Pubkey,
    pub wager: u64,
    pub deadline: u64,
    pub rate: u64,
}

#[event]
pub struct BetJoined {
    pub player: Pubkey,
    pub owner: Pubkey,
}

#[event]
pub struct BetWon {
    pub player: Pubkey,
    pub amount: u64,
    pub winning_price: u64,
    pub bet_rate: u64,
}

#[event]
pub struct BetTimedOut {
    pub owner: Pubkey,
    pub amount: u64,
}

#[error_code]
pub enum BettingError {
    #[msg("Invalid delay provided")]
    InvalidDelay,
    #[msg("Invalid wager amount")]
    InvalidWager,
    #[msg("Invalid rate provided")]
    InvalidRate,
    #[msg("Bet has already been joined")]
    BetAlreadyJoined,
    #[msg("Owner cannot join their own bet")]
    OwnerCannotJoin,
    #[msg("Bet has expired")]
    BetExpired,
    #[msg("Only the player can call this function")]
    NotPlayer,
    #[msg("Bet has not been joined yet")]
    BetNotJoined,
    #[msg("Bet has not expired yet")]
    BetNotExpired,
    #[msg("Invalid price feed owner")]
    InvalidPriceFeeedOwner,
    #[msg("Invalid price feed data")]
    InvalidPriceFeed,
    #[msg("Price data is too stale")]
    PriceTooStale,
    #[msg("Invalid price from oracle")]
    InvalidPrice,
    #[msg("Bet lost - current price is not above bet rate")]
    BetLost,
    #[msg("Only the owner can call this function")]
    NotOwner,
    #[msg("Invalid owner reference")]
    InvalidOwner,
    #[msg("Invalid Pyth program ID")]
    InvalidPythProgram,
}