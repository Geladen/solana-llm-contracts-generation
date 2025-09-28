use anchor_lang::prelude::*;
use anchor_lang::solana_program::{keccak, system_instruction, system_program, clock::Clock};
use std::io::Write;

declare_id!("H6tBeUkuvp47bFVGxVgYCumigBKezsbSd7XjiXfVNtWb");

#[program]
pub mod htlc {
    use super::*;

    pub fn initialize(
        ctx: Context<InitializeCtx>,
        hashed_secret: [u8; 32],
        delay: u64,
        amount: u64,
    ) -> Result<()> {
        let clock = Clock::get()?;
        
        // Get the PDA key before mutable borrow
        let htlc_info_key = ctx.accounts.htlc_info.key();
        let owner_key = ctx.accounts.owner.key();
        
        // Transfer funds to HTLC PDA first
        let transfer_instruction = system_instruction::transfer(
            &owner_key,
            &htlc_info_key,
            amount,
        );

        anchor_lang::solana_program::program::invoke(
            &transfer_instruction,
            &[
                ctx.accounts.owner.to_account_info(),
                ctx.accounts.htlc_info.to_account_info(),
                ctx.accounts.system_program.to_account_info(),
            ],
        )?;

        // Now initialize HTLC state after transfer
        let htlc_info = &mut ctx.accounts.htlc_info;
        htlc_info.owner = owner_key;
        htlc_info.verifier = ctx.accounts.verifier.key();
        htlc_info.hashed_secret = hashed_secret;
        htlc_info.reveal_timeout = clock.slot.checked_add(delay)
            .ok_or(ErrorCode::ArithmeticOverflow)?;
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
        // Read values without mutable borrow first
        let htlc_info = &ctx.accounts.htlc_info;

        // Verify HTLC still has funds
        require!(htlc_info.amount > 0, ErrorCode::AlreadyClaimed);

        // Compute Keccak-256 hash of provided secret
        let secret_bytes = secret.as_bytes();
        let computed_hash = keccak::hash(secret_bytes).to_bytes();

        // Verify hash matches the committed hash
        require!(
            computed_hash == htlc_info.hashed_secret,
            ErrorCode::InvalidSecret
        );

        // Get the locked amount for transfer
        let amount = htlc_info.amount;

        // Transfer the locked funds to owner (rent will be handled by close attribute)
        let htlc_info_account_info = ctx.accounts.htlc_info.to_account_info();
        let owner_account_info = ctx.accounts.owner.to_account_info();
        
        **owner_account_info.try_borrow_mut_lamports()? += amount;
        **htlc_info_account_info.try_borrow_mut_lamports()? -= amount;

        msg!("Secret revealed successfully! Funds transferred to owner.");
        Ok(())
    }

    pub fn timeout(ctx: Context<TimeoutCtx>) -> Result<()> {
        let clock = Clock::get()?;

        // Read values without mutable borrow first
        let htlc_info = &ctx.accounts.htlc_info;

        // Verify deadline has passed
        require!(
            clock.slot > htlc_info.reveal_timeout,
            ErrorCode::DeadlineNotReached
        );

        // Verify HTLC still has funds
        require!(htlc_info.amount > 0, ErrorCode::AlreadyClaimed);

        // Get the locked amount for transfer
        let amount = htlc_info.amount;

        // Transfer the locked funds to verifier (rent will be handled by close attribute)
        let htlc_info_account_info = ctx.accounts.htlc_info.to_account_info();
        let verifier_account_info = ctx.accounts.verifier.to_account_info();
        
        **verifier_account_info.try_borrow_mut_lamports()? += amount;
        **htlc_info_account_info.try_borrow_mut_lamports()? -= amount;

        msg!("Timeout reached! Funds transferred to verifier.");
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
        has_one = owner @ ErrorCode::InvalidOwner,
        has_one = verifier @ ErrorCode::InvalidVerifier,
        close = owner // Close account and send rent to owner
    )]
    pub htlc_info: Account<'info, HtlcPDA>,
    
    pub system_program: Program<'info, System>,
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
        has_one = owner @ ErrorCode::InvalidOwner,
        has_one = verifier @ ErrorCode::InvalidVerifier,
        close = verifier // Close account and send rent to verifier
    )]
    pub htlc_info: Account<'info, HtlcPDA>,
    
    pub system_program: Program<'info, System>,
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
    // Account discriminator (8 bytes) + fields
    pub const SIZE: usize = 8 + 32 + 32 + 32 + 8 + 8 + 1;
}

#[error_code]
pub enum ErrorCode {
    #[msg("Invalid secret provided")]
    InvalidSecret,
    #[msg("Deadline has not been reached yet")]
    DeadlineNotReached,
    #[msg("Funds have already been claimed")]
    AlreadyClaimed,
    #[msg("Arithmetic overflow occurred")]
    ArithmeticOverflow,
    #[msg("Invalid owner account")]
    InvalidOwner,
    #[msg("Invalid verifier account")]
    InvalidVerifier,
}