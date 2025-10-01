use anchor_lang::prelude::*;
use anchor_lang::system_program;
use pyth_sdk_solana::load_price_feed_from_account_info;
use std::convert::TryInto;

declare_id!("DGqH7rzxnj3iaqPBWFWfmoHSvense9b1XLZ5Xk91fUEu");

const MAX_STALENESS_SECONDS: i64 = 300; // 5 minutes
const BET_INFO_SIZE: usize = 8 + 32 + 32 + 8 + 8 + 8; // discriminator + owner + player + wager + deadline + rate

#[program]
pub mod price_bet {
    use super::*;

    pub fn init(ctx: Context<InitCtx>, delay: u64, wager: u64, rate: u64) -> Result<()> {
        require!(wager > 0, ErrorCode::InvalidWager);
        require!(rate > 0, ErrorCode::InvalidRate);

        let clock = Clock::get()?;
        require!(clock.unix_timestamp >= 0, ErrorCode::NegativeTimestamp);
        let now_u64: u64 = clock.unix_timestamp as u64;

        let deadline = now_u64
            .checked_add(delay)
            .ok_or(error!(ErrorCode::Overflow))?;

        // Transfer the initial wager from owner to the bet PDA before mutably writing into bet account
        let bet_info_ai = ctx.accounts.bet_info.to_account_info();
        let cpi_accounts = anchor_lang::system_program::Transfer {
            from: ctx.accounts.owner.to_account_info(),
            to: bet_info_ai.clone(),
        };
        let cpi_program = ctx.accounts.system_program.to_account_info();
        anchor_lang::system_program::transfer(
            CpiContext::new(cpi_program, cpi_accounts),
            wager,
        )?;

        // Now safely write to the bet account
        let bet = &mut ctx.accounts.bet_info;
        bet.owner = ctx.accounts.owner.key();
        bet.player = Pubkey::default();
        bet.wager = wager;
        bet.deadline = deadline;
        bet.rate = rate;

        Ok(())
    }

    pub fn join(ctx: Context<JoinCtx>) -> Result<()> {
        // Read immutable fields first to avoid borrow conflicts
        let bet_ai = ctx.accounts.bet_info.to_account_info();
        let bet_read = &ctx.accounts.bet_info;

        // Basic checks using immutable borrow
        let clock = Clock::get()?;
        require!(clock.unix_timestamp >= 0, ErrorCode::NegativeTimestamp);
        let now_u64 = clock.unix_timestamp as u64;

        require!(now_u64 <= bet_read.deadline, ErrorCode::DeadlinePassed);
        require!(bet_read.player == Pubkey::default(), ErrorCode::AlreadyJoined);
        require!(ctx.accounts.player.key() != bet_read.owner, ErrorCode::OwnerCannotJoin);
        require!(ctx.accounts.owner.key() == bet_read.owner, ErrorCode::InvalidOwnerReference);

        // Transfer matching wager from player to PDA
        let wager = bet_read.wager;
        let cpi_accounts = anchor_lang::system_program::Transfer {
            from: ctx.accounts.player.to_account_info(),
            to: bet_ai.clone(),
        };
        let cpi_program = ctx.accounts.system_program.to_account_info();
        anchor_lang::system_program::transfer(
            CpiContext::new(cpi_program, cpi_accounts),
            wager,
        )?;

        // Mutably set player
        let bet = &mut ctx.accounts.bet_info;
        bet.player = ctx.accounts.player.key();

        Ok(())
    }

    pub fn win(ctx: Context<WinCtx>) -> Result<()> {
        // Verify player is signer and matches stored player
        let bet = &ctx.accounts.bet_info;
        require!(ctx.accounts.player.key() == bet.player, ErrorCode::NotPlayer);
        require!(ctx.accounts.owner.key() == bet.owner, ErrorCode::InvalidOwnerReference);

        let clock = Clock::get()?;
        require!(clock.unix_timestamp >= 0, ErrorCode::NegativeTimestamp);
        let now = clock.unix_timestamp as i64;
        require!(now as u64 >= bet.deadline, ErrorCode::TooEarlyToResolve);

        // Load and validate Pyth price feed account
        let price_feed = load_price_feed_from_account_info(&ctx.accounts.price_feed)
            .map_err(|_| error!(ErrorCode::InvalidOracleAccount))?;

        // Get price info and validate
        let pyth_price = price_feed.get_price_unchecked();
        let publish_time = pyth_price.publish_time; // i64
        let price_mantissa = pyth_price.price; // i64

        require!(publish_time > 0, ErrorCode::InvalidOracleData);
        require!(price_mantissa != 0, ErrorCode::InvalidOracleData);

        // Staleness check (seconds)
        let age = now
            .checked_sub(publish_time)
            .ok_or(error!(ErrorCode::StalePrice))?;
        require!(age <= MAX_STALENESS_SECONDS, ErrorCode::StalePrice);

        // Compare mantissas directly: both are integer mantissas (owner must store rate in same units)
        let current_mantissa = price_mantissa as i128;
        let target_mantissa = bet.rate as i128;

        if current_mantissa >= target_mantissa {
            // Closing is handled by Anchor because bet_info has `close = player` in WinCtx accounts
            Ok(())
        } else {
            Err(error!(ErrorCode::ConditionNotMet))
        }
    }

