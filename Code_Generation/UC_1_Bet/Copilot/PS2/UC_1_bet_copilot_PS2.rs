#![allow(unexpected_cfgs)]
use anchor_lang::prelude::*;
use anchor_lang::system_program;
use anchor_lang::solana_program::sysvar::{clock::Clock, Sysvar};

declare_id!("6qvR3ezKg21cJGoRyphGyKttgo1feQwihV7MKTNb9rDo");

#[program]
pub mod two_party_bet {
    use super::*;

    /// Both participants deposit their wager in a single, atomic transaction.
    pub fn join(ctx: Context<JoinCtx>, delay: u64, wager: u64) -> Result<()> {
        let bet = &mut ctx.accounts.bet_info;

        // Initialize on-chain state
        bet.participant1 = ctx.accounts.participant1.key();
        bet.participant2 = ctx.accounts.participant2.key();
        bet.oracle       = ctx.accounts.oracle.key();
        bet.wager        = wager;
        bet.bump = ctx.bumps.bet_info;
        let now = Clock::get()?.slot;
        bet.deadline     = now.checked_add(delay).ok_or(ErrorCode::Overflow)?;
        bet.is_resolved  = false;

        // Transfer participant1’s wager into the PDA
        system_program::transfer(
            CpiContext::new(
                ctx.accounts.system_program.to_account_info(),
                system_program::Transfer {
                    from: ctx.accounts.participant1.to_account_info(),
                    to:   ctx.accounts.bet_info.to_account_info(),
                },
            ),
            wager,
        )?;

        // Transfer participant2’s wager into the PDA
        system_program::transfer(
            CpiContext::new(
                ctx.accounts.system_program.to_account_info(),
                system_program::Transfer {
                    from: ctx.accounts.participant2.to_account_info(),
                    to:   ctx.accounts.bet_info.to_account_info(),
                },
            ),
            wager,
        )?;

        Ok(())
    }

    /// Oracle settles the bet—must be called *before* the deadline.
    pub fn win(ctx: Context<WinCtx>) -> Result<()> {
    // 1. Pre-read all state and do checks without holding a mutable borrow
    let now = Clock::get()?.slot;
    require!(!ctx.accounts.bet_info.is_resolved, ErrorCode::AlreadyResolved);
    require!(now <= ctx.accounts.bet_info.deadline, ErrorCode::DeadlinePassed);
    require!(
        ctx.accounts.oracle.key() == ctx.accounts.bet_info.oracle,
        ErrorCode::InvalidOracle
    );

    // 2. New check: ensure the designated winner is one of the participants
    let winner_key = ctx.accounts.winner.key();
    let p1 = ctx.accounts.bet_info.participant1;
    let p2 = ctx.accounts.bet_info.participant2;
    require!(
        winner_key == p1 || winner_key == p2,
        ErrorCode::InvalidWinner
    );

    // Calculate pot size
    let total = ctx
        .accounts
        .bet_info
        .wager
        .checked_mul(2)
        .ok_or(ErrorCode::Overflow)?;

    // 2. Perform the lamport transfer in its own scope
    {
        // Bind AccountInfo handles so borrows end at the closing brace
        let bet_info_ai = ctx.accounts.bet_info.to_account_info();
        let winner_ai   = ctx.accounts.winner.to_account_info();

        // Borrow lamports and adjust balances
        let mut from_lams = bet_info_ai.try_borrow_mut_lamports()?;
        let mut to_lams   = winner_ai.try_borrow_mut_lamports()?;

        **from_lams = from_lams
            .checked_sub(total)
            .ok_or(ErrorCode::Overflow)?;
        **to_lams = to_lams
            .checked_add(total)
            .ok_or(ErrorCode::Overflow)?;
    }

    // 3. Now safely borrow mutably to update resolution flag
    let bet = &mut ctx.accounts.bet_info;
    bet.is_resolved = true;
    Ok(())
}


