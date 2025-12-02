use anchor_lang::prelude::*;
use anchor_lang::solana_program::keccak;

declare_id!("3SqipyQux9d7XG4nBvugs7xUn3K7QhDnjvc82sjVSSNg");

#[program]
pub mod htlc {
    use super::*;

    pub fn initialize(
        ctx: Context<InitializeCtx>,
        hashed_secret: [u8; 32],
        delay: u64,
        amount: u64,
    ) -> Result<()> {
        // Transfer collateral first
        let cpi_context = CpiContext::new(
            ctx.accounts.system_program.to_account_info(),
            anchor_lang::system_program::Transfer {
                from: ctx.accounts.owner.to_account_info(),
                to: ctx.accounts.htlc_info.to_account_info(),
            },
        );
        anchor_lang::system_program::transfer(cpi_context, amount)?;

        // Set HTLC parameters
        let htlc_info = &mut ctx.accounts.htlc_info;
        htlc_info.owner = ctx.accounts.owner.key();
        htlc_info.verifier = ctx.accounts.verifier.key();
        htlc_info.hashed_secret = hashed_secret;
        htlc_info.reveal_timeout = Clock::get()?.slot + delay;
        htlc_info.amount = amount;
        htlc_info.bump = ctx.bumps.htlc_info;

        msg!(
            "HTLC initialized: owner={}, verifier={}, amount={}, timeout_slot={}",
            htlc_info.owner,
            htlc_info.verifier,
            amount,
            htlc_info.reveal_timeout
        );

        Ok(())
    }

    pub fn reveal(ctx: Context<RevealCtx>, secret: String) -> Result<()> {
        let htlc_info = &ctx.accounts.htlc_info;
        
        // Check if funds are still available
        let htlc_lamports = ctx.accounts.htlc_info.to_account_info().lamports();
        require!(htlc_lamports > 0, HtlcError::AlreadyClaimed);

        // Compute Keccak-256 hash of provided secret
        let secret_bytes = secret.as_bytes();
        let computed_hash = keccak::hash(secret_bytes).to_bytes();

        // Verify hash matches commitment
        require!(
            computed_hash == htlc_info.hashed_secret,
            HtlcError::InvalidSecret
        );

        // Transfer funds
        **ctx.accounts.htlc_info.to_account_info().try_borrow_mut_lamports()? = 0;
        **ctx.accounts.owner.to_account_info().try_borrow_mut_lamports()? += htlc_lamports;

        msg!(
            "Secret revealed successfully: owner={}, amount={} reclaimed",
            htlc_info.owner,
            htlc_lamports
        );

        Ok(())
    }

    pub fn timeout(ctx: Context<TimeoutCtx>) -> Result<()> {
        let htlc_info = &ctx.accounts.htlc_info;
        
        // Verify timeout has been reached (STRICT check)
        let current_slot = Clock::get()?.slot;
        require!(
            current_slot > htlc_info.reveal_timeout,  // Changed to STRICT greater than
            HtlcError::TimeoutNotReached
        );

        // Additional check to ensure HTLC still has funds
        let htlc_lamports = ctx.accounts.htlc_info.to_account_info().lamports();
        require!(htlc_lamports > 0, HtlcError::AlreadyClaimed);

        // Transfer funds
        **ctx.accounts.htlc_info.to_account_info().try_borrow_mut_lamports()? = 0;
        **ctx.accounts.verifier.to_account_info().try_borrow_mut_lamports()? += htlc_lamports;

        msg!(
            "Timeout executed: verifier={}, amount={} claimed",
            htlc_info.verifier,
            htlc_lamports
        );

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
    pub htlc_info: Account<'info, HtlcPDA>,
    
    pub system_program: Program<'info, System>,
}

#[derive(Accounts)]
pub struct RevealCtx<'info> {
    #[account(mut)]
    pub owner: Signer<'info>,
    
    /// CHECK: Verifier reference for PDA validation
    pub verifier: AccountInfo<'info>,
    
    #[account(
        mut,
        seeds = [
            owner.key().as_ref(), 
            verifier.key().as_ref()
        ],
        bump = htlc_info.bump,
        has_one = owner @ HtlcError::InvalidOwner,
        has_one = verifier @ HtlcError::InvalidVerifier
    )]
    pub htlc_info: Account<'info, HtlcPDA>,
}

#[derive(Accounts)]
pub struct TimeoutCtx<'info> {
    #[account(mut)]
    pub verifier: Signer<'info>,
    
    /// CHECK: Owner reference for PDA validation
    pub owner: AccountInfo<'info>,
    
    #[account(
        mut,
        seeds = [
            owner.key().as_ref(), 
            verifier.key().as_ref()
        ],
        bump = htlc_info.bump,
        has_one = owner @ HtlcError::InvalidOwner,
        has_one = verifier @ HtlcError::InvalidVerifier
    )]
    pub htlc_info: Account<'info, HtlcPDA>,
}

#[account]
pub struct HtlcPDA {
    pub owner: Pubkey,        // 32 bytes
    pub verifier: Pubkey,     // 32 bytes
    pub hashed_secret: [u8; 32], // 32 bytes
    pub reveal_timeout: u64,  // 8 bytes
    pub amount: u64,          // 8 bytes
    pub bump: u8,             // 1 byte
}

impl HtlcPDA {
    pub const SIZE: usize = 32 + 32 + 32 + 8 + 8 + 1 + 8; // 121 bytes total
}

#[error_code]
pub enum HtlcError {
    #[msg("Timeout has not been reached yet")]
    TimeoutNotReached,
    #[msg("Invalid secret provided")]
    InvalidSecret,
    #[msg("Invalid owner")]
    InvalidOwner,
    #[msg("Invalid verifier")]
    InvalidVerifier,
    #[msg("Funds have already been claimed")]
    AlreadyClaimed,
}