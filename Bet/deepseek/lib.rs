#![allow(unexpected_cfgs)]

use anchor_lang::prelude::*;
use anchor_lang::system_program::{transfer, Transfer};

declare_id!("4CU7PgH4PyReJgi8iqutbyAax8tYepEdicYPsMWbYGuV");

#[program]
pub mod betting_contract {
    use super::*;

    #[derive(Accounts)]
    pub struct Initialize<'info> {
        #[account(mut)]
        pub admin: Signer<'info>,
        #[account(
            init,
            payer = admin,
            space = 8 + BetState::INIT_SPACE,
            seeds = [b"bet_state"],
            bump
        )]
        pub bet_state: Account<'info, BetState>,
        pub system_program: Program<'info, System>,
    }

    pub fn initialize(ctx: Context<Initialize>, oracle: Pubkey, deadline: i64) -> Result<()> {
        ctx.accounts.bet_state.oracle = oracle;
        ctx.accounts.bet_state.deadline = deadline;
        Ok(())
    }

    #[derive(Accounts)]
    pub struct Join<'info> {
        #[account(mut)]
        pub player: Signer<'info>,
        #[account(
            mut,
            seeds = [b"bet_state"],
            bump
        )]
        pub bet_state: Account<'info, BetState>,
        pub system_program: Program<'info, System>,
    }

    pub fn join(ctx: Context<Join>, amount: u64) -> Result<()> {
        require!(ctx.accounts.bet_state.player1 == Pubkey::default() || 
                 ctx.accounts.bet_state.player2 == Pubkey::default(), 
                 BettingError::GameFull);

        let cpi_accounts = Transfer {
            from: ctx.accounts.player.to_account_info(),
            to: ctx.accounts.bet_state.to_account_info(),
        };
        let cpi_ctx = CpiContext::new(ctx.accounts.system_program.to_account_info(), cpi_accounts);
        transfer(cpi_ctx, amount)?;

        if ctx.accounts.bet_state.player1 == Pubkey::default() {
            ctx.accounts.bet_state.player1 = ctx.accounts.player.key();
            ctx.accounts.bet_state.amount1 = amount;
        } else {
            ctx.accounts.bet_state.player2 = ctx.accounts.player.key();
            ctx.accounts.bet_state.amount2 = amount;
        }

        Ok(())
    }

    #[derive(Accounts)]
    pub struct Win<'info> {
        #[account(mut)]
        pub oracle: Signer<'info>,
        #[account(
            mut,
            seeds = [b"bet_state"],
            bump
        )]
        pub bet_state: Account<'info, BetState>,
        #[account(mut)]
        pub winner: SystemAccount<'info>,
        pub system_program: Program<'info, System>,
    }

    pub fn win(ctx: Context<Win>, winner: Pubkey) -> Result<()> {
        require!(ctx.accounts.oracle.key() == ctx.accounts.bet_state.oracle, BettingError::InvalidOracle);
        require!(ctx.accounts.bet_state.player1 != Pubkey::default() && 
                 ctx.accounts.bet_state.player2 != Pubkey::default(), 
                 BettingError::PlayersNotReady);
        require!(ctx.accounts.bet_state.amount1 == ctx.accounts.bet_state.amount2, 
                 BettingError::UnequalDeposits);
        require!(winner == ctx.accounts.bet_state.player1 || 
                 winner == ctx.accounts.bet_state.player2, 
                 BettingError::InvalidWinner);

        let amount = ctx.accounts.bet_state.amount1 + ctx.accounts.bet_state.amount2;
        **ctx.accounts.bet_state.to_account_info().try_borrow_mut_lamports()? -= amount;
        **ctx.accounts.winner.to_account_info().try_borrow_mut_lamports()? += amount;

        ctx.accounts.bet_state.player1 = Pubkey::default();
        ctx.accounts.bet_state.player2 = Pubkey::default();

        Ok(())
    }

    #[derive(Accounts)]
    pub struct Timeout<'info> {
        #[account(
            mut,
            seeds = [b"bet_state"],
            bump
        )]
        pub bet_state: Account<'info, BetState>,
        #[account(mut)]
        pub player1: SystemAccount<'info>,
        #[account(mut)]
        pub player2: SystemAccount<'info>,
        pub system_program: Program<'info, System>,
    }

    pub fn timeout(ctx: Context<Timeout>) -> Result<()> {
        require!(Clock::get()?.unix_timestamp > ctx.accounts.bet_state.deadline, BettingError::DeadlineNotPassed);
        require!(ctx.accounts.bet_state.player1 != Pubkey::default() && 
                 ctx.accounts.bet_state.player2 != Pubkey::default(), 
                 BettingError::PlayersNotReady);
        require!(ctx.accounts.bet_state.amount1 == ctx.accounts.bet_state.amount2, 
                 BettingError::UnequalDeposits);

        let amount = ctx.accounts.bet_state.amount1;
        **ctx.accounts.bet_state.to_account_info().try_borrow_mut_lamports()? -= amount * 2;
        **ctx.accounts.player1.to_account_info().try_borrow_mut_lamports()? += amount;
        **ctx.accounts.player2.to_account_info().try_borrow_mut_lamports()? += amount;

        ctx.accounts.bet_state.player1 = Pubkey::default();
        ctx.accounts.bet_state.player2 = Pubkey::default();

        Ok(())
    }
    
    pub fn close(ctx: Context<Close>) -> Result<()> {
        Ok(())
    }

    #[derive(Accounts)]
    pub struct Close<'info> {
        #[account(mut, close = admin)]
        pub bet_state: Account<'info, BetState>,
        #[account(mut)]
        pub admin: Signer<'info>,
    }
        
}

#[account]
#[derive(InitSpace)]
pub struct BetState {
    pub oracle: Pubkey,
    pub deadline: i64,
    pub player1: Pubkey,
    pub player2: Pubkey,
    pub amount1: u64,
    pub amount2: u64,
}

#[error_code]
pub enum BettingError {
    #[msg("Invalid oracle signature")]
    InvalidOracle,
    #[msg("Both players must join before resolution")]
    PlayersNotReady,
    #[msg("Deposit amounts must be equal")]
    UnequalDeposits,
    #[msg("Deadline has not passed yet")]
    DeadlineNotPassed,
    #[msg("Game already has two players")]
    GameFull,
    #[msg("Winner must be one of the players")]
    InvalidWinner,
}
