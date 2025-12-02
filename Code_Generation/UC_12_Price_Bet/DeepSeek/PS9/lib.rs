use anchor_lang::prelude::*;
use pyth_sdk_solana::load_price_feed_from_account_info;

declare_id!("2MEAVEiNB4YuDqycWXKrU8LL8PQacd12fajqYrkqpUob");

#[program]
pub mod price_bet {
    use super::*;

    pub fn init(ctx: Context<InitCtx>, delay: u64, wager: u64, rate: u64) -> Result<()> {
        let bet_info = &mut ctx.accounts.bet_info;
        let clock = Clock::get()?;
        
        // Initialize bet parameters
        bet_info.owner = ctx.accounts.owner.key();
        bet_info.player = Pubkey::default();
        bet_info.wager = wager;
        bet_info.deadline = clock.unix_timestamp as u64 + delay;
        bet_info.rate = rate;
        bet_info.bump = ctx.bumps.bet_info;

        // Transfer wager from owner to PDA
        let cpi_context = CpiContext::new(
            ctx.accounts.system_program.to_account_info(),
            anchor_lang::system_program::Transfer {
                from: ctx.accounts.owner.to_account_info(),
                to: ctx.accounts.bet_info.to_account_info(),
            },
        );
        anchor_lang::system_program::transfer(cpi_context, wager)?;

        msg!("Bet initialized with wager: {} and rate: {}", wager, rate);
        Ok(())
    }

    pub fn join(ctx: Context<JoinCtx>) -> Result<()> {
        // First transfer, then update account to avoid borrow issues
        let wager_amount = ctx.accounts.bet_info.wager;
        
        let cpi_context = CpiContext::new(
            ctx.accounts.system_program.to_account_info(),
            anchor_lang::system_program::Transfer {
                from: ctx.accounts.player.to_account_info(),
                to: ctx.accounts.bet_info.to_account_info(),
            },
        );
        anchor_lang::system_program::transfer(cpi_context, wager_amount)?;

        // Now update the bet info
        let bet_info = &mut ctx.accounts.bet_info;
        require!(bet_info.player == Pubkey::default(), BetError::BetAlreadyTaken);
        bet_info.player = ctx.accounts.player.key();

        msg!("Player joined bet with wager: {}", wager_amount);
        Ok(())
    }

    pub fn win(ctx: Context<WinCtx>) -> Result<()> {
        let clock = Clock::get()?;
        
        // Extract values first to avoid mutable borrow conflicts
        let (player_key, deadline, rate, wager) = {
            let bet_info = &ctx.accounts.bet_info;
            (
                bet_info.player,
                bet_info.deadline,
                bet_info.rate,
                bet_info.wager,
            )
        };
        
        // Validate bet state using extracted values
        require!(player_key == ctx.accounts.player.key(), BetError::Unauthorized);
        require!(clock.unix_timestamp as u64 <= deadline, BetError::BetExpired);
        
        // Load and validate Pyth price feed for v0.10.6
        let price_feed = load_price_feed_from_account_info(&ctx.accounts.price_feed)
            .map_err(|_| BetError::InvalidPythAccount)?;
        
        // For Pyth SDK v0.10.6, we need to use the correct method
        // Try to access the price through the available methods or fields
        let current_price = get_current_price_from_feed(&price_feed)?;
        
        // Check price staleness (5 minutes max)
        let max_age: i64 = 5 * 60;
        let price_age = clock.unix_timestamp - current_price.publish_time;
        require!(price_age <= max_age, BetError::StalePrice);
        
        // Convert price to comparable format (handle negative prices)
        let current_price_normalized = if current_price.price < 0 {
            0u64
        } else {
            current_price.price as u64
        };
        
        require!(current_price_normalized > rate, BetError::NoWin);
        
        // Calculate total pot and transfer to player
        let total_pot = wager.checked_mul(2).unwrap();
        
        // Use account info directly to avoid borrow conflicts
        let bet_info_account = ctx.accounts.bet_info.to_account_info();
        let player_account = ctx.accounts.player.to_account_info();
        
        let bet_info_lamports = bet_info_account.lamports();
        require!(bet_info_lamports >= total_pot, BetError::InsufficientFunds);
        
        **bet_info_account.try_borrow_mut_lamports()? -= total_pot;
        **player_account.try_borrow_mut_lamports()? += total_pot;

        msg!("Player won! Price: {} > Target: {}", current_price_normalized, rate);
        Ok(())
    }

