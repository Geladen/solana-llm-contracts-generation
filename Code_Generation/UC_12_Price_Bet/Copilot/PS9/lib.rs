use anchor_lang::prelude::*;
use anchor_lang::solana_program::{program::invoke, system_instruction};
use pyth_sdk_solana::load_price_feed_from_account_info;

declare_id!("14Cn1c92JgT9P8oV3mVunmpM9QeJKjG2PQxRuY3yCgL6");

const STALE_PRICE_SECS: i64 = 300; // 5 minutes

#[program]
pub mod price_bet {
    use super::*;

    pub fn init(ctx: Context<InitCtx>, delay: u64, wager: u64, rate: u64) -> Result<()> {
        let owner = &ctx.accounts.owner;
        let bet = &mut ctx.accounts.bet_info;

        let now = Clock::get()?.unix_timestamp as u64;
        bet.owner = owner.key();
        bet.player = Pubkey::default();
        bet.wager = wager;
        bet.deadline = now
            .checked_add(delay)
            .ok_or(error!(ErrorCode::NumericOverflow))?;
        bet.rate = rate;

        // Owner transfers wager into the PDA account (owner -> PDA)
        let ix = system_instruction::transfer(&owner.key(), &bet.to_account_info().key(), wager);
        invoke(
            &ix,
            &[
                owner.to_account_info(),
                bet.to_account_info(),
                ctx.accounts.system_program.to_account_info(),
            ],
        )?;

        Ok(())
    }

    pub fn join(ctx: Context<JoinCtx>) -> Result<()> {
        let player = &ctx.accounts.player;
        let owner_ref = &ctx.accounts.owner;
        let bet = &mut ctx.accounts.bet_info;

        require_keys_eq!(bet.owner, owner_ref.key(), ErrorCode::Unauthorized);
        require!(bet.player == Pubkey::default(), ErrorCode::AlreadyJoined);

        let now = Clock::get()?.unix_timestamp as u64;
        require!(now <= bet.deadline, ErrorCode::DeadlinePassed);

        // Player transfers matching wager to the PDA (player -> PDA)
        let ix = system_instruction::transfer(&player.key(), &bet.to_account_info().key(), bet.wager);
        invoke(
            &ix,
            &[
                player.to_account_info(),
                bet.to_account_info(),
                ctx.accounts.system_program.to_account_info(),
            ],
        )?;

        bet.player = player.key();

        Ok(())
    }

    pub fn win(ctx: Context<WinCtx>) -> Result<()> {
        let player = &ctx.accounts.player;
        let owner_ref = &ctx.accounts.owner;
        let bet = &mut ctx.accounts.bet_info;

        require_keys_eq!(bet.owner, owner_ref.key(), ErrorCode::Unauthorized);
        require!(bet.player == player.key(), ErrorCode::NotPlayer);

        let now_i64 = Clock::get()?.unix_timestamp;
        let now_u64 = now_i64 as u64;
        require!(now_u64 <= bet.deadline, ErrorCode::DeadlinePassed);

        // Load Pyth feed and ensure freshness
        let price_feed = load_price_feed_from_account_info(&ctx.accounts.price_feed.to_account_info())
            .map_err(|_| error!(ErrorCode::OracleInvalid))?;

        let agg_price = price_feed
            .get_price_no_older_than(STALE_PRICE_SECS, now_u64)
            .ok_or(error!(ErrorCode::StalePrice))?;

        // Compose floating price and check condition
        let oracle_price = (agg_price.price as f64) * 10f64.powi(agg_price.expo);
        if oracle_price <= bet.rate as f64 {
            return Err(error!(ErrorCode::NoWin));
        }

        // Validate PDA address matches derived PDA
        let (expected_pda, _bump) = Pubkey::find_program_address(&[owner_ref.key.as_ref()], ctx.program_id);
        require_keys_eq!(expected_pda, bet.to_account_info().key(), ErrorCode::Unauthorized);

        // Move entire PDA lamports to player using direct lamport bookkeeping (PDA -> player)
        let pot = **bet.to_account_info().lamports.borrow();
        require!(pot > 0, ErrorCode::InsufficientFunds);

        transfer_lamports(&bet.to_account_info(), &player.to_account_info(), pot)?;

        Ok(())
    }

