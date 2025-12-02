use anchor_lang::prelude::*;
use anchor_lang::system_program;
use pyth_sdk_solana::load_price_feed_from_account_info;
use std::str::FromStr;

declare_id!("3sG19eiLedgp2pxAyTyimSt1bMTboiPrJ6SiGMA9Qw1N");

// Staleness threshold in seconds for Pyth prices
const STALENESS_THRESHOLD_SECONDS: u64 = 60;

// -- NOTE: This program validates the Pyth oracle account owner against the well-known
// mainnet Pyth program ID. If you deploy to devnet/testnet or use a different Pyth program,
// replace this string with the correct program id for that cluster.
const PYTH_MAINNET_PROGRAM_ID_STR: &str = "3sG19eiLedgp2pxAyTyimSt1bMTboiPrJ6SiGMA9Qw1N";

#[program]
pub mod price_bet {
    use super::*;

    /// init: called by owner (signer)
    /// - delay: number of seconds from now until deadline
    /// - wager: lamports the owner deposits as the initial pot
    /// - rate: target rate (IN PYTH RAW PRICE UNITS — see notes)
    pub fn init(ctx: Context<InitCtx>, delay: u64, wager: u64, rate: u64) -> Result<()> {
        let clock = Clock::get()?;
        let now = clock.unix_timestamp as u64;
        let deadline = now.checked_add(delay).ok_or(ErrorCode::ArithmeticOverflow)?;

        // Fill struct fields
        {
            let bet = &mut ctx.accounts.bet_info;
            bet.owner = ctx.accounts.owner.key();
            bet.player = Pubkey::default();
            bet.wager = wager;
            bet.deadline = deadline;
            bet.rate = rate;
        }

        // Transfer after dropping the mutable borrow
        let cpi_accounts = anchor_lang::system_program::Transfer {
            from: ctx.accounts.owner.to_account_info(),
            to: ctx.accounts.bet_info.to_account_info(),
        };
        let cpi_ctx = CpiContext::new(ctx.accounts.system_program.to_account_info(), cpi_accounts);
        system_program::transfer(cpi_ctx, wager)?;

        msg!(
            "Init: owner {} created bet PDA {} with wager {} lamports, deadline {} (unix)",
            ctx.accounts.owner.key(),
            ctx.accounts.bet_info.key(),
            wager,
            deadline
        );

        Ok(())
    }

    /// join: called by the player (signer)
    /// - player transfers a matching wager to the bet PDA
    pub fn join(ctx: Context<JoinCtx>) -> Result<()> {
        let clock = Clock::get()?;
        let now = clock.unix_timestamp as u64;

        // Immutable borrow for checks
        {
            let bet = &ctx.accounts.bet_info;
            require!(bet.player == Pubkey::default(), ErrorCode::AlreadyHasPlayer);
            require!(ctx.accounts.player.key() != bet.owner, ErrorCode::PlayerIsOwner);
            require!(now < bet.deadline, ErrorCode::BetAlreadyExpired);
        }

        let wager = ctx.accounts.bet_info.wager;

        // Transfer player's matching wager
        let cpi_accounts = anchor_lang::system_program::Transfer {
            from: ctx.accounts.player.to_account_info(),
            to: ctx.accounts.bet_info.to_account_info(),
        };
        let cpi_ctx = CpiContext::new(ctx.accounts.system_program.to_account_info(), cpi_accounts);
        system_program::transfer(cpi_ctx, wager)?;

        // Now mutate the account
        ctx.accounts.bet_info.player = ctx.accounts.player.key();

        msg!(
            "Join: player {} joined bet {} by owner {} (wager {}).",
            ctx.accounts.player.key(),
            ctx.accounts.bet_info.key(),
            ctx.accounts.owner.key(),
            wager
        );

        Ok(())
    }

    /// win: called by the player (signer) after the deadline (player claims if price condition met)
    /// * validates Pyth feed account owner and staleness (uses `load_price_feed_from_account_info`).
    /// * On success, Anchor `close = player` for bet_info ensures all lamports are sent to player and the PDA is closed.
    #[access_control(player_is_signer(&ctx.accounts.player))]
    pub fn win(ctx: Context<WinCtx>) -> Result<()> {
        let bet = &ctx.accounts.bet_info;

        // only the recorded player can call win
        require!(bet.player == ctx.accounts.player.key(), ErrorCode::NotPlayer);

        // ensure bet expired (deadline reached)
        let clock = Clock::get()?;
        let now = clock.unix_timestamp as u64;
        require!(now >= bet.deadline, ErrorCode::BetNotExpired);

        // Validate the price feed account owner (basic oracle ownership check)
        let pyth_prog_pubkey = Pubkey::from_str(PYTH_MAINNET_PROGRAM_ID_STR)
            .map_err(|_| error!(ErrorCode::InvalidOracleAccount))?;
        require!(
            ctx.accounts.price_feed.owner == &pyth_prog_pubkey,
            ErrorCode::InvalidOracleAccount
        );

        // Load the PriceFeed and get a recent price (staleness checked)
        let price_feed = load_price_feed_from_account_info(&ctx.accounts.price_feed)
            .map_err(|_| error!(ErrorCode::InvalidOracleAccount))?;

        let current_price = price_feed
            .get_price_no_older_than(clock.unix_timestamp, STALENESS_THRESHOLD_SECONDS)
            .ok_or_else(|| error!(ErrorCode::PriceStale))?;

        // Note: Pyth's Price.price is an i64; we compare against the stored u64 `bet.rate`.
        // The `rate` must be provided in the same raw price units as Pyth's Price.price.
        let pyth_price_i128 = current_price.price as i128;
        let bet_rate_i128 = bet.rate as i128;

        require!(
            pyth_price_i128 >= bet_rate_i128,
            ErrorCode::PriceConditionNotMet
        );

        // If we reach here, the player wins. The account attribute `close = player` on bet_info
        // will cause Anchor runtime to transfer all lamports from the PDA to `player` and deallocate the PDA.
        msg!("Win: player {} won bet {}. pyth_price = {} expo = {}", 
             ctx.accounts.player.key(), ctx.accounts.bet_info.key(), current_price.price, current_price.expo);
        Ok(())
    }

