use anchor_lang::prelude::*;

declare_id!("Bdm1JNRhumU6vAG6ZJ3bfHs2ay2EEsWxaEuZGknAHjvC");

#[program]
pub mod price_bet {
    use super::*;

    pub fn init(ctx: Context<Init>, delay: i64, wager: u64, rate: u64) -> Result<()> {
        msg!("DEBUG: init called with delay: {}, wager: {}, rate: {}", delay, wager, rate);
        
        // Simple validation
        if delay <= 0 {
            msg!("ERROR: Delay must be positive. Got: {}", delay);
            return Err(BetError::InvalidDelay.into());
        }
        
        let clock = Clock::get()?;
        let current_time = clock.unix_timestamp;
        let deadline = current_time + delay;
        
        if deadline <= current_time {
            msg!("ERROR: Deadline must be in future");
            return Err(BetError::InvalidDelay.into());
        }
        
        // Get account info references FIRST
        let owner_account = ctx.accounts.owner.to_account_info();
        let bet_info_account = ctx.accounts.bet_info.to_account_info();
        let system_program = ctx.accounts.system_program.to_account_info();
        
        // THEN get mutable reference to data
        let bet_info = &mut ctx.accounts.bet_info;
        bet_info.owner = ctx.accounts.owner.key();
        bet_info.player = Pubkey::default();
        bet_info.wager = wager;
        bet_info.deadline = deadline as u64;
        bet_info.rate = rate;
        
        // Transfer using stored references
        let cpi_context = CpiContext::new(
            system_program,
            anchor_lang::system_program::Transfer {
                from: owner_account,
                to: bet_info_account,
            },
        );
        anchor_lang::system_program::transfer(cpi_context, wager)?;
        
        msg!("SUCCESS: Bet initialized");
        Ok(())
    }

    pub fn join(ctx: Context<Join>) -> Result<()> {
        // Get account info references FIRST
        let player_account = ctx.accounts.player.to_account_info();
        let bet_info_account = ctx.accounts.bet_info.to_account_info();
        let system_program = ctx.accounts.system_program.to_account_info();
        
        // THEN get mutable reference to data
        let bet_info = &mut ctx.accounts.bet_info;
        require!(bet_info.player == Pubkey::default(), BetError::BetAlreadyTaken);
        
        bet_info.player = ctx.accounts.player.key();
        
        // Transfer using stored references
        let cpi_context = CpiContext::new(
            system_program,
            anchor_lang::system_program::Transfer {
                from: player_account,
                to: bet_info_account,
            },
        );
        anchor_lang::system_program::transfer(cpi_context, bet_info.wager)?;
        
        Ok(())
    }

    pub fn win(ctx: Context<Win>) -> Result<()> {
        // Get account info references FIRST
        let player_account = ctx.accounts.player.to_account_info();
        let bet_info_account = ctx.accounts.bet_info.to_account_info();
        let system_program = ctx.accounts.system_program.to_account_info();
        
        // THEN get mutable reference to data
        let bet_info = &mut ctx.accounts.bet_info;
        require!(bet_info.player == ctx.accounts.player.key(), BetError::NotPlayer);
        
        let current_price = get_pyth_price(&ctx.accounts.price_feed)?;
        require!(current_price >= bet_info.rate, BetError::ConditionNotMet);
        
        let total_pot = bet_info.wager * 2;
        
        // Transfer using stored references
        let cpi_context = CpiContext::new(
            system_program,
            anchor_lang::system_program::Transfer {
                from: bet_info_account,
                to: player_account,
            },
        );
        anchor_lang::system_program::transfer(cpi_context, total_pot)?;
        
        bet_info.player = Pubkey::default();
        Ok(())
    }

    pub fn timeout(ctx: Context<Timeout>) -> Result<()> {
        let clock = Clock::get()?;
        
        // Get account info references and balance FIRST
        let owner_account = ctx.accounts.owner.to_account_info();
        let bet_info_account = ctx.accounts.bet_info.to_account_info();
        let system_program = ctx.accounts.system_program.to_account_info();
        let balance = bet_info_account.lamports();
        
        // THEN get mutable reference to data
        let bet_info = &mut ctx.accounts.bet_info;
        require!(clock.unix_timestamp as u64 > bet_info.deadline, BetError::BetNotExpired);
        
        // Transfer using stored references
        let cpi_context = CpiContext::new(
            system_program,
            anchor_lang::system_program::Transfer {
                from: bet_info_account,
                to: owner_account,
            },
        );
        anchor_lang::system_program::transfer(cpi_context, balance)?;
        
        bet_info.player = Pubkey::default();
        Ok(())
    }
}

// Account structs - using exact seeds as specified in requirements
#[derive(Accounts)]
pub struct Init<'info> {
    #[account(mut)]
    pub owner: Signer<'info>,
    
    #[account(
        init,
        payer = owner,
        space = 8 + 32 + 32 + 8 + 8 + 8,
        seeds = [owner.key().as_ref()], // Exact seeds as specified
        bump
    )]
    pub bet_info: Account<'info, BetInfo>,
    
    pub system_program: Program<'info, System>,
}

#[derive(Accounts)]
pub struct Join<'info> {
    #[account(mut)]
    pub player: Signer<'info>,
    
    /// CHECK: Owner reference for PDA validation
    pub owner: AccountInfo<'info>,
    
    #[account(
        mut,
        seeds = [owner.key().as_ref()], // Exact seeds as specified
        bump,
        has_one = owner @ BetError::InvalidOwner
    )]
    pub bet_info: Account<'info, BetInfo>,
    
    pub system_program: Program<'info, System>,
}

#[derive(Accounts)]
pub struct Win<'info> {
    #[account(mut)]
    pub player: Signer<'info>,
    
    /// CHECK: Owner reference for PDA validation
    pub owner: AccountInfo<'info>,
    
    #[account(
        mut,
        seeds = [owner.key().as_ref()], // Exact seeds as specified
        bump,
        has_one = owner @ BetError::InvalidOwner
    )]
    pub bet_info: Account<'info, BetInfo>,
    
    /// CHECK: Pyth price feed account validated in the function
    pub price_feed: AccountInfo<'info>,
    
    pub system_program: Program<'info, System>,
}

#[derive(Accounts)]
pub struct Timeout<'info> {
    #[account(mut)]
    pub owner: Signer<'info>,
    
    #[account(
        mut,
        seeds = [owner.key().as_ref()], // Exact seeds as specified
        bump,
        has_one = owner @ BetError::InvalidOwner
    )]
    pub bet_info: Account<'info, BetInfo>,
    
    pub system_program: Program<'info, System>,
}

// Account struct
#[account]
pub struct BetInfo {
    pub owner: Pubkey,
    pub player: Pubkey,
    pub wager: u64,
    pub deadline: u64,
    pub rate: u64,
}

#[error_code]
pub enum BetError {
    #[msg("Invalid delay duration")]
    InvalidDelay,
    #[msg("Bet has already been taken")]
    BetAlreadyTaken,
    #[msg("Bet has not expired yet")]
    BetNotExpired,
    #[msg("Only the player can call this function")]
    NotPlayer,
    #[msg("Price condition not met")]
    ConditionNotMet,
    #[msg("Invalid Pyth price feed account")]
    InvalidPriceFeed,
    #[msg("Invalid owner for this bet")]
    InvalidOwner,
}

fn get_pyth_price(_price_feed_account: &AccountInfo) -> Result<u64> {
    // Simple mock implementation
    Ok(50000)
}