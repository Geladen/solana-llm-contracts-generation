#![allow(unexpected_cfgs)]
use anchor_lang::prelude::*;
use anchor_lang::system_program::{self, Transfer};

declare_id!("68DBMjPpJVtsPdG1zaLbveAyZC4F16e4QGmpM5J5ynyD");

#[program]
pub mod two_party_betting {
    use super::*;

    /// Join function - Both participants must join in the same transaction
    pub fn join(ctx: Context<JoinCtx>, delay: u64, wager: u64) -> Result<()> {
        let clock = Clock::get()?;
        
        // Ensure wager is greater than 0
        require!(wager > 0, BettingError::InvalidWager);
        
        // Initialize bet info
        let bet_info = &mut ctx.accounts.bet_info;
        bet_info.participant1 = ctx.accounts.participant1.key();
        bet_info.participant2 = ctx.accounts.participant2.key();
        bet_info.oracle = ctx.accounts.oracle.key();
        bet_info.wager = wager;
        bet_info.deadline = clock.slot.checked_add(delay).ok_or(BettingError::ArithmeticOverflow)?;
        bet_info.state = BetState::Active;
        bet_info.bump = ctx.bumps.bet_info;

        // Store values for event emission
        let participant1_key = bet_info.participant1;
        let participant2_key = bet_info.participant2;
        let oracle_key = bet_info.oracle;
        let deadline = bet_info.deadline;

        // Transfer wager from participant1 to bet PDA
        let transfer_instruction1 = Transfer {
            from: ctx.accounts.participant1.to_account_info(),
            to: bet_info.to_account_info(),
        };
        let cpi_context1 = CpiContext::new(
            ctx.accounts.system_program.to_account_info(),
            transfer_instruction1,
        );
        system_program::transfer(cpi_context1, wager)?;

        // Transfer wager from participant2 to bet PDA
        let transfer_instruction2 = Transfer {
            from: ctx.accounts.participant2.to_account_info(),
            to: bet_info.to_account_info(),
        };
        let cpi_context2 = CpiContext::new(
            ctx.accounts.system_program.to_account_info(),
            transfer_instruction2,
        );
        system_program::transfer(cpi_context2, wager)?;

        emit!(BetCreated {
            participant1: participant1_key,
            participant2: participant2_key,
            oracle: oracle_key,
            wager: wager,
            deadline: deadline,
        });

        Ok(())
    }

    /// Win function - Only callable by oracle before deadline
    pub fn win(ctx: Context<WinCtx>) -> Result<()> {
        let bet_info = &ctx.accounts.bet_info;
        let clock = Clock::get()?;
        
        // Ensure bet is still active
        require!(bet_info.state == BetState::Active, BettingError::BetAlreadyResolved);
        
        // Ensure deadline hasn't passed
        require!(clock.slot <= bet_info.deadline, BettingError::DeadlinePassed);
        
        // Ensure oracle is the authorized one
        require!(ctx.accounts.oracle.key() == bet_info.oracle, BettingError::UnauthorizedOracle);
        
        // Ensure winner is one of the participants
        let winner_key = ctx.accounts.winner.key();
        require!(
            winner_key == bet_info.participant1 || winner_key == bet_info.participant2,
            BettingError::InvalidWinner
        );

        // Calculate total pot (2 * wager)
        let total_pot = bet_info.wager.checked_mul(2).ok_or(BettingError::ArithmeticOverflow)?;
        
        // Store values for event emission
        let participant1_key = bet_info.participant1;
        let participant2_key = bet_info.participant2;
        
        // Transfer entire pot to winner
        let bet_account_lamports = ctx.accounts.bet_info.to_account_info().lamports();
        **ctx.accounts.bet_info.to_account_info().try_borrow_mut_lamports()? = bet_account_lamports
            .checked_sub(total_pot)
            .ok_or(BettingError::InsufficientFunds)?;
        
        **ctx.accounts.winner.try_borrow_mut_lamports()? = ctx.accounts.winner
            .lamports()
            .checked_add(total_pot)
            .ok_or(BettingError::ArithmeticOverflow)?;

        // Mark bet as resolved
        let bet_info = &mut ctx.accounts.bet_info;
        bet_info.state = BetState::Resolved;

        emit!(BetResolved {
            participant1: participant1_key,
            participant2: participant2_key,
            winner: winner_key,
            amount: total_pot,
        });

        Ok(())
    }

