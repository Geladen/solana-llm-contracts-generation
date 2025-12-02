use anchor_lang::prelude::*;
use anchor_lang::solana_program::keccak;

declare_id!("DygfZd6wy6EDpw338E1fo61kjTxfwBFaDybb9azTg9YD");

#[program]
pub mod htlc {
    use super::*;

    /// Initialize a new Hash Timed Locked Contract
    /// @param ctx: The context containing accounts
    /// @param hashed_secret: Keccak-256 hash of the secret (32 bytes)
    /// @param delay: Number of slots before timeout becomes valid
    /// @param amount: Amount of lamports to lock in the contract
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
        
        // Calculate timeout slot (current slot + delay)
        let reveal_timeout = clock.slot.checked_add(delay)
            .ok_or(HtlcError::TimeoutOverflow)?;
        
        // Initialize HTLC data
        htlc_info.owner = ctx.accounts.owner.key();
        htlc_info.verifier = ctx.accounts.verifier.key();
        htlc_info.hashed_secret = hashed_secret;
        htlc_info.reveal_timeout = reveal_timeout;
        htlc_info.amount = amount;
        htlc_info.state = HtlcPDA::STATE_ACTIVE;
        
        // Transfer funds from owner to HTLC PDA
        let transfer_instruction = anchor_lang::solana_program::system_instruction::transfer(
            &ctx.accounts.owner.key(),
            &htlc_info.key(),
            amount,
        );
        
        anchor_lang::solana_program::program::invoke(
            &transfer_instruction,
            &[
                ctx.accounts.owner.to_account_info(),
                htlc_info.to_account_info(),
            ],
        )?;
        
        msg!("HTLC initialized with timeout at slot: {}", reveal_timeout);
        Ok(())
    }

    /// Reveal the secret to claim funds (called by owner/committer)
    /// @param ctx: The context containing accounts
    /// @param secret: The secret string to reveal
    pub fn reveal(ctx: Context<RevealCtx>, secret: String) -> Result<()> {
        let htlc_info = &mut ctx.accounts.htlc_info;
        
        // Check that contract is still active
        require!(htlc_info.state == HtlcPDA::STATE_ACTIVE, HtlcError::ContractNotActive);
        
        // NOTE: No timeout check for reveals - owner can reveal at any time
        // The timeout only affects when verifier can claim funds
        
        // Convert secret string to bytes and compute Keccak-256 hash
        let secret_bytes = secret.as_bytes();
        let computed_hash = keccak::hash(secret_bytes);
        
        // Verify the hash matches the committed hash
        require!(
            computed_hash.to_bytes() == htlc_info.hashed_secret,
            HtlcError::InvalidSecret
        );
        
        // Transfer all funds from HTLC PDA to owner and close the account
        let htlc_lamports = htlc_info.to_account_info().lamports();
        
        **htlc_info.to_account_info().try_borrow_mut_lamports()? = 0;
        **ctx.accounts.owner.to_account_info().try_borrow_mut_lamports()? += htlc_lamports;
        
        // Mark contract as revealed
        htlc_info.state = HtlcPDA::STATE_REVEALED;
        
        msg!("Secret revealed successfully, {} lamports transferred to owner", htlc_lamports);
        Ok(())
    }

    /// Claim funds after timeout (called by verifier/receiver)
    /// @param ctx: The context containing accounts
    pub fn timeout(ctx: Context<TimeoutCtx>) -> Result<()> {
        let htlc_info = &mut ctx.accounts.htlc_info;
        let clock = Clock::get()?;
        
        // Check that contract is still active
        require!(htlc_info.state == HtlcPDA::STATE_ACTIVE, HtlcError::ContractNotActive);
        
        // Debug logging
        msg!("Current slot: {}, Timeout slot: {}", clock.slot, htlc_info.reveal_timeout);
        
        // Check that timeout has been reached
        require!(clock.slot >= htlc_info.reveal_timeout, HtlcError::TimeoutNotReached);
        
        // Transfer all funds from HTLC PDA to verifier and close the account
        let htlc_lamports = htlc_info.to_account_info().lamports();
        
        **htlc_info.to_account_info().try_borrow_mut_lamports()? = 0;
        **ctx.accounts.verifier.to_account_info().try_borrow_mut_lamports()? += htlc_lamports;
        
        // Mark contract as timed out
        htlc_info.state = HtlcPDA::STATE_TIMED_OUT;
        
        msg!("Timeout reached, {} lamports transferred to verifier", htlc_lamports);
        Ok(())
    }
}

