#![allow(unexpected_cfgs)]
use anchor_lang::prelude::*;
use anchor_lang::system_program;

declare_id!("Am8xvztKQ6g9C3C8nUfqdwZ3rz8Jv4kG6pXzeKyFng58");

#[program]
pub mod two_party_bet {
    use super::*;

    pub fn join(ctx: Context<JoinCtx>, delay: u64, wager: u64) -> Result<()> {
        // Validate both participants have signed
        require!(
            ctx.accounts.participant1.is_signer && ctx.accounts.participant2.is_signer,
            BetError::MissingRequiredSignatures
        );

        // Validate wager is positive
        require!(wager > 0, BetError::InvalidWager);

        // Transfer wager from participant1
        let cpi_program = ctx.accounts.system_program.to_account_info();
        let cpi_accounts = system_program::Transfer {
            from: ctx.accounts.participant1.to_account_info(),
            to: ctx.accounts.bet_info.to_account_info(),
        };
        let cpi_ctx = CpiContext::new(cpi_program.clone(), cpi_accounts);
        system_program::transfer(cpi_ctx, wager)?;

        // Transfer wager from participant2
        let cpi_accounts = system_program::Transfer {
            from: ctx.accounts.participant2.to_account_info(),
            to: ctx.accounts.bet_info.to_account_info(),
        };
        let cpi_ctx = CpiContext::new(cpi_program, cpi_accounts);
        system_program::transfer(cpi_ctx, wager)?;

        // Initialize bet info
        let bet_info = &mut ctx.accounts.bet_info;
        bet_info.participant1 = ctx.accounts.participant1.key();
        bet_info.participant2 = ctx.accounts.participant2.key();
        bet_info.oracle = ctx.accounts.oracle.key();
        bet_info.wager = wager;
        bet_info.deadline = Clock::get()?.slot + delay;
        bet_info.state = BetState::Active;
        bet_info.bump = ctx.bumps.bet_info;

        Ok(())
    }

    pub fn win(ctx: Context<WinCtx>) -> Result<()> {
        // Validate oracle signature
        require!(ctx.accounts.oracle.is_signer, BetError::UnauthOracle); // Fixed error variant name

        // Extract values before mutable borrow to avoid issues
        let (bump, participant1_key, participant2_key, wager, deadline) = {
            let bet_info = &ctx.accounts.bet_info;
            
            // Validate bet is active
            require!(bet_info.state == BetState::Active, BetError::BetAlreadyResolved);
            
            // Validate deadline hasn't passed for win
            require!(Clock::get()?.slot <= bet_info.deadline, BetError::DeadlinePassed);
            
            // Validate oracle matches stored oracle
            require!(
                bet_info.oracle == ctx.accounts.oracle.key(),
                BetError::InvalidOracle
            );

            // Validate winner is one of the participants
            require!(
                ctx.accounts.winner.key() == bet_info.participant1 || 
                ctx.accounts.winner.key() == bet_info.participant2,
                BetError::InvalidWinner
            );

            (bet_info.bump, bet_info.participant1, bet_info.participant2, bet_info.wager, bet_info.deadline)
        };

        // Calculate total pot (2x wager)
        let total_pot = wager.checked_mul(2).unwrap();
        
        // Manual lamport transfer to avoid issues with data-carrying accounts
        let bet_info_account = &ctx.accounts.bet_info.to_account_info();
        let winner_account = &ctx.accounts.winner;
        
        // Check if bet account has enough lamports
        require!(
            bet_info_account.lamports() >= total_pot,
            BetError::InsufficientFunds
        );
        
        // Perform manual transfer
        **bet_info_account.try_borrow_mut_lamports()? = bet_info_account
            .lamports()
            .checked_sub(total_pot)
            .ok_or(BetError::InsufficientFunds)?;
            
        **winner_account.try_borrow_mut_lamports()? = winner_account
            .lamports()
            .checked_add(total_pot)
            .ok_or(BetError::TransferFailed)?;

        // Mark bet as resolved
        let bet_info = &mut ctx.accounts.bet_info;
        bet_info.state = BetState::Resolved;

        Ok(())
    }

