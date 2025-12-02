use anchor_lang::prelude::*;
use anchor_lang::solana_program::{
    keccak,
    system_program,
    clock::Clock,
    program::invoke,
    system_instruction
};

declare_id!("FFeRzuHPdNxRSMGd7c1mfCjpPKgFkmb8Pd4hFXwewfhU");

#[program]
pub mod htlc {
    use super::*;

    pub fn initialize(
        ctx: Context<InitializeCtx>,
        hashed_secret: [u8; 32],
        delay: u64,
        amount: u64,
    ) -> Result<()> {
        require!(amount > 0, HtlcError::InvalidAmount);
        require!(delay > 0, HtlcError::InvalidDelay);

        let clock = Clock::get()?;
        let reveal_timeout = clock.slot + delay;

        // Transfer funds FIRST (before any mutable borrows)
        let transfer_ix = system_instruction::transfer(
            &ctx.accounts.owner.key(),
            &ctx.accounts.htlc_pda.key(),
            amount,
        );

        invoke(
            &transfer_ix,
            &[
                ctx.accounts.owner.to_account_info(),
                ctx.accounts.htlc_pda.to_account_info(),
                ctx.accounts.system_program.to_account_info(),
            ],
        )?;

        // THEN initialize account data
        let htlc_pda = &mut ctx.accounts.htlc_pda;
        htlc_pda.owner = ctx.accounts.owner.key();
        htlc_pda.verifier = ctx.accounts.verifier.key();
        htlc_pda.hashed_secret = hashed_secret;
        htlc_pda.reveal_timeout = reveal_timeout;
        htlc_pda.amount = amount;
        htlc_pda.bump = ctx.bumps.htlc_pda;

        emit!(HtlcInitialized {
            owner: htlc_pda.owner,
            verifier: htlc_pda.verifier,
            hashed_secret: htlc_pda.hashed_secret,
            reveal_timeout: htlc_pda.reveal_timeout,
            amount: htlc_pda.amount,
        });

        Ok(())
    }

    pub fn reveal(ctx: Context<RevealCtx>, secret: String) -> Result<()> {
        let htlc_pda = &ctx.accounts.htlc_pda;
        
        // Verify secret matches the commitment
        let secret_bytes = secret.as_bytes();
        let computed_hash = keccak::hash(secret_bytes);
        
        require!(
            computed_hash.0 == htlc_pda.hashed_secret,
            HtlcError::InvalidSecret
        );

        // Verify deadline hasn't passed
        let clock = Clock::get()?;
        require!(
            clock.slot <= htlc_pda.reveal_timeout,
            HtlcError::DeadlineExceeded
        );

        // Transfer funds back to owner
        let htlc_balance = ctx.accounts.htlc_pda.to_account_info().lamports();
        **ctx.accounts.htlc_pda.to_account_info().try_borrow_mut_lamports()? = 0;
        **ctx.accounts.owner.to_account_info().try_borrow_mut_lamports()? += htlc_balance;

        emit!(SecretRevealed {
            owner: htlc_pda.owner,
            verifier: htlc_pda.verifier,
            amount: htlc_pda.amount,
        });

        Ok(())
    }

    pub fn timeout(ctx: Context<TimeoutCtx>) -> Result<()> {
        let htlc_pda = &ctx.accounts.htlc_pda;
        
        // Verify deadline has passed
        let clock = Clock::get()?;
        require!(
            clock.slot > htlc_pda.reveal_timeout,
            HtlcError::DeadlineNotReached
        );

        // Transfer funds to verifier
        let htlc_balance = ctx.accounts.htlc_pda.to_account_info().lamports();
        **ctx.accounts.htlc_pda.to_account_info().try_borrow_mut_lamports()? = 0;
        **ctx.accounts.verifier.to_account_info().try_borrow_mut_lamports()? += htlc_balance;

        emit!(TimeoutTriggered {
            owner: htlc_pda.owner,
            verifier: htlc_pda.verifier,
            amount: htlc_pda.amount,
        });

        Ok(())
    }
}

#[derive(Accounts)]
pub struct InitializeCtx<'info> {
    #[account(mut)]
    pub owner: Signer<'info>,
    
    /// CHECK: This is the verifier/receiver who can claim after timeout
    pub verifier: AccountInfo<'info>,
    
    #[account(
        init,
        payer = owner,
        space = HtlcPDA::SIZE,
        seeds = [
            owner.key().as_ref(),
            verifier.key().as_ref()
        ],
        bump
    )]
    pub htlc_pda: Account<'info, HtlcPDA>,
    
    pub system_program: Program<'info, System>,
}

#[derive(Accounts)]
pub struct RevealCtx<'info> {
    #[account(mut)]
    pub owner: Signer<'info>,
    
    /// CHECK: Verifier reference for PDA verification
    pub verifier: AccountInfo<'info>,
    
    #[account(
        mut,
        seeds = [
            owner.key().as_ref(),
            verifier.key().as_ref()
        ],
        bump = htlc_pda.bump,
        has_one = owner @ HtlcError::InvalidOwner,
        has_one = verifier @ HtlcError::InvalidVerifier
    )]
    pub htlc_pda: Account<'info, HtlcPDA>,
}

#[derive(Accounts)]
pub struct TimeoutCtx<'info> {
    #[account(mut)]
    pub verifier: Signer<'info>,
    
    /// CHECK: Owner reference for PDA verification
    pub owner: AccountInfo<'info>,
    
    #[account(
        mut,
        seeds = [
            owner.key().as_ref(),
            verifier.key().as_ref()
        ],
        bump = htlc_pda.bump,
        has_one = owner @ HtlcError::InvalidOwner,
        has_one = verifier @ HtlcError::InvalidVerifier
    )]
    pub htlc_pda: Account<'info, HtlcPDA>,
}

#[account]
pub struct HtlcPDA {
    pub owner: Pubkey,           // 32 bytes
    pub verifier: Pubkey,        // 32 bytes
    pub hashed_secret: [u8; 32], // 32 bytes
    pub reveal_timeout: u64,     // 8 bytes
    pub amount: u64,             // 8 bytes
    pub bump: u8,                // 1 byte
}

impl HtlcPDA {
    pub const SIZE: usize = 8 + 32 + 32 + 32 + 8 + 8 + 1; // 121 bytes (8 discriminator + 113 data)
}

#[event]
pub struct HtlcInitialized {
    pub owner: Pubkey,
    pub verifier: Pubkey,
    pub hashed_secret: [u8; 32],
    pub reveal_timeout: u64,
    pub amount: u64,
}

#[event]
pub struct SecretRevealed {
    pub owner: Pubkey,
    pub verifier: Pubkey,
    pub amount: u64,
}

#[event]
pub struct TimeoutTriggered {
    pub owner: Pubkey,
    pub verifier: Pubkey,
    pub amount: u64,
}

#[error_code]
pub enum HtlcError {
    #[msg("Invalid secret provided")]
    InvalidSecret,
    #[msg("Deadline has already passed")]
    DeadlineExceeded,
    #[msg("Deadline has not been reached yet")]
    DeadlineNotReached,
    #[msg("Amount must be greater than zero")]
    InvalidAmount,
    #[msg("Delay must be greater than zero")]
    InvalidDelay,
    #[msg("Invalid owner account")]
    InvalidOwner,
    #[msg("Invalid verifier account")]
    InvalidVerifier,
}