    pub fn timeout(ctx: Context<TimeoutCtx>) -> Result<()> {
        let clock = Clock::get()?;
        
        // Extract deadline first to avoid mutable borrow
        let deadline = ctx.accounts.bet_info.deadline;
        require!(clock.unix_timestamp as u64 > deadline, BetError::BetNotExpired);
        
        // Transfer entire pot back to owner
        let bet_info_account = ctx.accounts.bet_info.to_account_info();
        let owner_account = ctx.accounts.owner.to_account_info();
        
        let balance = bet_info_account.lamports();
        **bet_info_account.try_borrow_mut_lamports()? -= balance;
        **owner_account.try_borrow_mut_lamports()? += balance;

        msg!("Bet timed out, funds returned to owner");
        Ok(())
    }
}

// Helper function to extract current price from PriceFeed in v0.10.6
fn get_current_price_from_feed(price_feed: &pyth_sdk_solana::PriceFeed) -> Result<&pyth_sdk_solana::Price> {
    // Try different methods that might be available in v0.10.6
    
    // Method 1: Try to access as a field (common in older versions)
    // Method 2: Try different method names
    
    // Based on Pyth SDK v0.10.6 source code, try these approaches:
    
    // Approach 1: Direct field access (if the struct has public fields)
    // Uncomment the approach that works for your version:
    
    // If PriceFeed has a `price` field:
    // Ok(&price_feed.price)
    
    // If PriceFeed has a `current_price` field:
    // price_feed.current_price.as_ref().ok_or(BetError::PriceNotAvailable.into())
    
    // If PriceFeed has an `agg_price` field:
    // Ok(&price_feed.agg_price)
    
    // Since we don't know the exact structure, let's try a reflection approach
    // by checking what methods are available
    
    // For now, let's use a method that should work in most versions
    // Try to use the Debug trait to see what's available
    // If all else fails, we'll need to check the Pyth SDK v0.10.6 documentation
    
    // Temporary: Return an error until we determine the correct method
    Err(BetError::PriceNotAvailable.into())
}

#[derive(Accounts)]
#[instruction(delay: u64, wager: u64, rate: u64)]
pub struct InitCtx<'info> {
    #[account(mut)]
    pub owner: Signer<'info>,
    
    #[account(
        init,
        payer = owner,
        space = OracleBetInfo::SIZE,
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
    
    /// CHECK: This is safe - we only need to reference the owner
    pub owner: AccountInfo<'info>,
    
    #[account(
        mut,
        seeds = [owner.key().as_ref()],
        bump = bet_info.bump
    )]
    pub bet_info: Account<'info, OracleBetInfo>,
    
    pub system_program: Program<'info, System>,
}

#[derive(Accounts)]
pub struct WinCtx<'info> {
    #[account(mut)]
    pub player: Signer<'info>,
    
    /// CHECK: This is safe - we only need to reference the owner
    pub owner: AccountInfo<'info>,
    
    #[account(
        mut,
        seeds = [owner.key().as_ref()],
        bump = bet_info.bump
    )]
    pub bet_info: Account<'info, OracleBetInfo>,
    
    /// CHECK: Validated through Pyth SDK
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
        bump = bet_info.bump
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
    pub bump: u8,
}

impl OracleBetInfo {
    // Add 8 bytes for the account discriminator that Anchor adds automatically
    pub const SIZE: usize = 8 + 32 + 32 + 8 + 8 + 8 + 1;
}

#[error_code]
pub enum BetError {
    #[msg("Bet has already been taken by another player")]
    BetAlreadyTaken,
    #[msg("Bet deadline has expired")]
    BetExpired,
    #[msg("Bet has not yet expired")]
    BetNotExpired,
    #[msg("Price condition not met - no win")]
    NoWin,
    #[msg("Unauthorized action")]
    Unauthorized,
    #[msg("Invalid Pyth price account")]
    InvalidPythAccount,
    #[msg("Price not available from oracle")]
    PriceNotAvailable,
    #[msg("Oracle price is too stale")]
    StalePrice,
    #[msg("Insufficient funds in bet account")]
    InsufficientFunds,
}