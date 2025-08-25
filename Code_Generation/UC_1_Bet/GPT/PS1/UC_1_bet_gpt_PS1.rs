#![allow(unexpected_cfgs)]
use anchor_lang::prelude::*;
use anchor_lang::system_program;
use anchor_lang::solana_program::{
    program::invoke_signed,
    system_instruction,
};

declare_id!("DnozfNC6MfEXQgnpYoYtTEpK4oPgfhdwb6UvGaCVHJdH");

#[program]
pub mod two_party_bet {
    use super::*;

    /// join: both participant1 and participant2 MUST sign the same transaction.
    /// Transfers `wager` lamports from each participant into the PDA (bet_info).
    /// Stores state and deadline = current_slot + delay.
    pub fn join(ctx: Context<JoinCtx>, delay: u64, wager: u64) -> Result<()> {
        // Enforce signatures (Anchor also ensures Signer<'info>, defensive check)
        require!(ctx.accounts.participant1.is_signer, BetError::MissingParticipantSignature);
        require!(ctx.accounts.participant2.is_signer, BetError::MissingParticipantSignature);

        // Basic sanity
        require!(wager > 0, BetError::InvalidWager);
        require_keys_neq!(ctx.accounts.participant1.key(), ctx.accounts.participant2.key());
        require!(ctx.accounts.oracle.key() != Pubkey::default(), BetError::InvalidOracle);

        let bet_info = &mut ctx.accounts.bet_info;

        // Shouldn't be already initialized (init enforces this, but keep defensive)
        require!(!bet_info.initialized, BetError::AlreadyInitialized);

        // compute deadline
        let now_slot = Clock::get()?.slot;
        let deadline = now_slot.checked_add(delay).ok_or(BetError::MathOverflow)?;

        // Save state
        bet_info.participant1 = ctx.accounts.participant1.key();
        bet_info.participant2 = ctx.accounts.participant2.key();
        bet_info.oracle = ctx.accounts.oracle.key();
        bet_info.wager = wager;
        bet_info.deadline_slot = deadline;
        bet_info.resolved = false;
        bet_info.initialized = true;
        // store the bump provided by Anchor on init
        bet_info.bump = ctx.bumps.bet_info;

        // Transfer wager from participant1 -> bet_info (participant1 is signer)
        let ix1 = system_instruction::transfer(
            &ctx.accounts.participant1.key(),
            &ctx.accounts.bet_info.key(),
            wager,
        );
        invoke_signed(
            &ix1,
            &[
                ctx.accounts.participant1.to_account_info(),
                ctx.accounts.bet_info.to_account_info(),
                ctx.accounts.system_program.to_account_info(),
            ],
            &[],
        )?;

        // Transfer wager from participant2 -> bet_info (participant2 is signer)
        let ix2 = system_instruction::transfer(
            &ctx.accounts.participant2.key(),
            &ctx.accounts.bet_info.key(),
            wager,
        );
        invoke_signed(
            &ix2,
            &[
                ctx.accounts.participant2.to_account_info(),
                ctx.accounts.bet_info.to_account_info(),
                ctx.accounts.system_program.to_account_info(),
            ],
            &[],
        )?;

        Ok(())
    }

    /// win: only callable by designated oracle (Signer).
    /// Transfers entire pot (2 * wager) from PDA to `winner`, marks resolved.
    pub fn win(ctx: Context<WinCtx>, winner: Pubkey) -> Result<()> {
        // --- Immutable stuff first ---
        let bet_info_ai = ctx.accounts.bet_info.to_account_info();
        let bet_info_key = ctx.accounts.bet_info.key();

        let system_program = ctx.accounts.system_program.to_account_info();

        let p1_key = ctx.accounts.participant1.key();
        let p2_key = ctx.accounts.participant2.key();

        // --- Now mutable borrow ---
        let bet_info = &mut ctx.accounts.bet_info;

        // Ensure not already resolved
        require!(!bet_info.resolved, CustomError::AlreadyResolved);

        // Determine winner and loser
        let (winner_key, loser_key) = if winner == p1_key {
            (p1_key, p2_key)
        } else if winner == p2_key {
            (p2_key, p1_key)
        } else {
            return Err(CustomError::InvalidWinner.into());
        };

        let total_pot = bet_info.wager * 2;

        // Transfer pot to winner
        let ix = system_instruction::transfer(&bet_info_key, &winner_key, total_pot);
        invoke_signed(
            &ix,
            &[bet_info_ai.clone(), ctx.accounts.winner.to_account_info(), system_program],
            &[&[
                ctx.accounts.participant1.key().as_ref(),
                ctx.accounts.participant2.key().as_ref(),
                &[bet_info.bump],
            ]],
        )?;

        bet_info.resolved = true;

        Ok(())
    }