    /// timeout: called by owner (signer) after deadline to reclaim pot if player didn't win
    /// * Anchor `close = owner` will send all lamports to owner and deallocate the PDA.
    #[access_control(owner_is_signer(&ctx.accounts.owner))]
    pub fn timeout(ctx: Context<TimeoutCtx>) -> Result<()> {
        let bet = &ctx.accounts.bet_info;

        // only the owner who created the bet can call timeout
        require!(ctx.accounts.owner.key() == bet.owner, ErrorCode::NotOwner);

        let clock = Clock::get()?;
        let now = clock.unix_timestamp as u64;
        require!(now >= bet.deadline, ErrorCode::BetNotExpired);

        msg!("Timeout: owner {} reclaimed bet {} after deadline {}", 
             ctx.accounts.owner.key(), ctx.accounts.bet_info.key(), bet.deadline);
        // Anchor will close the account and send lamports to owner (since `close = owner` on account).
        Ok(())
    }
}

// ---------------------------- Contexts ----------------------------

#[derive(Accounts)]
#[instruction(delay: u64, wager: u64, rate: u64)]
pub struct InitCtx<'info> {
    /// Owner must be signer
    #[account(mut)]
    pub owner: Signer<'info>,

    /// PDA created using seeds = [owner.key().as_ref()]
    #[account(
        init,
        payer = owner,
        space = OracleBetInfo::LEN,
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

    /// Owner is only a reference here (not a signer)
    /// CHECK: validated by PDA seeds on bet_info
    pub owner: UncheckedAccount<'info>,

    /// PDA must match `seeds = [owner.key().as_ref()]`
    #[account(mut, seeds = [owner.key().as_ref()], bump)]
    pub bet_info: Account<'info, OracleBetInfo>,

    pub system_program: Program<'info, System>,
}

#[derive(Accounts)]
pub struct WinCtx<'info> {
    /// Player signs
    #[account(mut)]
    pub player: Signer<'info>,

    /// Owner reference (for PDA seeds)
    /// CHECK: used as seed only
    pub owner: UncheckedAccount<'info>,

    /// PDA; when the player wins we close this account and send lamports to the player
    #[account(
        mut,
        seeds = [owner.key().as_ref()],
        bump,
        close = player
    )]
    pub bet_info: Account<'info, OracleBetInfo>,

    /// Pyth price feed account (raw AccountInfo)
    /// CHECK: validated at runtime (owner + parsing)
    pub price_feed: AccountInfo<'info>,

    pub system_program: Program<'info, System>,
}

#[derive(Accounts)]
pub struct TimeoutCtx<'info> {
    /// Owner must sign
    #[account(mut)]
    pub owner: Signer<'info>,

    /// PDA; owner reclaims pot, account is closed and lamports returned to owner
    #[account(
        mut,
        seeds = [owner.key().as_ref()],
        bump,
        close = owner
    )]
    pub bet_info: Account<'info, OracleBetInfo>,

    pub system_program: Program<'info, System>,
}

// ---------------------------- Account data ----------------------------

#[account]
pub struct OracleBetInfo {
    pub owner: Pubkey,
    pub player: Pubkey,
    pub wager: u64,
    pub deadline: u64,
    pub rate: u64,
}

impl OracleBetInfo {
    // space calculation: 8 discriminator + sizes of fields
    // Pubkey (32) + Pubkey (32) + u64 (8) + u64 (8) + u64 (8) = 88, plus 8 = 96
    pub const LEN: usize = 8 + 32 + 32 + 8 + 8 + 8;
}

// ---------------------------- Errors & Guards ----------------------------

fn owner_is_signer(owner: &Signer) -> Result<()> {
    // the anchor account type already ensures signer, this is for explicit access control macro usage
    Ok(())
}

fn player_is_signer(player: &Signer) -> Result<()> {
    Ok(())
}

#[error_code]
pub enum ErrorCode {
    #[msg("Arithmetic overflow when computing deadline")]
    ArithmeticOverflow,
    #[msg("A player has already joined this bet")]
    AlreadyHasPlayer,
    #[msg("The player cannot be the owner")]
    PlayerIsOwner,
    #[msg("The bet has already expired")]
    BetAlreadyExpired,
    #[msg("Only the recorded player can call this")]
    NotPlayer,
    #[msg("Only the owner can call this")]
    NotOwner,
    #[msg("Bet not expired yet")]
    BetNotExpired,
    #[msg("Oracle account is invalid or not a Pyth price account")]
    InvalidOracleAccount,
    #[msg("Price data is stale")]
    PriceStale,
    #[msg("Price condition not met")]
    PriceConditionNotMet,
    #[msg("Attempt to transfer more lamports than available or arithmetic overflow")]
    InsufficientFundsOrOverflow,
    #[msg("Generic arithmetic overflow")]
    ArithmeticOverflowGeneric,
}
