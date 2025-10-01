use anchor_lang::prelude::*;
use anchor_lang::system_program;
use std::str::FromStr;

declare_id!("CJDtsrEsaBrNqnRxNFZCQpQqRLyMfCQ1Wzq5xozqbdGo");

#[program]
pub mod price_bet{
    use super::*;

    pub fn init(ctx: Context<Init>, delay: u64, wager: u64, rate: u64) -> Result<()> {
        let bet_info = &mut ctx.accounts.bet_info;
        let clock = Clock::get()?;
        
        bet_info.owner = ctx.accounts.owner.key();
        bet_info.player = Pubkey::default();
        bet_info.wager = wager;
        bet_info.deadline = clock.unix_timestamp as u64 + delay;
        bet_info.rate = rate;
        
        let cpi_context = CpiContext::new(
            ctx.accounts.system_program.to_account_info(),
            system_program::Transfer {
                from: ctx.accounts.owner.to_account_info(),
                to: bet_info.to_account_info(),
            },
        );
        system_program::transfer(cpi_context, wager)?;
        
        Ok(())
    }

    pub fn join(ctx: Context<Join>) -> Result<()> {
        let bet_info = &mut ctx.accounts.bet_info;
        
        require!(bet_info.player == Pubkey::default(), BetError::AlreadyJoined);
        require!(bet_info.owner == ctx.accounts.owner.key(), BetError::OwnerMismatch);
        
        bet_info.player = ctx.accounts.player.key();
        
        let cpi_context = CpiContext::new(
            ctx.accounts.system_program.to_account_info(),
            system_program::Transfer {
                from: ctx.accounts.player.to_account_info(),
                to: bet_info.to_account_info(),
            },
        );
        system_program::transfer(cpi_context, bet_info.wager)?;
        
        Ok(())
    }

    pub fn win(ctx: Context<Win>) -> Result<()> {
        let bet_info = &mut ctx.accounts.bet_info;
        let clock = Clock::get()?;
        
        require!(clock.unix_timestamp as u64 >= bet_info.deadline, BetError::DeadlineNotReached);
        require!(bet_info.player == ctx.accounts.player.key(), BetError::UnauthorizedPlayer);
        require!(bet_info.owner == ctx.accounts.owner.key(), BetError::OwnerMismatch);
        
        validate_pyth_feed(&ctx.accounts.price_feed)?;
        
        let price_data = parse_pyth_price_data(&ctx.accounts.price_feed)?;
        
        let max_age: u64 = 300;
        let price_age = clock.unix_timestamp as u64 - price_data.publish_time;
        require!(price_age <= max_age, BetError::StalePrice);
        
        require!(price_data.price > 0, BetError::InvalidPrice);
        let current_price_value = price_data.price;
        
        require!(current_price_value >= bet_info.rate, BetError::ConditionNotMet);
        
        let total_pot = bet_info.wager.checked_mul(2).unwrap();
        
        **bet_info.to_account_info().try_borrow_mut_lamports()? = bet_info
            .to_account_info()
            .lamports()
            .checked_sub(total_pot)
            .ok_or(BetError::InsufficientFunds)?;
            
        **ctx.accounts.player.try_borrow_mut_lamports()? = ctx.accounts.player
            .lamports()
            .checked_add(total_pot)
            .ok_or(BetError::TransferFailed)?;
        
        Ok(())
    }

    pub fn timeout(ctx: Context<Timeout>) -> Result<()> {
        let bet_info = &mut ctx.accounts.bet_info;
        let clock = Clock::get()?;
        
        require!(clock.unix_timestamp as u64 >= bet_info.deadline, BetError::DeadlineNotReached);
        require!(bet_info.owner == ctx.accounts.owner.key(), BetError::OwnerMismatch);
        
        let total_funds = bet_info.to_account_info().lamports();
        
        **bet_info.to_account_info().try_borrow_mut_lamports()? = 0;
        **ctx.accounts.owner.try_borrow_mut_lamports()? = ctx.accounts.owner
            .lamports()
            .checked_add(total_funds)
            .ok_or(BetError::TransferFailed)?;
        
        Ok(())
    }
}

