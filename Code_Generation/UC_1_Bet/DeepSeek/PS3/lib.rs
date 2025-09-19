use anchor_lang::prelude::*;
use anchor_lang::system_program;

declare_id!("83bpwXMY356dufjxSCviNa7KiJjqD2AgsDpJmFh1T4Z1");

#[program]
pub mod betting_system {
    use super::*;

    pub fn join(ctx: Context<JoinCtx>, delay: u64, wager: u64) -> Result<()> {
        let bet_info = &mut ctx.accounts.bet_info;
        bet_info.oracle = ctx.accounts.oracle.key();
        bet_info.participant1 = ctx.accounts.participant1.key();
        bet_info.participant2 = ctx.accounts.participant2.key();
        bet_info.wager = wager;
        bet_info.deadline = Clock::get()?.slot + delay;
        bet_info.bump = ctx.bumps.bet_info;

        let cpi_ctx = CpiContext::new(
            ctx.accounts.system_program.to_account_info(),
            system_program::Transfer {
                from: ctx.accounts.participant1.to_account_info(),
                to: ctx.accounts.bet_info.to_account_info(),
            },
        );
        system_program::transfer(cpi_ctx, wager)?;

        let cpi_ctx = CpiContext::new(
            ctx.accounts.system_program.to_account_info(),
            system_program::Transfer {
                from: ctx.accounts.participant2.to_account_info(),
                to: ctx.accounts.bet_info.to_account_info(),
            },
        );
        system_program::transfer(cpi_ctx, wager)?;

        Ok(())
    }

    pub fn win(ctx: Context<WinCtx>) -> Result<()> {
        let bet_info = &ctx.accounts.bet_info;
        let current_slot = Clock::get()?.slot;
        require!(current_slot < bet_info.deadline, BetError::DeadlinePassed);
        require!(
            ctx.accounts.winner.key() == bet_info.participant1 || 
            ctx.accounts.winner.key() == bet_info.participant2,
            BetError::InvalidWinner
        );

        let amount = bet_info.wager * 2;
        
        // Direct lamport transfer instead of CPI
        **ctx.accounts.bet_info.to_account_info().try_borrow_mut_lamports()? -= amount;
        **ctx.accounts.winner.to_account_info().try_borrow_mut_lamports()? += amount;

        Ok(())
    }

    pub fn timeout(ctx: Context<TimeoutCtx>) -> Result<()> {
        let bet_info = &ctx.accounts.bet_info;
        let current_slot = Clock::get()?.slot;
        require!(current_slot >= bet_info.deadline, BetError::DeadlineNotReached);

        let wager_amount = bet_info.wager;
        
        // Direct lamport transfers instead of CPI
        **ctx.accounts.bet_info.to_account_info().try_borrow_mut_lamports()? -= wager_amount;
        **ctx.accounts.participant1.to_account_info().try_borrow_mut_lamports()? += wager_amount;
        
        **ctx.accounts.bet_info.to_account_info().try_borrow_mut_lamports()? -= wager_amount;
        **ctx.accounts.participant2.to_account_info().try_borrow_mut_lamports()? += wager_amount;

        Ok(())
    }
}

#[derive(Accounts)]
pub struct JoinCtx<'info> {
    #[account(mut)]
    participant1: Signer<'info>,
    #[account(mut)]
    participant2: Signer<'info>,
    oracle: SystemAccount<'info>,
    #[account(
        init,
        payer = participant1,
        space = 8 + BetInfo::INIT_SPACE,
        seeds = [participant1.key().as_ref(), participant2.key().as_ref()],
        bump
    )]
    bet_info: Account<'info, BetInfo>,
    system_program: Program<'info, System>,
}

#[derive(Accounts)]
pub struct WinCtx<'info> {
    #[account(mut)]
    oracle: Signer<'info>,
    #[account(mut)]
    winner: SystemAccount<'info>,
    #[account(
        mut,
        seeds = [participant1.key().as_ref(), participant2.key().as_ref()],
        bump = bet_info.bump,
        has_one = oracle
    )]
    bet_info: Account<'info, BetInfo>,
    /// CHECK: This account is used for PDA derivation
    participant1: UncheckedAccount<'info>,
    /// CHECK: This account is used for PDA derivation
    participant2: UncheckedAccount<'info>,
    system_program: Program<'info, System>,
}

#[derive(Accounts)]
pub struct TimeoutCtx<'info> {
    #[account(mut)]
    participant1: SystemAccount<'info>,
    #[account(mut)]
    participant2: SystemAccount<'info>,
    #[account(
        mut,
        seeds = [participant1.key().as_ref(), participant2.key().as_ref()],
        bump = bet_info.bump
    )]
    bet_info: Account<'info, BetInfo>,
    system_program: Program<'info, System>,
}

#[account]
#[derive(InitSpace)]
pub struct BetInfo {
    oracle: Pubkey,
    participant1: Pubkey,
    participant2: Pubkey,
    wager: u64,
    deadline: u64,
    bump: u8,
}

#[error_code]
pub enum BetError {
    #[msg("Deadline has not been reached")]
    DeadlineNotReached,
    #[msg("Deadline has already passed")]
    DeadlinePassed,
    #[msg("Invalid oracle")]
    InvalidOracle,
    #[msg("Winner is not one of the participants")]
    InvalidWinner,
}
