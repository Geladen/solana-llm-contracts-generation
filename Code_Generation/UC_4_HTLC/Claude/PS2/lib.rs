use anchor_lang::prelude::*;
use anchor_lang::solana_program::keccak;

declare_id!("57PKu5mvA7zch3Ddp5sxy3VLF6wu8E1qgkSiqMq7Hktw");

// Helper function to compute Keccak-256 hash consistently
fn compute_keccak256_hash(input: &str) -> [u8; 32] {
    keccak::hash(input.as_bytes()).to_bytes()
}

#[program]
pub mod htlc {
    use super::*;

    /// Initialize a new Hash Timed Locked Contract
    /// Called by the committer (owner) to deposit funds and create the contract
    pub fn initialize(
        ctx: Context<InitializeCtx>,
        hashed_secret: [u8; 32],
        delay: u64,
        amount: u64,
    ) -> Result<()> {
        let htlc_info = &mut ctx.accounts.htlc_info;
        let clock = Clock::get()?;

        // Validate amount is greater than zero
        require!(amount > 0, HtlcError::InvalidAmount);

        // Validate delay is reasonable (must be in the future)
        require!(delay > 0, HtlcError::InvalidDelay);

        // Calculate the reveal timeout (current slot + delay)
        let reveal_timeout = clock.slot.checked_add(delay)
            .ok_or(HtlcError::ArithmeticOverflow)?;

        // Initialize the HTLC PDA with contract parameters
        htlc_info.owner = ctx.accounts.owner.key();
        htlc_info.verifier = ctx.accounts.verifier.key();
        htlc_info.hashed_secret = hashed_secret;
        htlc_info.reveal_timeout = reveal_timeout;
        htlc_info.amount = amount;

        // Transfer the specified amount from owner to HTLC PDA
        let cpi_context = CpiContext::new(
            ctx.accounts.system_program.to_account_info(),
            anchor_lang::system_program::Transfer {
                from: ctx.accounts.owner.to_account_info(),
                to: ctx.accounts.htlc_info.to_account_info(),
            },
        );
        anchor_lang::system_program::transfer(cpi_context, amount)?;

        emit!(HtlcInitialized {
            owner: ctx.accounts.owner.key(),
            verifier: ctx.accounts.verifier.key(),
            amount,
            reveal_timeout,
            hashed_secret,
        });

        Ok(())
    }

    /// Reveal the secret to claim the locked funds
    /// Called by the committer (owner) with the correct secret at any time
    pub fn reveal(ctx: Context<RevealCtx>, secret: String) -> Result<()> {
        let htlc_info = &ctx.accounts.htlc_info;

        // Hash the provided secret using Keccak-256
        let computed_hash = compute_keccak256_hash(&secret);

        // Verify the hash matches the committed hash
        require!(
            computed_hash == htlc_info.hashed_secret,
            HtlcError::InvalidSecret
        );

        // Get the amount before account closure for the event
        let htlc_lamports = ctx.accounts.htlc_info.to_account_info().lamports();

        emit!(SecretRevealed {
            owner: ctx.accounts.owner.key(),
            verifier: ctx.accounts.verifier.key(),
            secret,
            amount: htlc_lamports,
        });

        // Account will be closed automatically by the `close = owner` constraint
        // and all lamports will be transferred to the owner

        Ok(())
    }

    /// Claim funds after timeout expires
    /// Called by the receiver (verifier) after the reveal timeout has passed
    pub fn timeout(ctx: Context<TimeoutCtx>) -> Result<()> {
        let htlc_info = &ctx.accounts.htlc_info;
        let clock = Clock::get()?;

        // Check that the timeout has been reached (strictly greater than)
        require!(
            clock.slot > htlc_info.reveal_timeout,
            HtlcError::TimeoutNotReached
        );

        // Get the amount before account closure for the event
        let htlc_lamports = ctx.accounts.htlc_info.to_account_info().lamports();

        emit!(TimeoutClaimed {
            owner: ctx.accounts.owner.key(),
            verifier: ctx.accounts.verifier.key(),
            amount: htlc_lamports,
        });

        // Account will be closed automatically by the `close = verifier` constraint
        // and all lamports will be transferred to the verifier

        Ok(())
    }
}