fn parse_pyth_price_data(price_feed_account: &AccountInfo) -> Result<PythPriceData> {
    let data = price_feed_account.try_borrow_data()?;
    
    require!(data.len() >= 48, BetError::InvalidPriceFeed);
    
    let price_bytes: [u8; 8] = data[8..16].try_into().map_err(|_| BetError::InvalidPriceFeed)?;
    let price = i64::from_le_bytes(price_bytes);
    
    let time_bytes: [u8; 8] = data[24..32].try_into().map_err(|_| BetError::InvalidPriceFeed)?;
    let publish_time = i64::from_le_bytes(time_bytes) as u64;
    
    Ok(PythPriceData {
        price: price.max(0) as u64,
        publish_time,
    })
}

#[derive(Debug)]
struct PythPriceData {
    price: u64,
    publish_time: u64,
}

#[derive(Accounts)]
pub struct Init<'info> {
    #[account(mut)]
    pub owner: Signer<'info>,
    
    #[account(
        init,
        payer = owner,
        space = 8 + OracleBetInfo::SIZE,
        seeds = [owner.key().as_ref()], // Exactly as specified
        bump
    )]
    pub bet_info: Account<'info, OracleBetInfo>,
    
    pub system_program: Program<'info, System>,
}

#[derive(Accounts)]
pub struct Join<'info> {
    #[account(mut)]
    pub player: Signer<'info>,
    
    /// CHECK: Owner reference as specified
    pub owner: AccountInfo<'info>,
    
    #[account(
        mut,
        seeds = [owner.key().as_ref()], // Exactly as specified
        bump
    )]
    pub bet_info: Account<'info, OracleBetInfo>,
    
    pub system_program: Program<'info, System>,
}

#[derive(Accounts)]
pub struct Win<'info> {
    #[account(mut)]
    pub player: Signer<'info>,
    
    /// CHECK: Owner reference as specified
    pub owner: AccountInfo<'info>,
    
    #[account(
        mut,
        seeds = [owner.key().as_ref()], // Exactly as specified
        bump
    )]
    pub bet_info: Account<'info, OracleBetInfo>,
    
    /// CHECK: Pyth price feed as specified
    pub price_feed: AccountInfo<'info>,
    
    pub system_program: Program<'info, System>,
}

#[derive(Accounts)]
pub struct Timeout<'info> {
    #[account(mut)]
    pub owner: Signer<'info>,
    
    #[account(
        mut,
        seeds = [owner.key().as_ref()], // Exactly as specified
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

impl OracleBetInfo {
    pub const SIZE: usize = 32 + 32 + 8 + 8 + 8;
}

#[error_code]
pub enum BetError {
    #[msg("Deadline not reached")]
    DeadlineNotReached,
    #[msg("Price condition not met")]
    ConditionNotMet,
    #[msg("Invalid Pyth price feed")]
    InvalidPriceFeed,
    #[msg("Price not available")]
    PriceNotAvailable,
    #[msg("Stale price data")]
    StalePrice,
    #[msg("Bet already joined")]
    AlreadyJoined,
    #[msg("Unauthorized player")]
    UnauthorizedPlayer,
    #[msg("Insufficient funds")]
    InsufficientFunds,
    #[msg("Transfer failed")]
    TransferFailed,
    #[msg("Invalid price")]
    InvalidPrice,
    #[msg("Owner mismatch")]
    OwnerMismatch,
}

fn validate_pyth_feed(price_feed_account: &AccountInfo) -> Result<()> {
    let pyth_program_id = Pubkey::from_str("FsJ3A3u2vn5cTVofAjvy6y5kwABJAqYWpe4975bi2epH")
        .or_else(|_| Pubkey::from_str("gSbePebfvPy7tRqimPoVecS2UsBvYv46ynrzWocc92A"))
        .map_err(|_| BetError::InvalidPriceFeed)?;
    
    require!(
        price_feed_account.owner == &pyth_program_id,
        BetError::InvalidPriceFeed
    );
    
    Ok(())
}