    pub fn timeout(ctx: Context<TimeoutCtx>) -> Result<()> {
        // Owner reclaim after deadline
        let bet = &ctx.accounts.bet_info;
        require!(ctx.accounts.owner.key() == bet.owner, ErrorCode::InvalidOwner);

        let clock = Clock::get()?;
        require!(clock.unix_timestamp >= 0, ErrorCode::NegativeTimestamp);
        let now_u64 = clock.unix_timestamp as u64;
        require!(now_u64 >= bet.deadline, ErrorCode::TooEarlyToResolve);

        // Closing handled by Anchor with `close = owner` in account attributes
        Ok(())
    }
}

#[account]
pub struct OracleBetInfo {
    pub owner: Pubkey,
    pub player: Pubkey,
    pub wager: u64,
    pub deadline: u64,
    pub rate: u64,
}

#[derive(Accounts)]
pub struct InitCtx<'info> {
    /// Owner must sign
    #[account(mut)]
    pub owner: Signer<'info>,

    /// Bet PDA seeded exactly as [owner.key().as_ref()]
    #[account(
        init,
        payer = owner,
        space = BET_INFO_SIZE,
        seeds = [owner.key().as_ref()],
        bump
    )]
    pub bet_info: Account<'info, OracleBetInfo>,

    pub system_program: Program<'info, System>,
}

#[derive(Accounts)]
pub struct JoinCtx<'info> {
    /// Player must sign
    #[account(mut)]
    pub player: Signer<'info>,

    /// Owner reference only (used to verify PDA seeds)
    /// CHECK: read-only reference
    pub owner: UncheckedAccount<'info>,

    /// PDA seeded by owner
    #[account(mut, seeds = [owner.key().as_ref()], bump)]
    pub bet_info: Account<'info, OracleBetInfo>,

    pub system_program: Program<'info, System>,
}

#[derive(Accounts)]
pub struct WinCtx<'info> {
    /// Player must sign
    #[account(mut)]
    pub player: Signer<'info>,

    /// Owner reference only
    /// CHECK: read-only reference
    pub owner: UncheckedAccount<'info>,

    /// PDA seeded by owner; closed to player on success so entire pot is sent
    #[account(mut, seeds = [owner.key().as_ref()], bump, close = player)]
    pub bet_info: Account<'info, OracleBetInfo>,

    /// Pyth price feed account (AccountInfo because we use pyth SDK loader)
    /// CHECK: validated at runtime via pyth_sdk_solana load_price_feed_from_account_info
    pub price_feed: AccountInfo<'info>,

    pub system_program: Program<'info, System>,
}

#[derive(Accounts)]
pub struct TimeoutCtx<'info> {
    /// Owner must sign
    #[account(mut)]
    pub owner: Signer<'info>,

    /// PDA seeded by owner; closed to owner on timeout so entire pot is returned to owner
    #[account(mut, seeds = [owner.key().as_ref()], bump, close = owner)]
    pub bet_info: Account<'info, OracleBetInfo>,

    pub system_program: Program<'info, System>,
}

#[error_code]
pub enum ErrorCode {
    #[msg("Wager must be greater than zero")]
    InvalidWager,
    #[msg("Rate must be greater than zero")]
    InvalidRate,
    #[msg("Deadline has already passed")]
    DeadlinePassed,
    #[msg("Bet already has a player")]
    AlreadyJoined,
    #[msg("Owner cannot join their own bet")]
    OwnerCannotJoin,
    #[msg("Not the player for this bet")]
    NotPlayer,
    #[msg("Too early to resolve the bet")]
    TooEarlyToResolve,
    #[msg("Pyth oracle account is invalid or not a Pyth price account")]
    InvalidOracleAccount,
    #[msg("Oracle price is stale")]
    StalePrice,
    #[msg("Price condition not met")]
    ConditionNotMet,
    #[msg("Owner mismatch")]
    InvalidOwner,
    #[msg("Integer overflow")]
    Overflow,
    #[msg("Clock unix_timestamp negative")]
    NegativeTimestamp,
    #[msg("Invalid oracle data")]
    InvalidOracleData,
    #[msg("Owner reference does not match bet owner")]
    InvalidOwnerReference,
}
