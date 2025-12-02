use anchor_lang::prelude::*;
use anchor_lang::system_program;

declare_id!("F3bivXSQDphVSSwWk23iEyX1GFouJnpX83gLHfwFgSRe");

#[program]
pub mod betting_contract {
    use super::*;

    /// Join instruction - both participants must call this in the same transaction
    /// Creates a new bet with equal wagers from both participants
    pub fn join(ctx: Context<JoinCtx>, delay: u64, wager: u64) -> Result<()> {
        let bet_info = &mut ctx.accounts.bet_info;
        let participant1 = &mut ctx.accounts.participant1;
        let participant2 = &mut ctx.accounts.participant2;
        let oracle = &ctx.accounts.oracle;

        // Validate minimum wager
        require!(wager > 0, BettingError::InvalidWager);

        // Validate delay
        require!(delay > 0, BettingError::InvalidDelay);

        // Check that participants have sufficient funds
        require!(
            participant1.lamports() >= wager,
            BettingError::InsufficientFunds
        );
        require!(
            participant2.lamports() >= wager,
            BettingError::InsufficientFunds
        );

        // Ensure participants are different
        require!(
            participant1.key() != participant2.key(),
            BettingError::SameParticipant
        );

        // Get current slot for deadline calculation
        let clock = Clock::get()?;
        let deadline = clock.slot + delay;

        // Initialize bet info
        bet_info.participant1 = participant1.key();
        bet_info.participant2 = participant2.key();
        bet_info.oracle = oracle.key();
        bet_info.wager = wager;
        bet_info.deadline = deadline;
        bet_info.is_active = true;

        // Transfer wager from participant1 to PDA
        let transfer_instruction_1 = system_program::Transfer {
            from: participant1.to_account_info(),
            to: bet_info.to_account_info(),
        };
        let cpi_ctx_1 = CpiContext::new(
            ctx.accounts.system_program.to_account_info(),
            transfer_instruction_1,
        );
        system_program::transfer(cpi_ctx_1, wager)?;

        // Transfer wager from participant2 to PDA
        let transfer_instruction_2 = system_program::Transfer {
            from: participant2.to_account_info(),
            to: bet_info.to_account_info(),
        };
        let cpi_ctx_2 = CpiContext::new(
            ctx.accounts.system_program.to_account_info(),
            transfer_instruction_2,
        );
        system_program::transfer(cpi_ctx_2, wager)?;

        emit!(BetCreated {
            participant1: participant1.key(),
            participant2: participant2.key(),
            oracle: oracle.key(),
            wager,
            deadline,
        });

        Ok(())
    }

    /// Win instruction - only the designated oracle can call this
    /// Transfers entire pot to the specified winner
    pub fn win(ctx: Context<WinCtx>) -> Result<()> {
        let bet_info = &mut ctx.accounts.bet_info;
        let oracle = &ctx.accounts.oracle;
        let winner = &mut ctx.accounts.winner;

        // Verify that the bet is still active
        require!(bet_info.is_active, BettingError::BetNotActive);

        // Verify that the oracle is the designated one
        require!(
            oracle.key() == bet_info.oracle,
            BettingError::UnauthorizedOracle
        );

        // Verify that the winner is one of the participants
        require!(
            winner.key() == bet_info.participant1 || winner.key() == bet_info.participant2,
            BettingError::InvalidWinner
        );

        // Check deadline hasn't passed
        let clock = Clock::get()?;
        require!(
            clock.slot <= bet_info.deadline,
            BettingError::DeadlinePassed
        );

        // Calculate total pot (2 * wager)
        let total_pot = bet_info.wager * 2;

        // Transfer entire pot to winner
        **bet_info.to_account_info().try_borrow_mut_lamports()? -= total_pot;
        **winner.to_account_info().try_borrow_mut_lamports()? += total_pot;

        // Mark bet as inactive
        bet_info.is_active = false;

        emit!(BetWon {
            winner: winner.key(),
            amount: total_pot,
        });

        Ok(())
    }

