use anchor_lang::prelude::*;
use anchor_lang::system_program;

declare_id!("7QpnPrTEHkhAeN5cwmwR4j3bmBU7Wwjhmq58RMtNfKXo");

#[program]
pub mod betting {
    use super::*;

    pub fn join(ctx: Context<JoinCtx>, delay: u64, wager: u64) -> Result<()> {
        let clock = Clock::get()?;
        let deadline = clock.slot + delay;

        // Initialize bet info
        let bet_info = &mut ctx.accounts.bet_info;
        bet_info.participant1 = ctx.accounts.participant1.key();
        bet_info.participant2 = ctx.accounts.participant2.key();
        bet_info.oracle = ctx.accounts.oracle.key();
        bet_info.wager = wager;
        bet_info.deadline = deadline;

        // Transfer wagers from participants to PDA
        let cpi_program = ctx.accounts.system_program.to_account_info();
        
        let cpi_accounts = system_program::Transfer {
            from: ctx.accounts.participant1.to_account_info(),
            to: ctx.accounts.bet_info.to_account_info(),
        };
        let cpi_ctx = CpiContext::new(cpi_program.clone(), cpi_accounts);
        system_program::transfer(cpi_ctx, wager)?;

        let cpi_accounts = system_program::Transfer {
            from: ctx.accounts.participant2.to_account_info(),
            to: ctx.accounts.bet_info.to_account_info(),
        };
        let cpi_ctx = CpiContext::new(cpi_program, cpi_accounts);
        system_program::transfer(cpi_ctx, wager)?;

        Ok(())
    }

    pub fn win(ctx: Context<WinCtx>) -> Result<()> {
        // Verify oracle signature
        require!(
            ctx.accounts.oracle.key() == ctx.accounts.bet_info.oracle,
            ErrorCode::InvalidOracle
        );

        // Verify winner is one of the participants
        require!(
            ctx.accounts.winner.key() == ctx.accounts.bet_info.participant1 ||
            ctx.accounts.winner.key() == ctx.accounts.bet_info.participant2,
            ErrorCode::InvalidWinner
        );

        // Transfer entire pot to winner
        let pot = ctx.accounts.bet_info.to_account_info().lamports();
        **ctx.accounts.bet_info.to_account_info().try_borrow_mut_lamports()? = 0;
        **ctx.accounts.winner.try_borrow_mut_lamports()? = ctx.accounts.winner.lamports().checked_add(pot).unwrap();

        Ok(())
    }

    pub fn timeout(ctx: Context<TimeoutCtx>) -> Result<()> {
        // Check if deadline has passed
        let clock = Clock::get()?;
        require!(
            clock.slot > ctx.accounts.bet_info.deadline,
            ErrorCode::DeadlineNotReached
        );

        // Return wagers to participants
        let bet_info_lamports = ctx.accounts.bet_info.to_account_info().lamports();
        let refund_amount = bet_info_lamports / 2;

        **ctx.accounts.bet_info.to_account_info().try_borrow_mut_lamports()? = 0;
        **ctx.accounts.participant1.try_borrow_mut_lamports()? = ctx.accounts.participant1.lamports().checked_add(refund_amount).unwrap();
        **ctx.accounts.participant2.try_borrow_mut_lamports()? = ctx.accounts.participant2.lamports().checked_add(refund_amount).unwrap();

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
    /// CHECK: This is the oracle account that will decide the winner
    pub oracle: AccountInfo<'info>,
    #[account(
        init,
        payer = participant1,
        space = 8 + BetInfo::LEN,
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
    #[account(mut)]
    /// CHECK: This is the winner account that will receive the funds
    pub winner: AccountInfo<'info>,
    #[account(
        mut,
        seeds = [bet_info.participant1.as_ref(), bet_info.participant2.as_ref()],
        bump
    )]
    pub bet_info: Account<'info, BetInfo>,
    /// CHECK: Used for PDA validation only
    pub participant1: AccountInfo<'info>,
    /// CHECK: Used for PDA validation only
    pub participant2: AccountInfo<'info>,
    pub system_program: Program<'info, System>,
}

#[derive(Accounts)]
pub struct TimeoutCtx<'info> {
    #[account(mut)]
    /// CHECK: This is participant1 who will receive their refund
    pub participant1: AccountInfo<'info>,
    #[account(mut)]
    /// CHECK: This is participant2 who will receive their refund
    pub participant2: AccountInfo<'info>,
    #[account(
        mut,
        seeds = [participant1.key().as_ref(), participant2.key().as_ref()],
        bump,
        close = participant1
    )]
    pub bet_info: Account<'info, BetInfo>,
    pub system_program: Program<'info, System>,
}

#[account]
pub struct BetInfo {
    participant1: Pubkey,
    participant2: Pubkey,
    oracle: Pubkey,
    wager: u64,
    deadline: u64,
}

impl BetInfo {
    const LEN: usize = 32 + 32 + 32 + 8 + 8; // 3 Pubkeys (32 bytes each) + 2 u64 (8 bytes each)
}

#[error_code]
pub enum ErrorCode {
    #[msg("Invalid oracle account")]
    InvalidOracle,
    #[msg("Winner must be one of the participants")]
    InvalidWinner,
    #[msg("Deadline has not been reached")]
    DeadlineNotReached,
}