    pub fn timeout(ctx: Context<TimeoutCtx>) -> Result<()> {
        let owner = &ctx.accounts.owner;
        let bet = &mut ctx.accounts.bet_info;

        require_keys_eq!(bet.owner, owner.key(), ErrorCode::Unauthorized);

        let now = Clock::get()?.unix_timestamp as u64;
        require!(now > bet.deadline, ErrorCode::DeadlineNotReached);

        // Validate PDA address matches derived PDA
        let (expected_pda, _bump) = Pubkey::find_program_address(&[owner.key.as_ref()], ctx.program_id);
        require_keys_eq!(expected_pda, bet.to_account_info().key(), ErrorCode::Unauthorized);

        // Move entire PDA lamports back to owner using direct lamport bookkeeping (PDA -> owner)
        let pot = **bet.to_account_info().lamports.borrow();
        require!(pot > 0, ErrorCode::InsufficientFunds);

        transfer_lamports(&bet.to_account_info(), &owner.to_account_info(), pot)?;

        Ok(())
    }
}

// Helper to move lamports between accounts without invoking system transfer from an account that carries data.
fn transfer_lamports(from: &AccountInfo, to: &AccountInfo, amount: u64) -> Result<()> {
    // Subtract from 'from'
    {
        let mut from_lamports = from.try_borrow_mut_lamports()?;
        if **from_lamports < amount {
            return Err(error!(ErrorCode::InsufficientFunds));
        }
        **from_lamports = (**from_lamports).checked_sub(amount).ok_or(error!(ErrorCode::NumericOverflow))?;
    }
    // Add to 'to'
    {
        let mut to_lamports = to.try_borrow_mut_lamports()?;
        **to_lamports = (**to_lamports).checked_add(amount).ok_or(error!(ErrorCode::NumericOverflow))?;
    }
    Ok(())
}

#[derive(Accounts)]
pub struct InitCtx<'info> {
    #[account(mut, signer)]
    pub owner: Signer<'info>,

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

#[derive(Accounts)]
pub struct JoinCtx<'info> {
    #[account(mut, signer)]
    pub player: Signer<'info>,

    /// CHECK: used only as PDA seed reference and compared against bet_info.owner
    pub owner: UncheckedAccount<'info>,

    #[account(mut, seeds = [owner.key().as_ref()], bump)]
    pub bet_info: Account<'info, OracleBetInfo>,

    pub system_program: Program<'info, System>,
}

#[derive(Accounts)]
pub struct WinCtx<'info> {
    #[account(mut, signer)]
    pub player: Signer<'info>,

    /// CHECK: used as PDA seed reference only
    pub owner: UncheckedAccount<'info>,

    #[account(mut, seeds = [owner.key().as_ref()], bump)]
    pub bet_info: Account<'info, OracleBetInfo>,

    /// CHECK: validated at runtime using pyth_sdk_solana::load_price_feed_from_account_info
    pub price_feed: UncheckedAccount<'info>,

    pub system_program: Program<'info, System>,
}

#[derive(Accounts)]
pub struct TimeoutCtx<'info> {
    #[account(mut, signer)]
    pub owner: Signer<'info>,

    #[account(mut, seeds = [owner.key().as_ref()], bump)]
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
pub enum ErrorCode {
    #[msg("Owner or PDA mismatch")]
    Unauthorized,
    #[msg("Player already joined")]
    AlreadyJoined,
    #[msg("Deadline has already passed")]
    DeadlinePassed,
    #[msg("Deadline not reached")]
    DeadlineNotReached,
    #[msg("Caller is not the registered player")]
    NotPlayer,
    #[msg("Price feed is invalid")]
    OracleInvalid,
    #[msg("Oracle did not publish a price or price is stale")]
    StalePrice,
    #[msg("Player did not win")]
    NoWin,
    #[msg("Numeric overflow")]
    NumericOverflow,
    #[msg("Insufficient funds in PDA")]
    InsufficientFunds,
}