#[derive(Accounts)]
pub struct InitializeCtx<'info> {
    #[account(mut)]
    pub owner: Signer<'info>,
    
    /// CHECK: This account is validated by being used in PDA seeds
    pub verifier: AccountInfo<'info>,
    
    #[account(
        init,
        payer = owner,
        space = 8 + HtlcPDA::LEN,
        seeds = [owner.key().as_ref(), verifier.key().as_ref()],
        bump
    )]
    pub htlc_info: Account<'info, HtlcPDA>,
    
    pub system_program: Program<'info, System>,
}

#[derive(Accounts)]
pub struct RevealCtx<'info> {
    #[account(mut)]
    pub owner: Signer<'info>,
    
    /// CHECK: This account is validated by being used in PDA seeds and stored in htlc_info
    pub verifier: AccountInfo<'info>,
    
    #[account(
        mut,
        seeds = [owner.key().as_ref(), verifier.key().as_ref()],
        bump,
        constraint = htlc_info.owner == owner.key() @ HtlcError::UnauthorizedOwner,
        constraint = htlc_info.verifier == verifier.key() @ HtlcError::UnauthorizedVerifier
    )]
    pub htlc_info: Account<'info, HtlcPDA>,
}

#[derive(Accounts)]
pub struct TimeoutCtx<'info> {
    /// CHECK: This account is validated by being stored in htlc_info
    pub owner: AccountInfo<'info>,
    
    #[account(mut)]
    pub verifier: Signer<'info>,
    
    #[account(
        mut,
        seeds = [owner.key().as_ref(), verifier.key().as_ref()],
        bump,
        constraint = htlc_info.owner == owner.key() @ HtlcError::UnauthorizedOwner,
        constraint = htlc_info.verifier == verifier.key() @ HtlcError::UnauthorizedVerifier
    )]
    pub htlc_info: Account<'info, HtlcPDA>,
}

#[account]
pub struct HtlcPDA {
    /// The owner/committer who deposited the funds
    pub owner: Pubkey,           // 32 bytes
    /// The verifier/receiver who can claim after timeout
    pub verifier: Pubkey,        // 32 bytes
    /// Keccak-256 hash of the secret
    pub hashed_secret: [u8; 32], // 32 bytes
    /// Slot number when timeout becomes valid
    pub reveal_timeout: u64,     // 8 bytes
    /// Amount of lamports locked in the contract
    pub amount: u64,             // 8 bytes
    /// State of the contract (0: active, 1: revealed, 2: timed out)
    pub state: u8,               // 1 byte
}

impl HtlcPDA {
    pub const LEN: usize = 32 + 32 + 32 + 8 + 8 + 1; // 113 bytes
    
    pub const STATE_ACTIVE: u8 = 0;
    pub const STATE_REVEALED: u8 = 1;
    pub const STATE_TIMED_OUT: u8 = 2;
}

#[error_code]
pub enum HtlcError {
    #[msg("Invalid amount: must be greater than zero")]
    InvalidAmount,
    
    #[msg("Timeout calculation overflow")]
    TimeoutOverflow,
    
    #[msg("Invalid secret: hash does not match")]
    InvalidSecret,
    
    #[msg("Timeout has been reached, cannot reveal secret")]
    TimeoutReached,
    
    #[msg("Timeout has not been reached, cannot claim funds")]
    TimeoutNotReached,
    
    #[msg("Unauthorized: not the contract owner")]
    UnauthorizedOwner,
    
    #[msg("Unauthorized: not the contract verifier")]
    UnauthorizedVerifier,
    
    #[msg("Contract is not active: funds have already been claimed")]
    ContractNotActive,
}