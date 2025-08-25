#![allow(unexpected_cfgs)]
use anchor_lang::prelude::*;
use anchor_lang::system_program::{self, Transfer};

declare_id!("4nkn5qSPe5mRjgAoPM3y2w54NQB1d3mnHX8S7gYss9L4");

#[program]
pub mod bet_openai {
    use super::*;

    /// Both players must sign this single instruction together.
    pub fn join(ctx: Context<JoinCtx>, delay: u64, wager: u64) -> Result<()> {
        // Both required signatures in the same tx
        require!(
            ctx.accounts.participant1.is_signer && ctx.accounts.participant2.is_signer,
            BetError::MissingParticipantSignatures
        );
        require!(wager > 0, BetError::InvalidWager);
        require!(delay > 0, BetError::InvalidDelay);

        let bet_ai = ctx.accounts.bet_info.to_account_info();
        let p1_ai = ctx.accounts.participant1.to_account_info();
        let p2_ai = ctx.accounts.participant2.to_account_info();

        // Move equal wagers from both participants to the PDA using direct lamport adjustments.
        // This is allowed because both participant accounts are writable and bet_info is owned by this program.
        // Note: We cannot *subtract* lamports from participant accounts directly (they are not owned by this program),
        // so we must deposit into bet_info via System Program for participants. To avoid the "data account" restriction,
        // we do it the other way around: debit participants and credit bet_info via CPI transfers is fine because
        // the *source* is a system account (participants). We'll implement that using the System Program.
        // However, to keep the account list minimal, we can use the simple "transfer" helper
        // by crafting a system_instruction and invoking it.

        // Safer approach for clarity: CPI to System Program for both deposits.
        {
            let ix1 = anchor_lang::solana_program::system_instruction::transfer(
                &p1_ai.key(),
                &bet_ai.key(),
                wager,
            );
            anchor_lang::solana_program::program::invoke(
                &ix1,
                &[
                    p1_ai.clone(),
                    bet_ai.clone(),
                    ctx.accounts.system_program.to_account_info(),
                ],
            )?;

            let ix2 = anchor_lang::solana_program::system_instruction::transfer(
                &p2_ai.key(),
                &bet_ai.key(),
                wager,
            );
            anchor_lang::solana_program::program::invoke(
                &ix2,
                &[
                    p2_ai,
                    bet_ai.clone(),
                    ctx.accounts.system_program.to_account_info(),
                ],
            )?;
        }

        // Initialize state AFTER funds arrive (prevents frontrunning / partial funding)
        let clock = Clock::get()?;
        let bet = &mut ctx.accounts.bet_info;
        require!(!bet.initialized, BetError::AlreadyInitialized);

        bet.participant1 = ctx.accounts.participant1.key();
        bet.participant2 = ctx.accounts.participant2.key();
        bet.oracle = ctx.accounts.oracle.key();
        bet.wager = wager;
        bet.deadline = clock.slot.checked_add(delay).ok_or(BetError::MathOverflow)?;
        bet.resolved = false;
        bet.initialized = true;
        bet.bump = ctx.bumps.bet_info;

        Ok(())
    }

    /// Oracle selects the winner before deadline. Drains PDA to winner and closes PDA.
    pub fn win(ctx: Context<WinCtx>) -> Result<()> {
        // Validate
        require!(ctx.accounts.oracle.is_signer, BetError::MissingOracleSignature);
        require_keys_eq!(ctx.accounts.oracle.key(), ctx.accounts.bet_info.oracle, BetError::InvalidOracle);
        require!(!ctx.accounts.bet_info.resolved, BetError::AlreadyResolved);

        let clock = Clock::get()?;
        require!(clock.slot <= ctx.accounts.bet_info.deadline, BetError::DeadlinePassed);

        let winner_key = ctx.accounts.winner.key();
        require!(
            winner_key == ctx.accounts.bet_info.participant1 || winner_key == ctx.accounts.bet_info.participant2,
            BetError::InvalidWinner
        );

        // Ensure fully funded (2 * wager) is present
        let pot = ctx.accounts.bet_info.to_account_info().lamports();
        let expected = ctx.accounts.bet_info.wager.checked_mul(2).ok_or(BetError::MathOverflow)?;
        require!(pot >= expected, BetError::NotFunded);

        // No CPI transfer from a data account. Simply mark resolved; the account will be closed to the winner,
        // which will move *all* lamports (pot + rent) to the winner in a single atomic operation.
        let bet = &mut ctx.accounts.bet_info;
        bet.resolved = true;

        Ok(())
    }

