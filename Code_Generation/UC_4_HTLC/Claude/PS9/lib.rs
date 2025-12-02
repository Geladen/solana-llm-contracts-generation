use anchor_lang::prelude::*;
use anchor_lang::solana_program::keccak;

declare_id!("8kVVgAAARTt6bRKrAwAJjUPRnnX4SqcQAkUS1RqYBvy");

#[program]
pub mod htlc {
    use super::*;

    /// Initialize a new Hash Timed Locked Contract
    /// Called by the committer (owner) to lock funds with a secret hash
    pub fn initialize(
        ctx: Context<InitializeCtx>,
        hashed_secret: [u8; 32],
        delay: u64,
        amount: u64,
    ) -> Result<()> {
        let current_slot = Clock::get()?.slot;
        let reveal_timeout = current_slot + delay;

        // Transfer funds from owner to HTLC PDA first
        let cpi_context = CpiContext::new(
            ctx.accounts.system_program.to_account_info(),
            anchor_lang::system_program::Transfer {
                from: ctx.accounts.owner.to_account_info(),
                to: ctx.accounts.htlc_info.to_account_info(),
            },
        );

        anchor_lang::system_program::transfer(cpi_context, amount)?;

        // Initialize HTLC state
        let htlc_info = &mut ctx.accounts.htlc_info;
        htlc_info.owner = ctx.accounts.owner.key();
        htlc_info.verifier = ctx.accounts.verifier.key();
        htlc_info.hashed_secret = hashed_secret;
        htlc_info.reveal_timeout = reveal_timeout;
        htlc_info.amount = amount;

        msg!("HTLC initialized with amount: {}, timeout: {}", amount, reveal_timeout);
        Ok(())
    }

    /// Reveal the secret and claim funds
    /// Called by the committer (owner) - no deadline restrictions in typical HTLC
    pub fn reveal(ctx: Context<RevealCtx>, secret: String) -> Result<()> {
        let htlc_info = &ctx.accounts.htlc_info;

        // No deadline check - owner can reveal anytime with correct secret
        
        // Compute Keccak-256 hash of the provided secret
        let secret_bytes = secret.as_bytes();
        let computed_hash = keccak::hash(secret_bytes);

        // Verify the hash matches the committed hash
        require!(
            computed_hash.to_bytes() == htlc_info.hashed_secret,
            HtlcError::InvalidSecret
        );

        // Transfer all available funds to owner (except minimum rent to keep account alive)
        let htlc_lamports = ctx.accounts.htlc_info.get_lamports();
        let rent_exempt = Rent::get()?.minimum_balance(8 + std::mem::size_of::<HtlcPda>());
        let transfer_amount = htlc_lamports.saturating_sub(rent_exempt);

        require!(transfer_amount > 0, HtlcError::InsufficientFunds);

        // Transfer funds from HTLC PDA to owner
        **ctx.accounts.htlc_info.to_account_info().try_borrow_mut_lamports()? -= transfer_amount;
        **ctx.accounts.owner.to_account_info().try_borrow_mut_lamports()? += transfer_amount;

        msg!("Secret revealed! Transferred {} lamports to owner", transfer_amount);
        Ok(())
    }

    /// Timeout claim - allows verifier to claim funds after deadline
    /// Called by the receiver (verifier) after the deadline has passed
    pub fn timeout(ctx: Context<TimeoutCtx>) -> Result<()> {
        let htlc_info = &ctx.accounts.htlc_info;
        let current_slot = Clock::get()?.slot;

        // Check if timeout has been reached (must be strictly after deadline)
        require!(current_slot > htlc_info.reveal_timeout, HtlcError::TimeoutNotReached);

        // Get total lamports in the HTLC account
        let htlc_lamports = ctx.accounts.htlc_info.get_lamports();

        // Transfer ALL lamports to verifier (this effectively closes the account)
        **ctx.accounts.htlc_info.to_account_info().try_borrow_mut_lamports()? = 0;
        **ctx.accounts.verifier.to_account_info().try_borrow_mut_lamports()? += htlc_lamports;

        msg!("Timeout reached! Transferred {} lamports to verifier", htlc_lamports);
        Ok(())
    }
}

// Context Structures

#[derive(Accounts)]
pub struct InitializeCtx<'info> {
    #[account(mut)]
    pub owner: Signer<'info>,
    
    /// CHECK: This account is only used as a reference for PDA generation
    pub verifier: UncheckedAccount<'info>,
    
    #[account(
        init,
        payer = owner,
        space = 8 + std::mem::size_of::<HtlcPda>(),
        seeds = [owner.key().as_ref(), verifier.key().as_ref()],
        bump
    )]
    pub htlc_info: Account<'info, HtlcPda>,
    
    pub system_program: Program<'info, System>,
}

#[derive(Accounts)]
pub struct RevealCtx<'info> {
    #[account(mut)]
    pub owner: Signer<'info>,
    
    /// CHECK: This account is only used as a reference for validation
    pub verifier: UncheckedAccount<'info>,
    
    #[account(
        mut,
        seeds = [owner.key().as_ref(), verifier.key().as_ref()],
        bump,
        constraint = htlc_info.owner == owner.key() @ HtlcError::UnauthorizedOwner,
        constraint = htlc_info.verifier == verifier.key() @ HtlcError::InvalidVerifier
    )]
    pub htlc_info: Account<'info, HtlcPda>,
}

#[derive(Accounts)]
pub struct TimeoutCtx<'info> {
    /// CHECK: This account is only used as a reference for validation
    pub owner: UncheckedAccount<'info>,
    
    #[account(mut)]
    pub verifier: Signer<'info>,
    
    #[account(
        mut,
        seeds = [owner.key().as_ref(), verifier.key().as_ref()],
        bump,
        constraint = htlc_info.owner == owner.key() @ HtlcError::UnauthorizedOwner,
        constraint = htlc_info.verifier == verifier.key() @ HtlcError::InvalidVerifier
    )]
    pub htlc_info: Account<'info, HtlcPda>,
}

// Account Structures

#[account]
pub struct HtlcPda {
    pub owner: Pubkey,           // 32 bytes - committer
    pub verifier: Pubkey,        // 32 bytes - receiver
    pub hashed_secret: [u8; 32], // 32 bytes - Keccak-256 hash of secret
    pub reveal_timeout: u64,     // 8 bytes - deadline slot
    pub amount: u64,             // 8 bytes - locked amount
}

// Error Types

#[error_code]
pub enum HtlcError {
    #[msg("The timeout has not been reached yet")]
    TimeoutNotReached,
    
    #[msg("The provided secret does not match the committed hash")]
    InvalidSecret,
    
    #[msg("Unauthorized: caller is not the owner")]
    UnauthorizedOwner,
    
    #[msg("Invalid verifier account")]
    InvalidVerifier,
    
    #[msg("Insufficient funds available for transfer")]
    InsufficientFunds,
}