#![allow(unexpected_cfgs)]
use anchor_lang::prelude::*;
use anchor_lang::system_program;

declare_id!("FW179MQnyxoKnoehwAqj3mpQhS7SSyyVujGstx3BQNMd");

#[program]
pub mod two_party_betting {
    use super::*;

    /// Join function - requires both participants to sign in same transaction
    pub fn join(ctx: Context<JoinCtx>, delay: u64, wager: u64) -> Result<()> {
        let bet_info = &mut ctx.accounts.bet_info;
        let participant1 = &mut ctx.accounts.participant1;
        let participant2 = &mut ctx.accounts.participant2;
        let oracle = &ctx.accounts.oracle;
        let system_program = &ctx.accounts.system_program;

        // Validate wager amount
        require!(wager > 0, BettingError::InvalidWager);
        
        // Validate delay
        require!(delay > 0, BettingError::InvalidDelay);

        // Get current slot
        let clock = Clock::get()?;
        let current_slot = clock.slot;
        
        // Calculate deadline
        let deadline = current_slot.checked_add(delay)
            .ok_or(BettingError::ArithmeticOverflow)?;

        // Initialize bet info
        bet_info.participant1 = participant1.key();
        bet_info.participant2 = participant2.key();
        bet_info.oracle = oracle.key();
        bet_info.wager = wager;
        bet_info.deadline = deadline;
        bet_info.state = BetState::Active;
        bet_info.total_pot = wager.checked_mul(2)
            .ok_or(BettingError::ArithmeticOverflow)?;

        // Transfer wager from participant1 to PDA
        let transfer_instruction1 = system_program::Transfer {
            from: participant1.to_account_info(),
            to: bet_info.to_account_info(),
        };
        let cpi_ctx1 = CpiContext::new(system_program.to_account_info(), transfer_instruction1);
        system_program::transfer(cpi_ctx1, wager)?;

        // Transfer wager from participant2 to PDA
        let transfer_instruction2 = system_program::Transfer {
            from: participant2.to_account_info(),
            to: bet_info.to_account_info(),
        };
        let cpi_ctx2 = CpiContext::new(system_program.to_account_info(), transfer_instruction2);
        system_program::transfer(cpi_ctx2, wager)?;

        emit!(BetCreated {
            participant1: participant1.key(),
            participant2: participant2.key(),
            oracle: oracle.key(),
            wager,
            deadline,
        });

        Ok(())
    }

    /// Win function - only callable by oracle before deadline
    pub fn win(ctx: Context<WinCtx>) -> Result<()> {
        let bet_info = &mut ctx.accounts.bet_info;
        let oracle = &ctx.accounts.oracle;
        let winner = &mut ctx.accounts.winner;

        // Validate oracle is the designated one
        require!(oracle.key() == bet_info.oracle, BettingError::InvalidOracle);
        
        // Validate bet is still active
        require!(bet_info.state == BetState::Active, BettingError::BetAlreadyResolved);
        
        // Validate winner is one of the participants
        require!(
            winner.key() == bet_info.participant1 || winner.key() == bet_info.participant2,
            BettingError::InvalidWinner
        );
        
        // Check deadline hasn't passed
        let clock = Clock::get()?;
        require!(clock.slot <= bet_info.deadline, BettingError::DeadlinePassed);

        // Calculate total pot (should be 2 * wager, but we use stored value for safety)
        let total_pot = bet_info.total_pot;
        
        // Transfer lamports directly from PDA to winner
        let bet_info_lamports = bet_info.to_account_info().lamports();
        let rent_exempt_minimum = Rent::get()?.minimum_balance(BetInfo::LEN);
        let available_lamports = bet_info_lamports.checked_sub(rent_exempt_minimum)
            .ok_or(BettingError::InsufficientFunds)?;
        
        // Ensure we have enough lamports for the total pot
        require!(available_lamports >= total_pot, BettingError::InsufficientFunds);
        
        // Transfer lamports manually
        **bet_info.to_account_info().try_borrow_mut_lamports()? -= total_pot;
        **winner.to_account_info().try_borrow_mut_lamports()? += total_pot;

        // Mark bet as resolved
        bet_info.state = BetState::Resolved;

        emit!(BetResolved {
            winner: winner.key(),
            amount: total_pot,
            resolved_by: oracle.key(),
        });

        Ok(())
    }

