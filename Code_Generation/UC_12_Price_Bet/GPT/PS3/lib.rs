use anchor_lang::prelude::*;
use anchor_lang::system_program;
use pyth_sdk_solana::load_price_feed_from_account_info;
use pyth_sdk_solana::state::Price; // ✅ correct
use std::str::FromStr;

declare_id!("gqssVNBhrpHNrsvDXWTrVvE9FRdUvpgUi8NjqRyyymL");

/// Staleness threshold in seconds for Pyth price reads (tunable)
const STALENESS_SECONDS: u64 = 60;

#[program]
pub mod price_bet {
    use super::*;

    /// Owner initializes the bet PDA and deposits the initial wager.
    /// - delay: seconds until deadline from now
    /// - wager: lamports that owner deposits as initial pot
    /// - rate: integer target (e.g. 50000 for $50k)
    pub fn init(ctx: Context<InitCtx>, delay: u64, wager: u64, rate: u64) -> Result<()> {
        let now = Clock::get()?.unix_timestamp as u64;

        // Initialize state
        let bet = &mut ctx.accounts.bet_info;
        bet.owner = ctx.accounts.owner.key();
        bet.player = Pubkey::default();
        bet.wager = wager;
        bet.deadline = now.checked_add(delay).ok_or(ErrorCode::Overflow)?;
        bet.rate = rate;

        // Transfer the `wager` lamports from owner -> bet PDA (in addition to rent paid by init)
        let cpi_ctx = CpiContext::new(
            ctx.accounts.system_program.to_account_info(),
            system_program::Transfer {
                from: ctx.accounts.owner.to_account_info(),
                to: ctx.accounts.bet_info.to_account_info(),
            },
        );
        system_program::transfer(cpi_ctx, wager)?;

        Ok(())
    }

    /// Player joins a bet by sending exactly the matching wager to the bet PDA.
    /// Accounts: player (signer), owner (reference), bet_info (PDA).
    pub fn join(ctx: Context<JoinCtx>) -> Result<()> {
    let now = Clock::get()?.unix_timestamp as u64;

    // Borrow immutably first
    if ctx.accounts.bet_info.player != Pubkey::default() {
        return err!(ErrorCode::AlreadyJoined);
    }
    if now > ctx.accounts.bet_info.deadline {
        return err!(ErrorCode::BetExpired);
    }

    // Save wager amount
    let wager = ctx.accounts.bet_info.wager;

    // CPI transfer
    let cpi_ctx = CpiContext::new(
        ctx.accounts.system_program.to_account_info(),
        system_program::Transfer {
            from: ctx.accounts.player.to_account_info(),
            to: ctx.accounts.bet_info.to_account_info(),
        },
    );
    system_program::transfer(cpi_ctx, wager)?;

    // Now mutate bet_info safely
    ctx.accounts.bet_info.player = ctx.accounts.player.key();

    Ok(())
}


    /// Called by the player to claim the pot if Pyth's current price meets the condition.
    /// This will close the bet PDA and transfer the entire pot to the player (because of `close = player` in the Accounts struct).
    ///
    /// Accounts: player (signer), owner (reference), bet_info (PDA), price_feed (Pyth account), system_program.
    pub fn win(ctx: Context<WinCtx>) -> Result<()> {
        let bet = &ctx.accounts.bet_info;

        // Ensure caller is the registered player (has_one player enforces this at Accounts level)
        // Ensure we are before the deadline
        let now = Clock::get()?.unix_timestamp;
        if (now as u64) > bet.deadline {
            return err!(ErrorCode::BetExpired);
        }

        // Parse price feed (this both validates the account data layout and returns a PriceFeed)
        // If parsing fails we map to InvalidOracle
        let feed = load_price_feed_from_account_info(&ctx.accounts.price_feed)
            .map_err(|_| error!(ErrorCode::InvalidOracle))?;

        // Get a recent price; this will return None if the on-chain price is too old
        let pyth_price: Price = feed
            .get_price_no_older_than(now, STALENESS_SECONDS)
            .ok_or(error!(ErrorCode::StalePrice))?;

        // Basic sanity checks on the pyth price
        if pyth_price.price <= 0 {
            return err!(ErrorCode::InvalidOracle);
        }

        // We expect typical Pyth price exponents for USD pairs to be <= 0 (e.g., expo = -8).
        // Convert bet.rate (an integer like 50000) into the same fixed-point integer representation
        // used by Pyth's `price` (i.e., price * 10^expo).
        //
        // If expo is negative: bet_scaled = rate * 10^(-expo)
        // If expo is positive (very unusual for USD quotes) we avoid introducing fractional comparisons
        // by requiring expo <= 0; if expo > 0 we fail with InvalidOracle (safer).
        let expo = pyth_price.expo;
        if expo > 0 {
            return err!(ErrorCode::InvalidOracle);
        }

        let bet_rate_scaled: i128 = {
            // safe to cast: bet.rate as i128; expo.abs() as u32 small
            let scale = 10i128.pow((-expo) as u32);
            (bet.rate as i128)
                .checked_mul(scale)
                .ok_or(error!(ErrorCode::Overflow))?
        };

        let pyth_raw: i128 = pyth_price.price as i128;

        // Winning condition: Pyth price > bet rate (scaled to same units)
        if pyth_raw > bet_rate_scaled {
            // success — Anchor will automatically close the bet_info PDA and send all lamports to player
            // because the Accounts definition has `close = player`.
            Ok(())
        } else {
            // Price condition not met
            err!(ErrorCode::NoWin)
        }
    }