    pub fn timeout(ctx: Context<TimeoutCtx>) -> Result<()> {
        // Validate at least one participant has signed
        require!(
            ctx.accounts.participant1.is_signer || ctx.accounts.participant2.is_signer,
            BetError::MissingRequiredSignatures
        );

        // Extract values before mutable borrow to avoid issues
        let (bump, participant1_key, participant2_key, wager, deadline) = {
            let bet_info = &ctx.accounts.bet_info;
            
            // Validate bet is active
            require!(bet_info.state == BetState::Active, BetError::BetAlreadyResolved);
            
            // Validate deadline has passed
            require!(Clock::get()?.slot > bet_info.deadline, BetError::DeadlineNotReached);

            (bet_info.bump, bet_info.participant1, bet_info.participant2, bet_info.wager, bet_info.deadline)
        };

        // Calculate refund amounts
        let refund_amount = wager;
        let total_refund = refund_amount.checked_mul(2).unwrap();
        
        // Manual lamport transfers to avoid issues with data-carrying accounts
        let bet_info_account = &ctx.accounts.bet_info.to_account_info();
        let participant1_account = &ctx.accounts.participant1.to_account_info();
        let participant2_account = &ctx.accounts.participant2.to_account_info();
        
        // Check if bet account has enough lamports
        require!(
            bet_info_account.lamports() >= total_refund,
            BetError::InsufficientFunds
        );
        
        // Perform manual transfers
        **bet_info_account.try_borrow_mut_lamports()? = bet_info_account
            .lamports()
            .checked_sub(total_refund)
            .ok_or(BetError::InsufficientFunds)?;
            
        **participant1_account.try_borrow_mut_lamports()? = participant1_account
            .lamports()
            .checked_add(refund_amount)
            .ok_or(BetError::TransferFailed)?;
            
        **participant2_account.try_borrow_mut_lamports()? = participant2_account
            .lamports()
            .checked_add(refund_amount)
            .ok_or(BetError::TransferFailed)?;

        // Mark bet as resolved
        let bet_info = &mut ctx.accounts.bet_info;
        bet_info.state = BetState::Resolved;

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
    
    /// CHECK: This is the oracle account that will be stored for future verification
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
    #[account(mut, signer)]
    pub oracle: Signer<'info>,
    
    #[account(mut)]
    /// CHECK: This is validated to be one of the participants in the win function
    pub winner: AccountInfo<'info>,
    
    #[account(
        mut,
        seeds = [bet_info.participant1.as_ref(), bet_info.participant2.as_ref()],
        bump = bet_info.bump
    )]
    pub bet_info: Account<'info, BetInfo>,
    
    /// CHECK: Used for PDA derivation validation
    pub participant1: AccountInfo<'info>,
    
    /// CHECK: Used for PDA derivation validation
    pub participant2: AccountInfo<'info>,
}

#[derive(Accounts)]
pub struct TimeoutCtx<'info> {
    #[account(mut)]
    pub participant1: Signer<'info>,
    #[account(mut)]
    pub participant2: Signer<'info>,
    
    #[account(
        mut,
        seeds = [bet_info.participant1.as_ref(), bet_info.participant2.as_ref()],
        bump = bet_info.bump
    )]
    pub bet_info: Account<'info, BetInfo>,
}

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
    pub const LEN: usize = 8 + 32 + 32 + 32 + 8 + 8 + 1 + 1;
}

#[derive(AnchorSerialize, AnchorDeserialize, Clone, PartialEq, Eq)]
pub enum BetState {
    Active,
    Resolved,
}

#[error_code]
pub enum BetError {
    #[msg("Missing required signatures")]
    MissingRequiredSignatures,
    #[msg("Unauthorized oracle access")]
    UnauthOracle, // This is the correct variant name
    #[msg("Invalid oracle account")]
    InvalidOracle,
    #[msg("Bet has already been resolved")]
    BetAlreadyResolved,
    #[msg("Deadline has not been reached")]
    DeadlineNotReached,
    #[msg("Deadline has already passed for win operation")]
    DeadlinePassed,
    #[msg("Invalid wager amount")]
    InvalidWager,
    #[msg("Insufficient funds in bet account")]
    InsufficientFunds,
    #[msg("Transfer failed")]
    TransferFailed,
    #[msg("Winner must be one of the participants")]
    InvalidWinner,
}
