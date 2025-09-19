use anchor_lang::prelude::*;
use anchor_lang::system_program;

declare_id!("68DBMjPpJVtsPdG1zaLbveAyZC4F16e4QGmpM5J5ynyD");

#[program]
pub mod betting_contract {
    use super::*;

    pub fn join(ctx: Context<JoinCtx>, delay: u64, wager: u64) -> Result<()> {
        let clock = Clock::get()?;
        
        // Validate wager amount
        require!(wager > 0, BettingError::InvalidWager);

        // Store keys for later use
        let participant1_key = ctx.accounts.participant1.key();
        let participant2_key = ctx.accounts.participant2.key();
        let oracle_key = ctx.accounts.oracle.key();
        let bet_key = ctx.accounts.bet_info.key();
        let deadline = clock.slot + delay;
        let pot = wager * 2;
        
        // Transfer wager from participant1 to PDA
        let transfer_ix1 = system_program::Transfer {
            from: ctx.accounts.participant1.to_account_info(),
            to: ctx.accounts.bet_info.to_account_info(),
        };
        system_program::transfer(
            CpiContext::new(
                ctx.accounts.system_program.to_account_info(),
                transfer_ix1,
            ),
            wager,
        )?;

        // Transfer wager from participant2 to PDA
        let transfer_ix2 = system_program::Transfer {
            from: ctx.accounts.participant2.to_account_info(),
            to: ctx.accounts.bet_info.to_account_info(),
        };
        system_program::transfer(
            CpiContext::new(
                ctx.accounts.system_program.to_account_info(),
                transfer_ix2,
            ),
            wager,
        )?;

        // Initialize bet info after transfers
        let bet_info = &mut ctx.accounts.bet_info;
        bet_info.participant1 = participant1_key;
        bet_info.participant2 = participant2_key;
        bet_info.oracle = oracle_key;
        bet_info.wager = wager;
        bet_info.deadline = deadline;
        bet_info.pot = pot;
        bet_info.is_active = true;

        emit!(BetCreated {
            bet_id: bet_key,
            participant1: participant1_key,
            participant2: participant2_key,
            oracle: oracle_key,
            wager,
            deadline,
            pot,
        });

        Ok(())
    }

    pub fn win(ctx: Context<WinCtx>) -> Result<()> {
        let clock = Clock::get()?;

        // Store values before borrowing mutably
        let bet_key = ctx.accounts.bet_info.key();
        let is_active = ctx.accounts.bet_info.is_active;
        let deadline = ctx.accounts.bet_info.deadline;
        let oracle_key = ctx.accounts.bet_info.oracle;
        let participant1_key = ctx.accounts.bet_info.participant1;
        let participant2_key = ctx.accounts.bet_info.participant2;
        let pot_amount = ctx.accounts.bet_info.pot;
        let winner_key = ctx.accounts.winner.key();

        // Validate bet is still active
        require!(is_active, BettingError::BetNotActive);
        
        // Validate deadline hasn't passed
        require!(clock.slot <= deadline, BettingError::BetExpired);
        
        // Validate oracle is calling this function
        require!(
            ctx.accounts.oracle.key() == oracle_key,
            BettingError::UnauthorizedOracle
        );

        // Validate winner is one of the participants
        require!(
            winner_key == participant1_key || winner_key == participant2_key,
            BettingError::InvalidWinner
        );

        // Transfer entire pot to winner
        **ctx.accounts.bet_info.to_account_info().try_borrow_mut_lamports()? -= pot_amount;
        **ctx.accounts.winner.to_account_info().try_borrow_mut_lamports()? += pot_amount;

        // Mark bet as inactive
        let bet_info = &mut ctx.accounts.bet_info;
        bet_info.is_active = false;
        bet_info.winner = Some(winner_key);

        emit!(BetWon {
            bet_id: bet_key,
            winner: winner_key,
            pot: pot_amount,
        });

        Ok(())
    }