    /// Called by the owner after the deadline to reclaim the pot (if nobody won).
    /// This will close the bet PDA and transfer everything back to the owner (because of `close = owner`).
    pub fn timeout(ctx: Context<TimeoutCtx>) -> Result<()> {
        let bet = &ctx.accounts.bet_info;
        let now = Clock::get()?.unix_timestamp as u64;

        // Owner signature enforced by Accounts (owner: Signer)
        // Ensure deadline passed
        if now < bet.deadline {
            return err!(ErrorCode::BetNotExpired);
        }

        // Anchor will close the account (close = owner) and transfer all lamports to owner.
        Ok(())
    }
}

/// Persistent state stored in the PDA (exact layout per your spec)
#[account]
pub struct OracleBetInfo {
    pub owner: Pubkey,
    pub player: Pubkey,
    pub wager: u64,
    pub deadline: u64,
    pub rate: u64,
}

/// Init accounts
#[derive(Accounts)]
pub struct InitCtx<'info> {
    /// Owner creates the bet (must sign)
    #[account(mut)]
    pub owner: Signer<'info>,

    /// The bet PDA: created here with seed `[owner.key().as_ref()]`
    /// NOTE: space = 8 (discriminator) + 32 + 32 + 8 + 8 + 8 = 96 bytes
    #[account(
        init,
        payer = owner,
        space = 8 + 32 + 32 + 8 + 8 + 8,
        seeds = [owner.key().as_ref()],
        bump
    )]
    pub bet_info: Account<'info, OracleBetInfo>,

    pub system_program: Program<'info, System>,
}

/// Join accounts
#[derive(Accounts)]
pub struct JoinCtx<'info> {
    /// Player joins (must sign)
    #[account(mut)]
    pub player: Signer<'info>,

    /// Owner as a reference (not signer here). We assert the PDA seeds and has_one on bet_info below.
    /// CHECK: validated by seeds and has_one constraint on bet_info
    pub owner: UncheckedAccount<'info>,

    /// The existing bet PDA (seeds must be derived from `owner`)
    #[account(
        mut,
        seeds = [owner.key().as_ref()],
        bump,
        has_one = owner
    )]
    pub bet_info: Account<'info, OracleBetInfo>,

    pub system_program: Program<'info, System>,
}

/// Win accounts
#[derive(Accounts)]
pub struct WinCtx<'info> {
    /// Player calls win (must sign)
    #[account(mut)]
    pub player: Signer<'info>,

    /// Owner as a reference (not a signer here)
    /// CHECK: validated by PDA seeds constraint on bet_info
    pub owner: UncheckedAccount<'info>,

    /// bet_info PDA must:
    ///  - be derived from owner (seeds)
    ///  - have `player` equal to the signer (has_one = player)
    ///  - be closed to the player when this instruction finishes (close = player)
    #[account(
        mut,
        seeds = [owner.key().as_ref()],
        bump,
        has_one = player,
        close = player
    )]
    pub bet_info: Account<'info, OracleBetInfo>,

    /// Pyth price feed account (raw AccountInfo).
    /// We intentionally accept it as AccountInfo and parse it with Pyth SDK.
    /// CHECK: validated by `load_price_feed_from_account_info` in handler.
    pub price_feed: AccountInfo<'info>,

    pub system_program: Program<'info, System>,
}

/// Timeout accounts
#[derive(Accounts)]
pub struct TimeoutCtx<'info> {
    /// Owner reclaims (must sign)
    #[account(mut)]
    pub owner: Signer<'info>,

    /// bet_info PDA must:
    ///  - be derived from owner (seeds)
    ///  - be closed to the owner when this instruction finishes (close = owner)
    #[account(
        mut,
        seeds = [owner.key().as_ref()],
        bump,
        has_one = owner,
        close = owner
    )]
    pub bet_info: Account<'info, OracleBetInfo>,

    pub system_program: Program<'info, System>,
}

/// Application errors (clear messages)
#[error_code]
pub enum ErrorCode {
    #[msg("The joiner is already registered for this bet")]
    AlreadyJoined,
    #[msg("Bet has already expired")]
    BetExpired,
    #[msg("Bet has not yet expired")]
    BetNotExpired,
    #[msg("Price did not meet the required rate — no win")]
    NoWin,
    #[msg("Oracle feed is invalid or not recognized")]
    InvalidOracle,
    #[msg("Price feed is stale (too old)")]
    StalePrice,
    #[msg("Integer overflow")]
    Overflow,
}