    /// If time runs out, both participants can reclaim their wagers.
    pub fn timeout(ctx: Context<TimeoutCtx>) -> Result<()> {
    // 1. Pre-read state and enforce checks
    let now = Clock::get()?.slot;
    require!(!ctx.accounts.bet_info.is_resolved, ErrorCode::AlreadyResolved);
    require!(now > ctx.accounts.bet_info.deadline, ErrorCode::DeadlineNotReached);

    // Capture wager amount before any borrows
    let wager = ctx.accounts.bet_info.wager;

    // 2. Refund participant1 in its own block
    {
        let bet_info_ai = ctx.accounts.bet_info.to_account_info();
        let p1_ai       = ctx.accounts.participant1.to_account_info();

        let mut from_lams = bet_info_ai.try_borrow_mut_lamports()?;
        let mut to_lams   = p1_ai.try_borrow_mut_lamports()?;

        **from_lams = from_lams
            .checked_sub(wager)
            .ok_or(ErrorCode::Overflow)?;
        **to_lams = to_lams
            .checked_add(wager)
            .ok_or(ErrorCode::Overflow)?;
    }

    // 3. Refund participant2 in its own block
    {
        let bet_info_ai = ctx.accounts.bet_info.to_account_info();
        let p2_ai       = ctx.accounts.participant2.to_account_info();

        let mut from_lams = bet_info_ai.try_borrow_mut_lamports()?;
        let mut to_lams   = p2_ai.try_borrow_mut_lamports()?;

        **from_lams = from_lams
            .checked_sub(wager)
            .ok_or(ErrorCode::Overflow)?;
        **to_lams = to_lams
            .checked_add(wager)
            .ok_or(ErrorCode::Overflow)?;
    }

    // 4. Finally, mark as resolved
    let bet = &mut ctx.accounts.bet_info;
    bet.is_resolved = true;
    Ok(())
}

}

#[derive(Accounts)]
#[instruction(delay: u64, wager: u64)]
pub struct JoinCtx<'info> {
    #[account(mut)]
    pub participant1: Signer<'info>,

    #[account(mut)]
    pub participant2: Signer<'info>,

    /// CHECK: this is any Pubkey; we only validate it in `win()`
    pub oracle: UncheckedAccount<'info>,

    #[account(
        init,
        payer = participant1,
        space = 8 + std::mem::size_of::<BetInfo>(),
        seeds = [participant1.key().as_ref(), participant2.key().as_ref()],
        bump
    )]
    pub bet_info: Account<'info, BetInfo>,

    pub system_program: Program<'info, System>,
}

#[derive(Accounts)]
pub struct WinCtx<'info> {
    #[account(mut)]
    pub oracle: Signer<'info>,

    /// CHECK: recipient of the pot; no further checks needed here
    #[account(mut)]
    pub winner: UncheckedAccount<'info>,

    #[account(
        mut,
        seeds = [bet_info.participant1.as_ref(), bet_info.participant2.as_ref()],
        bump = bet_info.bump,
        has_one = participant1,
        has_one = participant2
    )]
    pub bet_info: Account<'info, BetInfo>,

    /// CHECK: only used to derive the PDA seeds
    pub participant1: UncheckedAccount<'info>,

    /// CHECK: only used to derive the PDA seeds
    pub participant2: UncheckedAccount<'info>,

    pub system_program: Program<'info, System>,
}

#[derive(Accounts)]
pub struct TimeoutCtx<'info> {
    #[account(mut)]
    pub participant1: Signer<'info>,

    #[account(mut)]
    pub participant2: Signer<'info>,

    #[account(
        mut,
        seeds = [bet_info.participant1.as_ref(), bet_info.participant2.as_ref()],
        bump = bet_info.bump,
        has_one = participant1,
        has_one = participant2
    )]
    pub bet_info: Account<'info, BetInfo>,

    pub system_program: Program<'info, System>,
}

#[account]
pub struct BetInfo {
    pub participant1: Pubkey,
    pub participant2: Pubkey,
    pub oracle: Pubkey,
    pub wager: u64,
    pub deadline: u64,
    pub is_resolved: bool,
    pub bump: u8,
}

#[error_code]
pub enum ErrorCode {
    #[msg("The bet has already been resolved")]
    AlreadyResolved,
    #[msg("The deadline has not been reached yet")]
    DeadlineNotReached,
    #[msg("The deadline has passed")]
    DeadlinePassed,
    #[msg("Invalid oracle")]
    InvalidOracle,
    #[msg("Arithmetic overflow")]
    Overflow,
    #[msg("The specified winner is not a participant")]
    InvalidWinner,      // ← New variant
}

