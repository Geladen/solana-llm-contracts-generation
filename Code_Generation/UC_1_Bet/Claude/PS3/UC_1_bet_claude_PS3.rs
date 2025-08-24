#![allow(unexpected_cfgs)]
use anchor_lang::prelude::*;
use anchor_lang::system_program;

declare_id!("F3bivXSQDphVSSwWk23iEyX1GFouJnpX83gLHfwFgSRe");

#[program]
pub mod betting_contract {
    use super::*;

    /// Join function - both participants must sign and deposit in same transaction
    pub fn join(ctx: Context<JoinCtx>, delay: u64, wager: u64) -> Result<()> {
        let clock = Clock::get()?;
        
        // Validate wager amount
        require!(wager > 0, BettingError::InvalidWager);
        
        // Validate delay
        require!(delay > 0, BettingError::InvalidDelay);
        
        // Initialize bet info
        ctx.accounts.bet_info.participant1 = ctx.accounts.participant1.key();
        ctx.accounts.bet_info.participant2 = ctx.accounts.participant2.key();
        ctx.accounts.bet_info.oracle = ctx.accounts.oracle.key();
        ctx.accounts.bet_info.wager = wager;
        ctx.accounts.bet_info.deadline = clock.slot + delay;
        ctx.accounts.bet_info.is_resolved = false;
        
        // Transfer wager from participant1 to PDA
        let transfer_instruction_1 = system_program::Transfer {
            from: ctx.accounts.participant1.to_account_info(),
            to: ctx.accounts.bet_info.to_account_info(),
        };
        let cpi_context_1 = CpiContext::new(
            ctx.accounts.system_program.to_account_info(),
            transfer_instruction_1,
        );
        system_program::transfer(cpi_context_1, wager)?;
        
        // Transfer wager from participant2 to PDA
        let transfer_instruction_2 = system_program::Transfer {
            from: ctx.accounts.participant2.to_account_info(),
            to: ctx.accounts.bet_info.to_account_info(),
        };
        let cpi_context_2 = CpiContext::new(
            ctx.accounts.system_program.to_account_info(),
            transfer_instruction_2,
        );
        system_program::transfer(cpi_context_2, wager)?;
        
        Ok(())
    }

    /// Win function - only oracle can call, transfers entire pot to winner
    pub fn win(ctx: Context<WinCtx>) -> Result<()> {
        let clock = Clock::get()?;
        
        // Validate oracle
        require!(
            ctx.accounts.oracle.key() == ctx.accounts.bet_info.oracle,
            BettingError::InvalidOracle
        );
        
        // Validate bet is not resolved
        require!(!ctx.accounts.bet_info.is_resolved, BettingError::BetAlreadyResolved);
        
        // Validate deadline has not passed
        require!(
            clock.slot <= ctx.accounts.bet_info.deadline,
            BettingError::DeadlinePassed
        );
        
        // Validate winner is one of the participants
        let winner_key = ctx.accounts.winner.key();
        require!(
            winner_key == ctx.accounts.bet_info.participant1 || winner_key == ctx.accounts.bet_info.participant2,
            BettingError::InvalidWinner
        );
        
        // Calculate total pot (2 * wager)
        let total_pot = ctx.accounts.bet_info.wager * 2;
        
        // Manual transfer: subtract from PDA, add to winner
        **ctx.accounts.bet_info.to_account_info().try_borrow_mut_lamports()? -= total_pot;
        **ctx.accounts.winner.to_account_info().try_borrow_mut_lamports()? += total_pot;
        
        // Mark bet as resolved
        ctx.accounts.bet_info.is_resolved = true;
        
        Ok(())
    }

    /// Timeout function - returns original wagers to participants after deadline
    pub fn timeout(ctx: Context<TimeoutCtx>) -> Result<()> {
        let clock = Clock::get()?;
        
        // Validate bet is not resolved
        require!(!ctx.accounts.bet_info.is_resolved, BettingError::BetAlreadyResolved);
        
        // Validate deadline has been reached
        require!(
            clock.slot > ctx.accounts.bet_info.deadline,
            BettingError::DeadlineNotReached
        );
        
        // Store wager amount
        let wager = ctx.accounts.bet_info.wager;
        
        // Manual transfers: subtract from PDA, add to participants
        **ctx.accounts.bet_info.to_account_info().try_borrow_mut_lamports()? -= wager * 2;
        **ctx.accounts.participant1.to_account_info().try_borrow_mut_lamports()? += wager;
        **ctx.accounts.participant2.to_account_info().try_borrow_mut_lamports()? += wager;
        
        // Mark bet as resolved
        ctx.accounts.bet_info.is_resolved = true;
        
        Ok(())
    }
}

// Context structs
#[derive(Accounts)]
pub struct JoinCtx<'info> {
    #[account(mut)]
    pub participant1: Signer<'info>,
    #[account(mut)]
    pub participant2: Signer<'info>,
    /// CHECK: Oracle account for storage reference only
    pub oracle: AccountInfo<'info>,
    #[account(
        init,
        payer = participant1,
        space = BetInfo::LEN,
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
    #[account(
        mut,
        seeds = [participant1.key().as_ref(), participant2.key().as_ref()],
        bump
    )]
    pub bet_info: Account<'info, BetInfo>,
    /// CHECK: Used for PDA derivation only
    pub participant1: AccountInfo<'info>,
    /// CHECK: Used for PDA derivation only
    pub participant2: AccountInfo<'info>,
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
        bump
    )]
    pub bet_info: Account<'info, BetInfo>,
    pub system_program: Program<'info, System>,
}

// Account data structure
#[account]
pub struct BetInfo {
    pub participant1: Pubkey,    // 32 bytes
    pub participant2: Pubkey,    // 32 bytes
    pub oracle: Pubkey,          // 32 bytes
    pub wager: u64,              // 8 bytes
    pub deadline: u64,           // 8 bytes
    pub is_resolved: bool,       // 1 byte
}

impl BetInfo {
    pub const LEN: usize = 8 + 32 + 32 + 32 + 8 + 8 + 1; // 121 bytes total (8 bytes discriminator + data)
}

// Error definitions
#[error_code]
pub enum BettingError {
    #[msg("Invalid wager amount")]
    InvalidWager,
    #[msg("Invalid delay")]
    InvalidDelay,
    #[msg("Invalid oracle")]
    InvalidOracle,
    #[msg("Bet already resolved")]
    BetAlreadyResolved,
    #[msg("Deadline has passed")]
    DeadlinePassed,
    #[msg("Deadline not reached")]
    DeadlineNotReached,
    #[msg("Invalid winner")]
    InvalidWinner,
}
