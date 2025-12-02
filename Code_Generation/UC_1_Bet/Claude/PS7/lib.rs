use anchor_lang::prelude::*;
use anchor_lang::system_program;

declare_id!("FW179MQnyxoKnoehwAqj3mpQhS7SSyyVujGstx3BQNMd");

#[program]
pub mod betting_program {
    use super::*;

    pub fn join(ctx: Context<JoinCtx>, delay: u64, wager: u64) -> Result<()> {
        let bet_info = &mut ctx.accounts.bet_info;
        let participant1 = &ctx.accounts.participant1;
        let participant2 = &ctx.accounts.participant2;
        let oracle = &ctx.accounts.oracle;
        let clock = Clock::get()?;

        // Validate wager amount
        require!(wager > 0, BettingError::InvalidWager);

        // Initialize bet info
        bet_info.participant1 = participant1.key();
        bet_info.participant2 = participant2.key();
        bet_info.oracle = oracle.key();
        bet_info.deadline = clock.slot + delay;
        bet_info.wager = wager;
        bet_info.total_pot = wager * 2;
        bet_info.is_active = true;

        // Transfer wager from participant1 to bet_info PDA
        let cpi_context = CpiContext::new(
            ctx.accounts.system_program.to_account_info(),
            system_program::Transfer {
                from: participant1.to_account_info(),
                to: bet_info.to_account_info(),
            },
        );
        system_program::transfer(cpi_context, wager)?;

        // Transfer wager from participant2 to bet_info PDA
        let cpi_context = CpiContext::new(
            ctx.accounts.system_program.to_account_info(),
            system_program::Transfer {
                from: participant2.to_account_info(),
                to: bet_info.to_account_info(),
            },
        );
        system_program::transfer(cpi_context, wager)?;

        msg!("Bet initialized with deadline: {}, wager: {}", bet_info.deadline, wager);
        Ok(())
    }

    pub fn win(ctx: Context<WinCtx>) -> Result<()> {
        let bet_info = &mut ctx.accounts.bet_info;
        let oracle = &ctx.accounts.oracle;
        let winner = &ctx.accounts.winner;
        let participant1 = &ctx.accounts.participant1;
        let participant2 = &ctx.accounts.participant2;

        // Validate oracle signature
        require!(oracle.key() == bet_info.oracle, BettingError::InvalidOracle);
        
        // Validate bet is still active
        require!(bet_info.is_active, BettingError::BetNotActive);

        // Validate winner is one of the participants
        require!(
            winner.key() == bet_info.participant1 || winner.key() == bet_info.participant2,
            BettingError::InvalidWinner
        );

        // Check deadline hasn't passed
        let clock = Clock::get()?;
        require!(clock.slot < bet_info.deadline, BettingError::DeadlinePassed);

        let total_pot = bet_info.total_pot;

        // Generate PDA seeds for signing
        let participant1_key = participant1.key();
        let participant2_key = participant2.key();
        let seeds = &[
            participant1_key.as_ref(),
            participant2_key.as_ref(),
        ];
        let (pda, bump) = Pubkey::find_program_address(seeds, ctx.program_id);
        require!(pda == bet_info.key(), BettingError::InvalidPDA);

        let signer_seeds = &[
            participant1_key.as_ref(),
            participant2_key.as_ref(),
            &[bump],
        ];

        // Transfer entire pot to winner
        **bet_info.to_account_info().try_borrow_mut_lamports()? -= total_pot;
        **winner.to_account_info().try_borrow_mut_lamports()? += total_pot;

        // Mark bet as inactive
        bet_info.is_active = false;

        msg!("Winner {} received {} lamports", winner.key(), total_pot);
        Ok(())
    }

    pub fn timeout(ctx: Context<TimeoutCtx>) -> Result<()> {
        let bet_info = &mut ctx.accounts.bet_info;
        let participant1 = &ctx.accounts.participant1;
        let participant2 = &ctx.accounts.participant2;
        let clock = Clock::get()?;

        // Validate bet is still active
        require!(bet_info.is_active, BettingError::BetNotActive);

        // Check deadline has passed
        require!(clock.slot >= bet_info.deadline, BettingError::DeadlineNotPassed);

        let wager = bet_info.wager;

        // Generate PDA seeds for signing
        let participant1_key = participant1.key();
        let participant2_key = participant2.key();
        let seeds = &[
            participant1_key.as_ref(),
            participant2_key.as_ref(),
        ];
        let (pda, bump) = Pubkey::find_program_address(seeds, ctx.program_id);
        require!(pda == bet_info.key(), BettingError::InvalidPDA);

        let signer_seeds = &[
            participant1_key.as_ref(),
            participant2_key.as_ref(),
            &[bump],
        ];

        // Return original wagers to participants
        **bet_info.to_account_info().try_borrow_mut_lamports()? -= wager;
        **participant1.to_account_info().try_borrow_mut_lamports()? += wager;

        **bet_info.to_account_info().try_borrow_mut_lamports()? -= wager;
        **participant2.to_account_info().try_borrow_mut_lamports()? += wager;

        // Mark bet as inactive
        bet_info.is_active = false;

        msg!("Timeout: Returned {} lamports to each participant", wager);
        Ok(())
    }
}

#[derive(Accounts)]
pub struct JoinCtx<'info> {
    #[account(mut)]
    pub participant1: Signer<'info>,
    
    #[account(mut)]
    pub participant2: Signer<'info>,
    
    /// CHECK: Oracle address is stored for later validation
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
    /// CHECK: Winner validation happens in instruction
    pub winner: AccountInfo<'info>,
    
    #[account(
        mut,
        seeds = [participant1.key().as_ref(), participant2.key().as_ref()],
        bump
    )]
    pub bet_info: Account<'info, BetInfo>,
    
    /// CHECK: Used only for PDA derivation
    pub participant1: AccountInfo<'info>,
    
    /// CHECK: Used only for PDA derivation
    pub participant2: AccountInfo<'info>,
    
    pub system_program: Program<'info, System>,
}

#[derive(Accounts)]
pub struct TimeoutCtx<'info> {
    #[account(mut)]
    /// CHECK: Either participant can call timeout
    pub participant1: AccountInfo<'info>,
    
    #[account(mut)]
    /// CHECK: Either participant can call timeout
    pub participant2: AccountInfo<'info>,
    
    #[account(
        mut,
        seeds = [participant1.key().as_ref(), participant2.key().as_ref()],
        bump
    )]
    pub bet_info: Account<'info, BetInfo>,
    
    pub system_program: Program<'info, System>,
}

#[account]
pub struct BetInfo {
    pub participant1: Pubkey,
    pub participant2: Pubkey,
    pub oracle: Pubkey,
    pub deadline: u64,
    pub wager: u64,
    pub total_pot: u64,
    pub is_active: bool,
}

impl BetInfo {
    pub const LEN: usize = 8 + // discriminator
        32 + // participant1
        32 + // participant2
        32 + // oracle
        8 + // deadline
        8 + // wager
        8 + // total_pot
        1; // is_active
}

#[error_code]
pub enum BettingError {
    #[msg("Invalid wager amount")]
    InvalidWager,
    #[msg("Invalid oracle")]
    InvalidOracle,
    #[msg("Bet is not active")]
    BetNotActive,
    #[msg("Invalid winner")]
    InvalidWinner,
    #[msg("Deadline has passed")]
    DeadlinePassed,
    #[msg("Deadline has not passed yet")]
    DeadlineNotPassed,
    #[msg("Invalid PDA")]
    InvalidPDA,
}
