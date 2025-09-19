use anchor_lang::prelude::*;
use anchor_lang::system_program;

declare_id!("4nkn5qSPe5mRjgAoPM3y2w54NQB1d3mnHX8S7gYss9L4");

#[program]
pub mod two_player_bet {
    use super::*;

    pub fn join(ctx: Context<JoinCtx>, delay: u64, wager: u64) -> Result<()> {
        require!(wager > 0, BetError::InvalidWager);
        require!(
            ctx.accounts.participant1.key() != ctx.accounts.participant2.key(),
            BetError::SameParticipant
        );

        let current_slot = Clock::get()?.slot;
        let deadline = current_slot
            .checked_add(delay)
            .ok_or(BetError::MathOverflow)?;

        let bet = &mut ctx.accounts.bet_info;
        bet.participant1 = ctx.accounts.participant1.key();
        bet.participant2 = ctx.accounts.participant2.key();
        bet.oracle = ctx.accounts.oracle.key();
        bet.wager = wager;
        bet.deadline = deadline;
        bet.bump = ctx.bumps.bet_info;
        bet.settled = false;

        // Transfer wagers
        system_program::transfer(
            CpiContext::new(
                ctx.accounts.system_program.to_account_info(),
                system_program::Transfer {
                    from: ctx.accounts.participant1.to_account_info(),
                    to: ctx.accounts.bet_info.to_account_info(),
                },
            ),
            wager,
        )?;

        system_program::transfer(
            CpiContext::new(
                ctx.accounts.system_program.to_account_info(),
                system_program::Transfer {
                    from: ctx.accounts.participant2.to_account_info(),
                    to: ctx.accounts.bet_info.to_account_info(),
                },
            ),
            wager,
        )?;

        Ok(())
    }

    pub fn win(ctx: Context<WinCtx>) -> Result<()> {
    let p1 = ctx.accounts.participant1.key();
    let p2 = ctx.accounts.participant2.key();
    let wager = ctx.accounts.bet_info.wager;
    let bump = ctx.accounts.bet_info.bump;
    let deadline = ctx.accounts.bet_info.deadline;
    let settled = ctx.accounts.bet_info.settled;

    require!(!settled, BetError::AlreadySettled);
    let now = Clock::get()?.slot;
    require!(now <= deadline, BetError::DeadlinePassed);

    let winner_key = ctx.accounts.winner.key();
    require!(winner_key == p1 || winner_key == p2, BetError::InvalidWinner);

    let pot = wager.checked_mul(2).ok_or(BetError::MathOverflow)?;
    require!(
        ctx.accounts.bet_info.to_account_info().lamports() >= pot,
        BetError::InsufficientPot
    );

    // --- Transfer pot to winner ---
    {
        let from = &mut ctx.accounts.bet_info.to_account_info();
        let to = &mut ctx.accounts.winner.to_account_info();

        **from.try_borrow_mut_lamports()? -= pot;
        **to.try_borrow_mut_lamports()? += pot;
    }

    // --- Drain remainder (rent, etc.) to participant1 ---
    {
        let from = &mut ctx.accounts.bet_info.to_account_info();
        let to = &mut ctx.accounts.participant1.to_account_info();
        let remainder = from.lamports();

        if remainder > 0 {
            **from.try_borrow_mut_lamports()? -= remainder;
            **to.try_borrow_mut_lamports()? += remainder;
        }
    }

    let bet = &mut ctx.accounts.bet_info;
    bet.settled = true;
    bet.wager = 0;

    Ok(())
}

pub fn timeout(ctx: Context<TimeoutCtx>) -> Result<()> {
    let p1 = ctx.accounts.participant1.key();
    let p2 = ctx.accounts.participant2.key();
    let wager = ctx.accounts.bet_info.wager;
    let deadline = ctx.accounts.bet_info.deadline;
    let settled = ctx.accounts.bet_info.settled;

    require!(
        ctx.accounts.participant1.to_account_info().is_signer
            || ctx.accounts.participant2.to_account_info().is_signer,
        BetError::Unauthorized
    );
    require!(!settled, BetError::AlreadySettled);

    let now = Clock::get()?.slot;
    require!(now >= deadline, BetError::DeadlineNotReached);

    let pot_needed = wager.checked_mul(2).ok_or(BetError::MathOverflow)?;
    require!(
        ctx.accounts.bet_info.to_account_info().lamports() >= pot_needed,
        BetError::InsufficientPot
    );

    // --- Refund wagers ---
    if wager > 0 {
        {
            let from = &mut ctx.accounts.bet_info.to_account_info();
            let to = &mut ctx.accounts.participant1.to_account_info();
            **from.try_borrow_mut_lamports()? -= wager;
            **to.try_borrow_mut_lamports()? += wager;
        }
        {
            let from = &mut ctx.accounts.bet_info.to_account_info();
            let to = &mut ctx.accounts.participant2.to_account_info();
            **from.try_borrow_mut_lamports()? -= wager;
            **to.try_borrow_mut_lamports()? += wager;
        }
    }

    // --- Drain remainder (rent) to participant1 ---
    {
        let from = &mut ctx.accounts.bet_info.to_account_info();
        let to = &mut ctx.accounts.participant1.to_account_info();
        let remainder = from.lamports();

        if remainder > 0 {
            **from.try_borrow_mut_lamports()? -= remainder;
            **to.try_borrow_mut_lamports()? += remainder;
        }
    }

    let bet = &mut ctx.accounts.bet_info;
    bet.settled = true;
    bet.wager = 0;

    Ok(())
}

}

