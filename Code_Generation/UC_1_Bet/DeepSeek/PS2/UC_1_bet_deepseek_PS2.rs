#![allow(unexpected_cfgs)]
use anchor_lang::prelude::*;
use anchor_lang::system_program::{transfer, Transfer};

declare_id!("7QpnPrTEHkhAeN5cwmwR4j3bmBU7Wwjhmq58RMtNfKXo");

#[program]
pub mod two_party_bet {
    use super::*;

    pub fn join(ctx: Context<JoinCtx>, delay: u64, wager: u64) -> Result<()> {
        let bet_info = &mut ctx.accounts.bet_info;
        
        // Initialize bet state
        bet_info.participant1 = ctx.accounts.participant1.key();
        bet_info.participant2 = ctx.accounts.participant2.key();
        bet_info.oracle = ctx.accounts.oracle.key();
        bet_info.wager = wager;
        bet_info.deadline = Clock::get()?.slot + delay;
        bet_info.state = BetState::Active;
        bet_info.bump = ctx.bumps.bet_info;

        // Transfer wagers from participants to the bet_info PDA
        let cpi_program = ctx.accounts.system_program.to_account_info();
        
        // First participant's transfer
        let cpi_accounts = Transfer {
            from: ctx.accounts.participant1.to_account_info(),
            to: ctx.accounts.bet_info.to_account_info(),
        };
        let cpi_ctx = CpiContext::new(cpi_program.clone(), cpi_accounts);
        transfer(cpi_ctx, wager)?;

        // Second participant's transfer
        let cpi_accounts = Transfer {
            from: ctx.accounts.participant2.to_account_info(),
            to: ctx.accounts.bet_info.to_account_info(),
        };
        let cpi_ctx = CpiContext::new(cpi_program, cpi_accounts);
        transfer(cpi_ctx, wager)?;

        Ok(())
    }

    pub fn win(ctx: Context<WinCtx>) -> Result<()> {
        // Validation checks
        require!(ctx.accounts.bet_info.state == BetState::Active, BetError::AlreadyResolved);
        require!(Clock::get()?.slot < ctx.accounts.bet_info.deadline, BetError::DeadlinePassed);
        require!(ctx.accounts.oracle.key() == ctx.accounts.bet_info.oracle, BetError::InvalidOracle);
        
        // Ensure the provided participants match the stored ones
        require!(
            ctx.accounts.participant1.key() == ctx.accounts.bet_info.participant1 &&
            ctx.accounts.participant2.key() == ctx.accounts.bet_info.participant2,
            BetError::InvalidParticipants
        );
        
        // Prevent declaring non-participant as winner
        require!(
            ctx.accounts.winner.key() == ctx.accounts.bet_info.participant1 ||
            ctx.accounts.winner.key() == ctx.accounts.bet_info.participant2,
            BetError::InvalidWinner
        );
        
        // Transfer total wager to winner
        let amount = ctx.accounts.bet_info.wager.checked_mul(2).unwrap();
        
        // Use manual lamport transfer to avoid system program restrictions
        let bet_info_account = ctx.accounts.bet_info.to_account_info();
        let winner_account = ctx.accounts.winner.to_account_info();
        
        // Transfer funds from bet_info to winner
        **winner_account.try_borrow_mut_lamports()? = winner_account
            .lamports()
            .checked_add(amount)
            .ok_or(BetError::Overflow)?;
            
        **bet_info_account.try_borrow_mut_lamports()? = bet_info_account
            .lamports()
            .checked_sub(amount)
            .ok_or(BetError::InsufficientFunds)?;

        // Update state after transfer
        ctx.accounts.bet_info.state = BetState::Resolved;
        
        Ok(())
    }

    pub fn timeout(ctx: Context<TimeoutCtx>) -> Result<()> {
        // Validation checks
        require!(ctx.accounts.bet_info.state == BetState::Active, BetError::AlreadyResolved);
        require!(Clock::get()?.slot >= ctx.accounts.bet_info.deadline, BetError::DeadlineNotReached);
        
        // Refund wagers to both participants
        let wager = ctx.accounts.bet_info.wager;
        let bet_info_account = ctx.accounts.bet_info.to_account_info();
        let participant1_account = ctx.accounts.participant1.to_account_info();
        let participant2_account = ctx.accounts.participant2.to_account_info();
        
        // Refund to participant1
        **participant1_account.try_borrow_mut_lamports()? = participant1_account
            .lamports()
            .checked_add(wager)
            .ok_or(BetError::Overflow)?;
            
        **bet_info_account.try_borrow_mut_lamports()? = bet_info_account
            .lamports()
            .checked_sub(wager)
            .ok_or(BetError::InsufficientFunds)?;

        // Refund to participant2
        **participant2_account.try_borrow_mut_lamports()? = participant2_account
            .lamports()
            .checked_add(wager)
            .ok_or(BetError::Overflow)?;
            
        **bet_info_account.try_borrow_mut_lamports()? = bet_info_account
            .lamports()
            .checked_sub(wager)
            .ok_or(BetError::InsufficientFunds)?;

        // Update state after transfers
        ctx.accounts.bet_info.state = BetState::Resolved;
        
        Ok(())
    }
}

#[derive(Accounts)]
pub struct JoinCtx<'info> {
    #[account(mut)]
    participant1: Signer<'info>,
    #[account(mut)]
    participant2: Signer<'info>,
    /// CHECK: This is the oracle account that will be stored and later used to resolve the bet
    oracle: AccountInfo<'info>,
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
    /// CHECK: This is the winner account that will receive the wager
    #[account(mut)]
    winner: AccountInfo<'info>,
    #[account(
        mut,
        seeds = [participant1.key().as_ref(), participant2.key().as_ref()],
        bump = bet_info.bump
    )]
    bet_info: Account<'info, BetInfo>,
    /// CHECK: This account is used for PDA derivation and validation
    participant1: AccountInfo<'info>,
    /// CHECK: This account is used for PDA derivation and validation
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
        seeds = [participant1.key().as_ref(), participant2.key().as_ref()],
        bump = bet_info.bump
    )]
    bet_info: Account<'info, BetInfo>,
    system_program: Program<'info, System>,
}

#[account]
#[derive(InitSpace)]
pub struct BetInfo {
    participant1: Pubkey,
    participant2: Pubkey,
    oracle: Pubkey,
    wager: u64,
    deadline: u64,
    state: BetState,
    bump: u8,
}

#[derive(Clone, Copy, PartialEq, AnchorSerialize, AnchorDeserialize, InitSpace)]
pub enum BetState {
    Active,
    Resolved,
}

#[error_code]
pub enum BetError {
    #[msg("Bet is already resolved")]
    AlreadyResolved,
    #[msg("Deadline has not been reached")]
    DeadlineNotReached,
    #[msg("Deadline has passed")]
    DeadlinePassed,
    #[msg("Invalid oracle")]
    InvalidOracle,
    #[msg("Invalid participants")]
    InvalidParticipants,
    #[msg("Invalid winner")]
    InvalidWinner,
    #[msg("Arithmetic overflow")]
    Overflow,
    #[msg("Insufficient funds")]
    InsufficientFunds,
}