    /// Timeout function - Callable by either participant after deadline
    pub fn timeout(ctx: Context<TimeoutCtx>) -> Result<()> {
        let clock = Clock::get()?;
        
        // Read values first (immutable borrow)
        let wager;
        let participant1_key;
        let participant2_key;
        {
            let bet_info = &ctx.accounts.bet_info;
            
            // Ensure bet is still active
            require!(bet_info.state == BetState::Active, BettingError::BetAlreadyResolved);
            
            // Ensure deadline has passed
            require!(clock.slot > bet_info.deadline, BettingError::DeadlineNotReached);

            // Store values for calculations
            wager = bet_info.wager;
            participant1_key = bet_info.participant1;
            participant2_key = bet_info.participant2;
        }

        // Calculate refund amount
        let total_refund = wager.checked_mul(2).ok_or(BettingError::ArithmeticOverflow)?;
        
        // Transfer lamports back to participants
        let bet_account_lamports = ctx.accounts.bet_info.to_account_info().lamports();
        **ctx.accounts.bet_info.to_account_info().try_borrow_mut_lamports()? = bet_account_lamports
            .checked_sub(total_refund)
            .ok_or(BettingError::InsufficientFunds)?;
        
        **ctx.accounts.participant1.try_borrow_mut_lamports()? = ctx.accounts.participant1
            .lamports()
            .checked_add(wager)
            .ok_or(BettingError::ArithmeticOverflow)?;
            
        **ctx.accounts.participant2.try_borrow_mut_lamports()? = ctx.accounts.participant2
            .lamports()
            .checked_add(wager)
            .ok_or(BettingError::ArithmeticOverflow)?;

        // Mark bet as resolved (mutable borrow)
        ctx.accounts.bet_info.state = BetState::Resolved;

        emit!(BetTimedOut {
            participant1: participant1_key,
            participant2: participant2_key,
            refunded_amount: wager,
        });

        Ok(())
    }
}

// Account contexts
#[derive(Accounts)]
pub struct JoinCtx<'info> {
    #[account(mut)]
    pub participant1: Signer<'info>,
    
    #[account(mut)]
    pub participant2: Signer<'info>,
    
    /// CHECK: Oracle account for reference storage
    pub oracle: AccountInfo<'info>,
    
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
    /// CHECK: Winner account validated in instruction logic
    pub winner: AccountInfo<'info>,
    
    #[account(
        mut,
        seeds = [participant1.key().as_ref(), participant2.key().as_ref()],
        bump = bet_info.bump
    )]
    pub bet_info: Account<'info, BetInfo>,
    
    /// CHECK: Used for PDA derivation
    pub participant1: AccountInfo<'info>,
    
    /// CHECK: Used for PDA derivation  
    pub participant2: AccountInfo<'info>,
    
    pub system_program: Program<'info, System>,
}

#[derive(Accounts)]
pub struct TimeoutCtx<'info> {
    #[account(
        mut,
        constraint = participant1.key() == bet_info.participant1 || participant2.key() == bet_info.participant1
            @ BettingError::InvalidParticipant
    )]
    /// CHECK: Validated through constraint
    pub participant1: AccountInfo<'info>,
    
    #[account(
        mut,
        constraint = participant2.key() == bet_info.participant2 || participant1.key() == bet_info.participant2
            @ BettingError::InvalidParticipant
    )]
    /// CHECK: Validated through constraint
    pub participant2: AccountInfo<'info>,
    
    #[account(
        mut,
        seeds = [bet_info.participant1.as_ref(), bet_info.participant2.as_ref()],
        bump = bet_info.bump
    )]
    pub bet_info: Account<'info, BetInfo>,
    
    pub system_program: Program<'info, System>,
}

// Data structures
#[account]
pub struct BetInfo {
    pub participant1: Pubkey,
    pub participant2: Pubkey,
    pub oracle: Pubkey,
    pub wager: u64,
    pub deadline: u64,
    pub state: BetState,
    pub bump: u8,
}

impl BetInfo {
    pub const SPACE: usize = 8 + // discriminator
        32 + // participant1
        32 + // participant2
        32 + // oracle
        8 +  // wager
        8 +  // deadline
        1 +  // state
        1;   // bump
}

#[derive(AnchorSerialize, AnchorDeserialize, Clone, PartialEq, Eq)]
pub enum BetState {
    Active,
    Resolved,
}

// Events
#[event]
pub struct BetCreated {
    pub participant1: Pubkey,
    pub participant2: Pubkey,
    pub oracle: Pubkey,
    pub wager: u64,
    pub deadline: u64,
}

#[event]
pub struct BetResolved {
    pub participant1: Pubkey,
    pub participant2: Pubkey,
    pub winner: Pubkey,
    pub amount: u64,
}

#[event]
pub struct BetTimedOut {
    pub participant1: Pubkey,
    pub participant2: Pubkey,
    pub refunded_amount: u64,
}

// Error definitions
#[error_code]
pub enum BettingError {
    #[msg("Invalid wager amount")]
    InvalidWager,
    #[msg("Bet has already been resolved")]
    BetAlreadyResolved,
    #[msg("Deadline has passed")]
    DeadlinePassed,
    #[msg("Deadline has not been reached")]
    DeadlineNotReached,
    #[msg("Unauthorized oracle")]
    UnauthorizedOracle,
    #[msg("Invalid winner")]
    InvalidWinner,
    #[msg("Invalid participant")]
    InvalidParticipant,
    #[msg("Insufficient funds")]
    InsufficientFunds,
    #[msg("Arithmetic overflow")]
    ArithmeticOverflow,
}