    /// After deadline, either participant can trigger refunds. Refund both wagers, then close PDA (any remainder to p1).
    pub fn timeout(ctx: Context<TimeoutCtx>) -> Result<()> {
        // Signature from either participant
        require!(
            ctx.accounts.participant1.is_signer || ctx.accounts.participant2.is_signer,
            BetError::MissingParticipantSignatures
        );
        require!(!ctx.accounts.bet_info.resolved, BetError::AlreadyResolved);

        let clock = Clock::get()?;
        require!(clock.slot > ctx.accounts.bet_info.deadline, BetError::DeadlineNotReached);

        // Refund wagers: directly move lamports from the program-owned PDA to the participants
        let refund = ctx.accounts.bet_info.wager;
        require!(refund > 0, BetError::InvalidWager);

        let bet_info_ai = ctx.accounts.bet_info.to_account_info();
        let p1_ai = ctx.accounts.participant1.to_account_info();
        let p2_ai = ctx.accounts.participant2.to_account_info();

        // Check sufficient balance for both refunds
        let pda_balance = bet_info_ai.lamports();
        let expected = refund.checked_mul(2).ok_or(BetError::MathOverflow)?;
        require!(pda_balance >= expected, BetError::NotFunded);

        // Move refund to participant1
        **p1_ai.try_borrow_mut_lamports()? += refund;
        **bet_info_ai.try_borrow_mut_lamports()? -= refund;

        // Move refund to participant2
        **p2_ai.try_borrow_mut_lamports()? += refund;
        **bet_info_ai.try_borrow_mut_lamports()? -= refund;

        // Mark resolved; remaining lamports (e.g., rent) are sent to participant1 via close.
        let bet = &mut ctx.accounts.bet_info;
        bet.resolved = true;

        Ok(())
    }
}

/* =========================
   Accounts
   ========================= */

#[derive(Accounts)]
#[instruction(delay: u64, wager: u64)]
pub struct JoinCtx<'info> {
    #[account(mut)]
    pub participant1: Signer<'info>,
    #[account(mut)]
    pub participant2: Signer<'info>,

    /// CHECK: Only stored and later verified in `win`.
    pub oracle: UncheckedAccount<'info>,

    // Single PDA storing state AND temporarily holding the pot.
    // Seeds must be exactly [p1, p2] as per your spec.
    #[account(
        init,
        payer = participant1,
        space = BetInfo::SPACE,
        seeds = [participant1.key().as_ref(), participant2.key().as_ref()],
        bump
    )]
    pub bet_info: Account<'info, BetInfo>,

    pub system_program: Program<'info, System>,
}

#[derive(Accounts)]
pub struct WinCtx<'info> {
    pub oracle: Signer<'info>,
    #[account(mut)]
    pub winner: SystemAccount<'info>,

    // Allow Anchor to close this PDA to `winner` automatically after handler returns.
    #[account(
        mut,
        close = winner,
        seeds = [participant1.key().as_ref(), participant2.key().as_ref()],
        bump = bet_info.bump
    )]
    pub bet_info: Account<'info, BetInfo>,

    /// CHECK: Used only for PDA seed derivation; not read/written.
    pub participant1: UncheckedAccount<'info>,
    /// CHECK: Used only for PDA seed derivation; not read/written.
    pub participant2: UncheckedAccount<'info>,

    pub system_program: Program<'info, System>,
}

#[derive(Accounts)]
pub struct TimeoutCtx<'info> {
    #[account(mut)]
    pub participant1: SystemAccount<'info>,
    #[account(mut)]
    pub participant2: SystemAccount<'info>,

    // Close to participant1 after refunds to drain remaining lamports (e.g., rent) and set account to zero.
    #[account(
        mut,
        close = participant1,
        seeds = [participant1.key().as_ref(), participant2.key().as_ref()],
        bump = bet_info.bump
    )]
    pub bet_info: Account<'info, BetInfo>,

    pub system_program: Program<'info, System>,
}

/* =========================
   State
   ========================= */

#[account]
pub struct BetInfo {
    pub participant1: Pubkey,
    pub participant2: Pubkey,
    pub oracle: Pubkey,
    pub wager: u64,
    pub deadline: u64, // slot
    pub resolved: bool,
    pub initialized: bool,
    pub bump: u8,
}
impl BetInfo {
    pub const SPACE: usize = 8 // discriminator
        + 32 + 32 + 32        // keys
        + 8 + 8               // wager + deadline
        + 1 + 1               // resolved + initialized
        + 1;                  // bump
}

/* =========================
   Errors
   ========================= */

#[error_code]
pub enum BetError {
    #[msg("Both participants must sign the join or timeout call.")]
    MissingParticipantSignatures,
    #[msg("Oracle signature required.")]
    MissingOracleSignature,
    #[msg("Math overflow.")]
    MathOverflow,
    #[msg("Bet already initialized.")]
    AlreadyInitialized,
    #[msg("Bet already resolved.")]
    AlreadyResolved,
    #[msg("Invalid oracle account.")]
    InvalidOracle,
    #[msg("Invalid winner: must be one of the two participants.")]
    InvalidWinner,
    #[msg("Deadline has already passed for win().")]
    DeadlinePassed,
    #[msg("Deadline not reached for timeout().")]
    DeadlineNotReached,
    #[msg("Bet is not fully funded.")]
    NotFunded,
    #[msg("Invalid wager.")]
    InvalidWager,
    #[msg("Invalid delay.")]
    InvalidDelay,
}