#[derive(Accounts)]
pub struct JoinCtx<'info> {
    #[account(mut, signer)]
    pub participant1: Signer<'info>,

    #[account(mut, signer)]
    pub participant2: Signer<'info>,

    /// CHECK: We only record this account’s Pubkey in the BetInfo state.
    /// No data or lamport access occurs, and validity is enforced later when the oracle signs `win`.
    pub oracle: UncheckedAccount<'info>,

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
    #[account(signer)]
    pub oracle: Signer<'info>,
    #[account(mut)]
    pub winner: SystemAccount<'info>,
    #[account(
        mut,
        seeds = [participant1.key().as_ref(), participant2.key().as_ref()],
        bump = bet_info.bump,
        has_one = oracle @ BetError::InvalidOracle,
        constraint = !bet_info.settled @ BetError::AlreadySettled
    )]
    pub bet_info: Account<'info, BetInfo>,
    #[account(mut)]
    pub participant1: SystemAccount<'info>,
    #[account(mut)]
    pub participant2: SystemAccount<'info>,
    pub system_program: Program<'info, System>,
}

#[derive(Accounts)]
pub struct TimeoutCtx<'info> {
    #[account(mut)]
    pub participant1: SystemAccount<'info>,
    #[account(mut)]
    pub participant2: SystemAccount<'info>,
    #[account(
        mut,
        seeds = [participant1.key().as_ref(), participant2.key().as_ref()],
        bump = bet_info.bump
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
    pub bump: u8,
    pub settled: bool,
}

impl BetInfo {
    pub const SPACE: usize = 8 + 32 + 32 + 32 + 8 + 8 + 1 + 1;
}

#[error_code]
pub enum BetError {
    #[msg("Invalid wager amount.")]
    InvalidWager,
    #[msg("Participants must be distinct.")]
    SameParticipant,
    #[msg("Math overflow.")]
    MathOverflow,
    #[msg("Only the designated oracle may call this.")]
    InvalidOracle,
    #[msg("This bet has already been settled.")]
    AlreadySettled,
    #[msg("The deadline has not been reached yet.")]
    DeadlineNotReached,
    #[msg("The deadline has already passed.")]
    DeadlinePassed,
    #[msg("The winner must be one of the two participants.")]
    InvalidWinner,
    #[msg("Insufficient pot in PDA.")]
    InsufficientPot,
    #[msg("Unauthorized: either participant must sign.")]
    Unauthorized,
}