    /// timeout: can be called by either participant (we check is_signer).
    /// Only if deadline reached. Refunds each participant `wager` lamports from PDA and marks resolved.
    pub fn timeout(ctx: Context<TimeoutCtx>) -> Result<()> {
        // --- Immutable stuff first ---
        let bet_info_ai = ctx.accounts.bet_info.to_account_info();
        let bet_info_key = ctx.accounts.bet_info.key();

        let system_program = ctx.accounts.system_program.to_account_info();

        let p1_key = ctx.accounts.participant1.key();
        let p2_key = ctx.accounts.participant2.key();

        // --- Now mutable borrow ---
        let bet_info = &mut ctx.accounts.bet_info;

        // Ensure not already resolved
        require!(!bet_info.resolved, CustomError::AlreadyResolved);

        let wager = bet_info.wager;

        // Refund both participants
        let ix1 = system_instruction::transfer(&bet_info_key, &p1_key, wager);
        invoke_signed(
            &ix1,
            &[bet_info_ai.clone(), ctx.accounts.participant1.to_account_info(), system_program.clone()],
            &[&[
                ctx.accounts.participant1.key().as_ref(),
                ctx.accounts.participant2.key().as_ref(),
                &[bet_info.bump],
            ]],
        )?;

        let ix2 = system_instruction::transfer(&bet_info_key, &p2_key, wager);
        invoke_signed(
            &ix2,
            &[bet_info_ai.clone(), ctx.accounts.participant2.to_account_info(), system_program],
            &[&[
                ctx.accounts.participant1.key().as_ref(),
                ctx.accounts.participant2.key().as_ref(),
                &[bet_info.bump],
            ]],
        )?;

        bet_info.resolved = true;

        Ok(())
    }

}

// ------------------------------ Contexts ------------------------------

#[derive(Accounts)]
#[instruction()]
pub struct JoinCtx<'info> {
    #[account(mut)]
    pub participant1: Signer<'info>,

    #[account(mut)]
    pub participant2: Signer<'info>,

    /// CHECK: oracle account is only stored; not modified
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
    /// CHECK: Winner account is validated in program logic
    #[account(mut)]
    pub winner: AccountInfo<'info>,

    #[account(
        mut,
        seeds = [participant1.key().as_ref(), participant2.key().as_ref()],
        bump = bet_info.bump
    )]
    pub bet_info: Account<'info, BetInfo>,

    pub oracle: Signer<'info>,

    /// CHECK: Used for PDA derivation
    pub participant1: UncheckedAccount<'info>,

    /// CHECK: Used for PDA derivation
    pub participant2: UncheckedAccount<'info>,

    pub system_program: Program<'info, System>,
}


#[derive(Accounts)]
pub struct TimeoutCtx<'info> {
    #[account(mut)]
    pub participant1: UncheckedAccount<'info>,

    #[account(mut)]
    pub participant2: UncheckedAccount<'info>,

    #[account(
        mut,
        seeds = [participant1.key().as_ref(), participant2.key().as_ref()],
        bump = bet_info.bump
    )]
    pub bet_info: Account<'info, BetInfo>,

    pub system_program: Program<'info, System>,
}

// ------------------------------ State ------------------------------

#[account]
pub struct BetInfo {
    pub participant1: Pubkey,
    pub participant2: Pubkey,
    pub oracle: Pubkey,
    pub resolved: bool,
    pub winner: Pubkey,
    pub bump: u8,
}

#[account]
pub struct BetInfo {
    pub participant1: Pubkey,
    pub participant2: Pubkey,
    pub oracle: Pubkey,
    pub resolved: bool,
    pub winner: Pubkey,
    pub bump: u8,

    pub initialized: bool,    // tracks if join() has been called
    pub wager: u64,           // amount each participant is betting
    pub deadline_slot: u64,   // slot until which bet can be claimed
}


// ------------------------------ Errors ------------------------------

#[error_code]
pub enum BetError {
    #[msg("Both participants already initialized this bet.")]
    AlreadyInitialized,
    #[msg("Bet already resolved.")]
    AlreadyResolved,
    #[msg("Invalid wager amount.")]
    InvalidWager,
    #[msg("Invalid oracle account.")]
    InvalidOracle,
    #[msg("Math overflow.")]
    MathOverflow,
    #[msg("Deadline has already passed for win.")]
    DeadlinePassedForWin,
    #[msg("Deadline not yet reached for timeout.")]
    DeadlineNotReached,
    #[msg("Oracle signature is required.")]
    MissingOracleSignature,
    #[msg("At least one participant signature is required.")]
    MissingParticipantSignature,
    #[msg("Bet account not initialized.")]
    Uninitialized,
    #[msg("Winner must be one of the two participants.")]
    InvalidWinner,
    #[msg("Insufficient lamports in pot.")]
    InsufficientPot,
}

#[error_code]
pub enum CustomError {
    #[msg("The bet has already been resolved")]
    AlreadyResolved,
    #[msg("Invalid winner account provided")]
    InvalidWinner,
}
