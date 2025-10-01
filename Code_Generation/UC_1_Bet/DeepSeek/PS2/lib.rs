use anchor_lang::prelude::*;
use anchor_lang::system_program;

declare_id!("Am8xvztKQ6g9C3C8nUfqdwZ3rz8Jv4kG6pXzeKyFng58");

#[program]
pub mod betting_system {
    use super::*;

    pub fn join(ctx: Context<JoinCtx>, delay: u64, wager: u64) -> Result<()> {
        let clock = Clock::get()?;
        let bet_info = &mut ctx.accounts.bet_info;
        
        // Initialize bet info
        bet_info.participant1 = ctx.accounts.participant1.key();
        bet_info.participant2 = ctx.accounts.participant2.key();
        bet_info.oracle = ctx.accounts.oracle.key();
        bet_info.wager = wager;
        bet_info.deadline = clock.slot + delay;
        bet_info.bump = ctx.bumps.bet_info;

        // Transfer wagers to PDA using system program (allowed since participants are system accounts)
        let cpi_program = ctx.accounts.system_program.to_account_info();
        
        // Transfer from participant1
        let cpi_accounts = system_program::Transfer {
            from: ctx.accounts.participant1.to_account_info(),
            to: ctx.accounts.bet_info.to_account_info(),
        };
        let cpi_ctx = CpiContext::new(cpi_program.clone(), cpi_accounts);
        system_program::transfer(cpi_ctx, wager)?;

        // Transfer from participant2
        let cpi_accounts = system_program::Transfer {
            from: ctx.accounts.participant2.to_account_info(),
            to: ctx.accounts.bet_info.to_account_info(),
        };
        let cpi_ctx = CpiContext::new(cpi_program, cpi_accounts);
        system_program::transfer(cpi_ctx, wager)?;

        Ok(())
    }

    pub fn win(ctx: Context<WinCtx>) -> Result<()> {
        let bet_info = &ctx.accounts.bet_info;
        
        // Validate oracle
        require!(
            ctx.accounts.oracle.key() == bet_info.oracle,
            BetError::UnauthorizedOracle
        );
        
        // Validate that winner is one of the participants
        require!(
            ctx.accounts.winner.key() == bet_info.participant1 || 
            ctx.accounts.winner.key() == bet_info.participant2,
            BetError::WinnerNotParticipant
        );
        
        // Get the total amount in the PDA
        let amount = ctx.accounts.bet_info.to_account_info().lamports();
        
        // Directly transfer lamports from PDA to winner
        **ctx.accounts.winner.to_account_info().try_borrow_mut_lamports()? = 
            ctx.accounts.winner.to_account_info().lamports()
                .checked_add(amount)
                .unwrap();
        
        **ctx.accounts.bet_info.to_account_info().try_borrow_mut_lamports()? = 0;

        Ok(())
    }

    pub fn timeout(ctx: Context<TimeoutCtx>) -> Result<()> {
        let bet_info = &ctx.accounts.bet_info;
        let clock = Clock::get()?;
        
        require!(clock.slot > bet_info.deadline, BetError::DeadlineNotReached);
        
        // Validate that at least one participant is a signer
        require!(
            ctx.accounts.participant1.is_signer || ctx.accounts.participant2.is_signer,
            BetError::Unauthorized
        );

        // Get the total amount in the PDA
        let total_amount = ctx.accounts.bet_info.to_account_info().lamports();
        let refund_amount = bet_info.wager;
        
        // Directly transfer lamports from PDA to participants
        **ctx.accounts.participant1.to_account_info().try_borrow_mut_lamports()? = 
            ctx.accounts.participant1.to_account_info().lamports()
                .checked_add(refund_amount)
                .unwrap();
        
        **ctx.accounts.participant2.to_account_info().try_borrow_mut_lamports()? = 
            ctx.accounts.participant2.to_account_info().lamports()
                .checked_add(refund_amount)
                .unwrap();
        
        **ctx.accounts.bet_info.to_account_info().try_borrow_mut_lamports()? = 
            total_amount.checked_sub(refund_amount * 2).unwrap();

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
    /// CHECK: Read-only reference for storage
    pub oracle: AccountInfo<'info>,
    #[account(
        init,
        seeds = [participant1.key().as_ref(), participant2.key().as_ref()],
        bump,
        payer = participant1,
        space = 8 + BetInfo::INIT_SPACE
    )]
    pub bet_info: Account<'info, BetInfo>,
    pub system_program: Program<'info, System>,
}

#[derive(Accounts)]
pub struct WinCtx<'info> {
    #[account(mut)]
    pub oracle: Signer<'info>,
    #[account(mut)]
    /// CHECK: Winner account validated by oracle and must be one of the participants
    pub winner: AccountInfo<'info>,
    #[account(
        mut,
        seeds = [bet_info.participant1.as_ref(), bet_info.participant2.as_ref()],
        bump = bet_info.bump
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
    /// CHECK: Participant account that may receive refund
    #[account(mut)]
    pub participant1: AccountInfo<'info>,
    /// CHECK: Participant account that may receive refund
    #[account(mut)]
    pub participant2: AccountInfo<'info>,
    #[account(
        mut,
        seeds = [bet_info.participant1.as_ref(), bet_info.participant2.as_ref()],
        bump = bet_info.bump
    )]
    pub bet_info: Account<'info, BetInfo>,
    pub system_program: Program<'info, System>,
}

#[account]
#[derive(InitSpace)]
pub struct BetInfo {
    pub participant1: Pubkey,
    pub participant2: Pubkey,
    pub oracle: Pubkey,
    pub wager: u64,
    pub deadline: u64,
    pub bump: u8,
}

#[error_code]
pub enum BetError {
    #[msg("Deadline has not been reached")]
    DeadlineNotReached,
    #[msg("Only the designated oracle can call this function")]
    UnauthorizedOracle,
    #[msg("Unauthorized access")]
    Unauthorized,
    #[msg("Winner must be one of the participants")]
    WinnerNotParticipant,
}