    pub fn timeout(ctx: Context<TimeoutCtx>) -> Result<()> {
        let clock = Clock::get()?;

        // Store values before borrowing mutably
        let bet_key = ctx.accounts.bet_info.key();
        let is_active = ctx.accounts.bet_info.is_active;
        let deadline = ctx.accounts.bet_info.deadline;
        let wager_amount = ctx.accounts.bet_info.wager;
        let participant1_key = ctx.accounts.bet_info.participant1;
        let participant2_key = ctx.accounts.bet_info.participant2;

        // Validate bet is still active
        require!(is_active, BettingError::BetNotActive);
        
        // Validate deadline has passed
        require!(clock.slot > deadline, BettingError::BetNotExpired);

        // Return wager to participant1
        **ctx.accounts.bet_info.to_account_info().try_borrow_mut_lamports()? -= wager_amount;
        **ctx.accounts.participant1.to_account_info().try_borrow_mut_lamports()? += wager_amount;

        // Return wager to participant2
        **ctx.accounts.bet_info.to_account_info().try_borrow_mut_lamports()? -= wager_amount;
        **ctx.accounts.participant2.to_account_info().try_borrow_mut_lamports()? += wager_amount;

        // Mark bet as inactive
        let bet_info = &mut ctx.accounts.bet_info;
        bet_info.is_active = false;

        emit!(BetTimedOut {
            bet_id: bet_key,
            participant1: participant1_key,
            participant2: participant2_key,
            refunded_amount: wager_amount,
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
    
    /// CHECK: Oracle address is stored for reference, not used as signer here
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
    pub oracle: Signer<'info>,
    
    #[account(mut)]
    /// CHECK: Winner validation is done in the instruction logic
    pub winner: UncheckedAccount<'info>,
    
    #[account(
        mut,
        seeds = [participant1.key().as_ref(), participant2.key().as_ref()],
        bump,
        constraint = bet_info.oracle == oracle.key() @ BettingError::UnauthorizedOracle
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
    #[account(mut)]
    /// CHECK: Participant1 address validation done through PDA constraint
    pub participant1: UncheckedAccount<'info>,
    
    #[account(mut)]
    /// CHECK: Participant2 address validation done through PDA constraint
    pub participant2: UncheckedAccount<'info>,
    
    #[account(
        mut,
        seeds = [participant1.key().as_ref(), participant2.key().as_ref()],
        bump,
        constraint = bet_info.participant1 == participant1.key() @ BettingError::InvalidParticipant,
        constraint = bet_info.participant2 == participant2.key() @ BettingError::InvalidParticipant
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
    pub pot: u64,
    pub is_active: bool,
    pub winner: Option<Pubkey>,
}

impl BetInfo {
    pub const SPACE: usize = 8 + // discriminator
        32 + // participant1
        32 + // participant2
        32 + // oracle
        8 +  // wager
        8 +  // deadline
        8 +  // pot
        1 +  // is_active
        (1 + 32); // winner (Option<Pubkey>)
}

#[event]
pub struct BetCreated {
    pub bet_id: Pubkey,
    pub participant1: Pubkey,
    pub participant2: Pubkey,
    pub oracle: Pubkey,
    pub wager: u64,
    pub deadline: u64,
    pub pot: u64,
}

#[event]
pub struct BetWon {
    pub bet_id: Pubkey,
    pub winner: Pubkey,
    pub pot: u64,
}

#[event]
pub struct BetTimedOut {
    pub bet_id: Pubkey,
    pub participant1: Pubkey,
    pub participant2: Pubkey,
    pub refunded_amount: u64,
}

#[error_code]
pub enum BettingError {
    #[msg("Invalid wager amount")]
    InvalidWager,
    #[msg("Bet is not active")]
    BetNotActive,
    #[msg("Bet has expired")]
    BetExpired,
    #[msg("Bet has not expired yet")]
    BetNotExpired,
    #[msg("Unauthorized oracle")]
    UnauthorizedOracle,
    #[msg("Invalid winner")]
    InvalidWinner,
    #[msg("Invalid participant")]
    InvalidParticipant,
}
