#![allow(unexpected_cfgs)]
use anchor_lang::prelude::*;
use anchor_lang::system_program;

declare_id!("83bpwXMY356dufjxSCviNa7KiJjqD2AgsDpJmFh1T4Z1");

#[program]
pub mod two_party_bet {
    use super::*;

    pub fn join(ctx: Context<JoinCtx>, delay: u64, wager: u64) -> Result<()> {
        require!(
            ctx.accounts.participant1.is_signer && ctx.accounts.participant2.is_signer,
            ErrorCode::MissingRequiredSignatures
        );

        let clock = Clock::get()?;
        let bet_info = &mut ctx.accounts.bet_info;
        
        bet_info.oracle = ctx.accounts.oracle.key();
        bet_info.participant1 = ctx.accounts.participant1.key();
        bet_info.participant2 = ctx.accounts.participant2.key();
        bet_info.wager = wager;
        bet_info.deadline = clock.slot + delay;
        bet_info.resolved = false;

        // Transfer wagers using CPI to avoid borrowing conflicts
        let cpi_program = ctx.accounts.system_program.to_account_info();
        let cpi_accounts = system_program::Transfer {
            from: ctx.accounts.participant1.to_account_info(),
            to: ctx.accounts.bet_info.to_account_info(),
        };
        let cpi_ctx = CpiContext::new(cpi_program.clone(), cpi_accounts);
        system_program::transfer(cpi_ctx, wager)?;

        let cpi_accounts2 = system_program::Transfer {
            from: ctx.accounts.participant2.to_account_info(),
            to: ctx.accounts.bet_info.to_account_info(),
        };
        let cpi_ctx2 = CpiContext::new(cpi_program, cpi_accounts2);
        system_program::transfer(cpi_ctx2, wager)?;

        Ok(())
    }

    pub fn win(ctx: Context<WinCtx>) -> Result<()> {
        let clock = Clock::get()?;
        
        // First, perform all checks without mutable borrows
        require!(
            ctx.accounts.oracle.key() == ctx.accounts.bet_info.oracle,
            ErrorCode::InvalidOracle
        );
        require!(!ctx.accounts.bet_info.resolved, ErrorCode::AlreadyResolved);
        require!(clock.slot <= ctx.accounts.bet_info.deadline, ErrorCode::DeadlinePassed);
        
        // NEW: Ensure winner is one of the participants
        require!(
            ctx.accounts.winner.key() == ctx.accounts.bet_info.participant1 ||
            ctx.accounts.winner.key() == ctx.accounts.bet_info.participant2,
            ErrorCode::InvalidWinner
        );

        // Calculate amount and perform transfers
        let amount = ctx.accounts.bet_info.wager.checked_mul(2).unwrap();
        let bet_info_ai = ctx.accounts.bet_info.to_account_info();
        let winner_ai = ctx.accounts.winner.to_account_info();
        
        // Perform lamport transfers directly
        **bet_info_ai.try_borrow_mut_lamports()? = bet_info_ai
            .lamports()
            .checked_sub(amount)
            .unwrap();
        **winner_ai.try_borrow_mut_lamports()? = winner_ai
            .lamports()
            .checked_add(amount)
            .unwrap();

        // Now update the state
        let bet_info = &mut ctx.accounts.bet_info;
        bet_info.resolved = true;

        Ok(())
    }

    pub fn timeout(ctx: Context<TimeoutCtx>) -> Result<()> {
        let clock = Clock::get()?;
        
        // First, perform all checks without mutable borrows
        require!(!ctx.accounts.bet_info.resolved, ErrorCode::AlreadyResolved);
        require!(clock.slot > ctx.accounts.bet_info.deadline, ErrorCode::DeadlineNotReached);

        // Calculate amount and perform transfers
        let amount = ctx.accounts.bet_info.wager;
        let total_amount = amount.checked_mul(2).unwrap();
        let bet_info_ai = ctx.accounts.bet_info.to_account_info();
        let participant1_ai = ctx.accounts.participant1.to_account_info();
        let participant2_ai = ctx.accounts.participant2.to_account_info();
        
        // Perform lamport transfers directly
        **bet_info_ai.try_borrow_mut_lamports()? = bet_info_ai
            .lamports()
            .checked_sub(total_amount)
            .unwrap();
        **participant1_ai.try_borrow_mut_lamports()? = participant1_ai
            .lamports()
            .checked_add(amount)
            .unwrap();
        **participant2_ai.try_borrow_mut_lamports()? = participant2_ai
            .lamports()
            .checked_add(amount)
            .unwrap();

        // Now update the state
        let bet_info = &mut ctx.accounts.bet_info;
        bet_info.resolved = true;

        Ok(())
    }
}

#[derive(Accounts)]
pub struct JoinCtx<'info> {
    #[account(mut)]
    participant1: Signer<'info>,
    #[account(mut)]
    participant2: Signer<'info>,
    /// CHECK: This is the oracle account that will be stored for future verification
    oracle: AccountInfo<'info>,
    #[account(
        init,
        payer = participant1,
        space = 8 + BetInfo::INIT_SPACE,
        seeds = [
            participant1.key().as_ref(),
            participant2.key().as_ref()
        ],
        bump
    )]
    bet_info: Account<'info, BetInfo>,
    system_program: Program<'info, System>,
}

#[derive(Accounts)]
pub struct WinCtx<'info> {
    #[account(mut, signer)]
    oracle: Signer<'info>,
    /// CHECK: This is the winner account that will receive the funds
    #[account(mut)]
    winner: AccountInfo<'info>,
    #[account(
        mut,
        seeds = [
            participant1.key().as_ref(),
            participant2.key().as_ref()
        ],
        bump,
        has_one = participant1,
        has_one = participant2
    )]
    bet_info: Account<'info, BetInfo>,
    /// CHECK: This is participant1 account for verification (matches bet_info)
    participant1: AccountInfo<'info>,
    /// CHECK: This is participant2 account for verification (matches bet_info)
    participant2: AccountInfo<'info>,
    system_program: Program<'info, System>,
}

#[derive(Accounts)]
pub struct TimeoutCtx<'info> {
    #[account(mut)]
    participant1: Signer<'info>,
    #[account(mut)]
    participant2: Signer<'info>,
    #[account(
        mut,
        seeds = [
            participant1.key().as_ref(),
            participant2.key().as_ref()
        ],
        bump
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
    resolved: bool,
}

#[error_code]
pub enum ErrorCode {
    #[msg("Invalid oracle account")]
    InvalidOracle,
    #[msg("Deadline has not been reached")]
    DeadlineNotReached,
    #[msg("Deadline has already passed")]
    DeadlinePassed,
    #[msg("Bet is already resolved")]
    AlreadyResolved,
    #[msg("Missing required signatures")]
    MissingRequiredSignatures,
    #[msg("Winner must be one of the participants")]
    InvalidWinner,
}