    /// Timeout instruction - either participant can call this after deadline
    /// Returns original wagers to both participants
    pub fn timeout(ctx: Context<TimeoutCtx>) -> Result<()> {
        let bet_info = &mut ctx.accounts.bet_info;
        let participant1 = &mut ctx.accounts.participant1;
        let participant2 = &mut ctx.accounts.participant2;

        // Verify that the bet is still active
        require!(bet_info.is_active, BettingError::BetNotActive);

        // Verify that the deadline has passed
        let clock = Clock::get()?;
        require!(
            clock.slot > bet_info.deadline,
            BettingError::DeadlineNotReached
        );

        // Return wager to participant1
        **bet_info.to_account_info().try_borrow_mut_lamports()? -= bet_info.wager;
        **participant1.to_account_info().try_borrow_mut_lamports()? += bet_info.wager;

        // Return wager to participant2
        **bet_info.to_account_info().try_borrow_mut_lamports()? -= bet_info.wager;
        **participant2.to_account_info().try_borrow_mut_lamports()? += bet_info.wager;

        // Mark bet as inactive
        bet_info.is_active = false;

        emit!(BetTimedOut {
            participant1: participant1.key(),
            participant2: participant2.key(),
            refunded_amount: bet_info.wager,
        });

        Ok(())
    }
}

// Context structs for each instruction

#[derive(Accounts)]
pub struct JoinCtx<'info> {
    #[account(mut)]
    pub participant1: Signer<'info>,
    
    #[account(mut)]
    pub participant2: Signer<'info>,
    
    /// CHECK: Oracle account is stored for reference but doesn't need to sign
    pub oracle: UncheckedAccount<'info>,
    
    #[account(
        init,
        payer = participant1,
        space = BetInfo::SIZE,
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
    pub participant1: UncheckedAccount<'info>,
    
    /// CHECK: Used for PDA derivation only
    pub participant2: UncheckedAccount<'info>,
    
    pub system_program: Program<'info, System>,
}

#[derive(Accounts)]
pub struct TimeoutCtx<'info> {
    #[account(
        mut,
        constraint = participant1.key() == bet_info.participant1 || 
                    participant2.key() == bet_info.participant1 @ BettingError::InvalidParticipant
    )]
    pub participant1: SystemAccount<'info>,
    
    #[account(
        mut,
        constraint = participant2.key() == bet_info.participant2 || 
                    participant1.key() == bet_info.participant2 @ BettingError::InvalidParticipant
    )]
    pub participant2: SystemAccount<'info>,
    
    #[account(
        mut,
        seeds = [bet_info.participant1.as_ref(), bet_info.participant2.as_ref()],
        bump
    )]
    pub bet_info: Account<'info, BetInfo>,
    
    pub system_program: Program<'info, System>,
}

// Account data structure

#[account]
pub struct BetInfo {
    pub participant1: Pubkey,      // 32 bytes
    pub participant2: Pubkey,      // 32 bytes
    pub oracle: Pubkey,            // 32 bytes
    pub wager: u64,                // 8 bytes
    pub deadline: u64,             // 8 bytes
    pub is_active: bool,           // 1 byte
}

impl BetInfo {
    pub const SIZE: usize = 8 + 32 + 32 + 32 + 8 + 8 + 1; // discriminator + fields = 121 bytes
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
pub struct BetWon {
    pub winner: Pubkey,
    pub amount: u64,
}

#[event]
pub struct BetTimedOut {
    pub participant1: Pubkey,
    pub participant2: Pubkey,
    pub refunded_amount: u64,
}

// Custom error types

#[error_code]
pub enum BettingError {
    #[msg("Invalid wager amount")]
    InvalidWager,
    
    #[msg("Invalid delay amount")]
    InvalidDelay,
    
    #[msg("Insufficient funds")]
    InsufficientFunds,
    
    #[msg("Participants cannot be the same")]
    SameParticipant,
    
    #[msg("Bet is not active")]
    BetNotActive,
    
    #[msg("Unauthorized oracle")]
    UnauthorizedOracle,
    
    #[msg("Invalid winner")]
    InvalidWinner,
    
    #[msg("Deadline has passed")]
    DeadlinePassed,
    
    #[msg("Deadline has not been reached")]
    DeadlineNotReached,
    
    #[msg("Invalid participant")]
    InvalidParticipant,
}