/// Context for initializing a new HTLC
#[derive(Accounts)]
pub struct InitializeCtx<'info> {
    #[account(mut)]
    pub owner: Signer<'info>,
    
    /// CHECK: This is the verifier's public key, used as a reference only
    pub verifier: UncheckedAccount<'info>,
    
    #[account(
        init,
        payer = owner,
        space = HtlcPda::LEN,
        seeds = [owner.key().as_ref(), verifier.key().as_ref()],
        bump
    )]
    pub htlc_info: Account<'info, HtlcPda>,
    
    pub system_program: Program<'info, System>,
}

/// Context for revealing the secret
#[derive(Accounts)]
pub struct RevealCtx<'info> {
    #[account(mut)]
    pub owner: Signer<'info>,
    
    /// CHECK: This is the verifier's public key, used as a reference only
    pub verifier: UncheckedAccount<'info>,
    
    #[account(
        mut,
        seeds = [owner.key().as_ref(), verifier.key().as_ref()],
        bump,
        constraint = htlc_info.owner == owner.key() @ HtlcError::UnauthorizedOwner,
        constraint = htlc_info.verifier == verifier.key() @ HtlcError::UnauthorizedVerifier,
        close = owner
    )]
    pub htlc_info: Account<'info, HtlcPda>,
}

/// Context for claiming funds after timeout
#[derive(Accounts)]
pub struct TimeoutCtx<'info> {
    #[account(mut)]
    pub verifier: Signer<'info>,
    
    /// CHECK: This is the owner's public key, used as a reference only
    pub owner: UncheckedAccount<'info>,
    
    #[account(
        mut,
        seeds = [owner.key().as_ref(), verifier.key().as_ref()],
        bump,
        constraint = htlc_info.owner == owner.key() @ HtlcError::UnauthorizedOwner,
        constraint = htlc_info.verifier == verifier.key() @ HtlcError::UnauthorizedVerifier,
        close = verifier
    )]
    pub htlc_info: Account<'info, HtlcPda>,
}

/// HTLC contract state account
#[account]
pub struct HtlcPda {
    /// The committer who deposits funds and can reveal the secret
    pub owner: Pubkey,           // 32 bytes
    
    /// The receiver who can claim funds after timeout
    pub verifier: Pubkey,        // 32 bytes
    
    /// Keccak-256 hash of the secret
    pub hashed_secret: [u8; 32], // 32 bytes
    
    /// Slot number when the reveal period expires
    pub reveal_timeout: u64,     // 8 bytes
    
    /// Amount of lamports locked in the contract
    pub amount: u64,             // 8 bytes
}

impl HtlcPda {
    /// Total space required for the HTLC account
    /// 8 bytes (discriminator) + 32 + 32 + 32 + 8 + 8 = 120 bytes
    pub const LEN: usize = 8 + 32 + 32 + 32 + 8 + 8;
}

/// Events emitted by the program
#[event]
pub struct HtlcInitialized {
    pub owner: Pubkey,
    pub verifier: Pubkey,
    pub amount: u64,
    pub reveal_timeout: u64,
    pub hashed_secret: [u8; 32],
}

#[event]
pub struct SecretRevealed {
    pub owner: Pubkey,
    pub verifier: Pubkey,
    pub secret: String,
    pub amount: u64,
}

#[event]
pub struct TimeoutClaimed {
    pub owner: Pubkey,
    pub verifier: Pubkey,
    pub amount: u64,
}

/// Custom error codes
#[error_code]
pub enum HtlcError {
    #[msg("Amount must be greater than zero")]
    InvalidAmount,
    
    #[msg("Delay must be greater than zero")]
    InvalidDelay,
    
    #[msg("Arithmetic overflow occurred")]
    ArithmeticOverflow,
    
    #[msg("The provided secret does not match the committed hash")]
    InvalidSecret,
    
    #[msg("The timeout period has not been reached yet")]
    TimeoutNotReached,
    
    #[msg("Unauthorized: caller is not the expected owner")]
    UnauthorizedOwner,
    
    #[msg("Unauthorized: caller is not the expected verifier")]
    UnauthorizedVerifier,
}