    /// Timeout function - callable by either participant after deadline
    pub fn timeout(ctx: Context<TimeoutCtx>) -> Result<()> {
        let bet_info = &mut ctx.accounts.bet_info;
        let participant1 = &mut ctx.accounts.participant1;
        let participant2 = &mut ctx.accounts.participant2;

        // Validate bet is still active
        require!(bet_info.state == BetState::Active, BettingError::BetAlreadyResolved);
        
        // Check deadline has passed
        let clock = Clock::get()?;
        require!(clock.slot > bet_info.deadline, BettingError::DeadlineNotReached);

        // Get individual wager amount
        let wager = bet_info.wager;
        
        // Calculate available lamports
        let bet_info_lamports = bet_info.to_account_info().lamports();
        let rent_exempt_minimum = Rent::get()?.minimum_balance(BetInfo::LEN);
        let available_lamports = bet_info_lamports.checked_sub(rent_exempt_minimum)
            .ok_or(BettingError::InsufficientFunds)?;
        
        // Ensure we have enough lamports for both wagers
        let total_refund = wager.checked_mul(2)
            .ok_or(BettingError::ArithmeticOverflow)?;
        require!(available_lamports >= total_refund, BettingError::InsufficientFunds);

        // Return wager to participant1
        **bet_info.to_account_info().try_borrow_mut_lamports()? -= wager;
        **participant1.to_account_info().try_borrow_mut_lamports()? += wager;

        // Return wager to participant2  
        **bet_info.to_account_info().try_borrow_mut_lamports()? -= wager;
        **participant2.to_account_info().try_borrow_mut_lamports()? += wager;

        // Mark bet as resolved
        bet_info.state = BetState::Resolved;

        emit!(BetTimedOut {
            participant1: participant1.key(),
            participant2: participant2.key(),
            wager,
        });

        Ok(())
    }
}

#[derive(Accounts)]
pub struct JoinCtx<'info> {
    #[account(mut)]
    pub participant1: Signer<'info>,
    
    #[account(mut)]
    pub participant2: Signer<'info>,
    
    /// CHECK: Oracle account is stored for later validation but doesn't need to sign join
    pub oracle: UncheckedAccount<'info>,
    
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
    /// CHECK: Winner validation is done in instruction logic
    pub winner: UncheckedAccount<'info>,
    
    #[account(
        mut,
        seeds = [participant1.key().as_ref(), participant2.key().as_ref()],
        bump
    )]
    pub bet_info: Account<'info, BetInfo>,
    
    /// CHECK: Used only for PDA derivation
    pub participant1: UncheckedAccount<'info>,
    
    /// CHECK: Used only for PDA derivation
    pub participant2: UncheckedAccount<'info>,
    
    pub system_program: Program<'info, System>,
}

#[derive(Accounts)]
pub struct TimeoutCtx<'info> {
    #[account(
        mut,
        constraint = participant1.key() == bet_info.participant1 || participant1.key() == bet_info.participant2 @ BettingError::InvalidParticipant
    )]
    pub participant1: Signer<'info>,
    
    #[account(mut)]
    /// CHECK: Validated against bet_info stored keys
    pub participant2: UncheckedAccount<'info>,
    
    #[account(
        mut,
        seeds = [bet_info.participant1.as_ref(), bet_info.participant2.as_ref()],
        bump,
        constraint = participant2.key() == bet_info.participant1 || participant2.key() == bet_info.participant2 @ BettingError::InvalidParticipant
    )]
    pub bet_info: Account<'info, BetInfo>,
    
    pub system_program: Program<'info, System>,
}

#[account]
pub struct BetInfo {
    pub participant1: Pubkey,      // 32 bytes
    pub participant2: Pubkey,      // 32 bytes
    pub oracle: Pubkey,            // 32 bytes
    pub wager: u64,                // 8 bytes
    pub total_pot: u64,            // 8 bytes
    pub deadline: u64,             // 8 bytes
    pub state: BetState,           // 1 byte
}

impl BetInfo {
    pub const LEN: usize = 8 + 32 + 32 + 32 + 8 + 8 + 8 + 1; // 129 bytes
}

#[derive(AnchorSerialize, AnchorDeserialize, Clone, PartialEq, Eq)]
pub enum BetState {
    Active,
    Resolved,
}

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
    pub winner: Pubkey,
    pub amount: u64,
    pub resolved_by: Pubkey,
}

#[event]
pub struct BetTimedOut {
    pub participant1: Pubkey,
    pub participant2: Pubkey,
    pub wager: u64,
}

#[error_code]
pub enum BettingError {
    #[msg("Invalid wager amount")]
    InvalidWager,
    
    #[msg("Invalid delay")]
    InvalidDelay,
    
    #[msg("Arithmetic overflow")]
    ArithmeticOverflow,
    
    #[msg("Invalid oracle")]
    InvalidOracle,
    
    #[msg("Bet already resolved")]
    BetAlreadyResolved,
    
    #[msg("Invalid winner")]
    InvalidWinner,
    
    #[msg("Deadline has passed")]
    DeadlinePassed,
    
    #[msg("Deadline not reached")]
    DeadlineNotReached,
    
    #[msg("Invalid participant")]
    InvalidParticipant,
    
    #[msg("Insufficient funds")]
    InsufficientFunds,
}
