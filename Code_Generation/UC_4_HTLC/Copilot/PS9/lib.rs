use anchor_lang::prelude::*;
use anchor_lang::solana_program::keccak;
use anchor_lang::solana_program::system_instruction;
use anchor_lang::solana_program::program::invoke;

declare_id!("5WBLPaU9dsGXecBoKjHzpEaKsYiqVhV4hbtVBjH4L7Lb");

#[program]
pub mod htlc_keccak {
    use super::*;

    pub fn initialize(
        ctx: Context<InitializeCtx>,
        hashed_secret: [u8; 32],
        delay: u64,
        amount: u64,
    ) -> Result<()> {
        require!(amount > 0, HtlcError::ZeroAmount);

        let clock = Clock::get()?;
        // canonical deadline: now + delay
        let deadline = clock.slot.checked_add(delay).ok_or(HtlcError::Overflow)?;

        // transfer lamports to PDA before mutably borrowing it
        let ix = system_instruction::transfer(
            &ctx.accounts.owner.key(),
            &ctx.accounts.htlcInfo.key(),
            amount,
        );
        invoke(
            &ix,
            &[
                ctx.accounts.owner.to_account_info(),
                ctx.accounts.htlcInfo.to_account_info(),
                ctx.accounts.system_program.to_account_info(),
            ],
        )?;

        let htlc = &mut ctx.accounts.htlcInfo;
        htlc.owner = ctx.accounts.owner.key();
        htlc.verifier = ctx.accounts.verifier.key();
        htlc.hashed_secret = hashed_secret;
        htlc.reveal_timeout = deadline;
        htlc.amount = amount;

        msg!(
            "HTLC initialized: owner={}, verifier={}, amount={}, reveal_timeout={}",
            htlc.owner,
            htlc.verifier,
            htlc.amount,
            htlc.reveal_timeout
        );

        Ok(())
    }

    pub fn reveal(ctx: Context<RevealCtx>, secret: String) -> Result<()> {
        let htlc = &mut ctx.accounts.htlcInfo;

        let current_slot = Clock::get()?.slot;
        msg!("reveal: current_slot: {}", current_slot);
        msg!("reveal: stored_reveal_timeout: {}", htlc.reveal_timeout);

        // allow reveal up to and including the deadline
        require!(
            current_slot <= htlc.reveal_timeout,
            HtlcError::RevealAfterTimeout
        );

        let secret_bytes = secret.into_bytes();
        let keccak_hash = keccak::hash(&secret_bytes).0;
        require!(keccak_hash == htlc.hashed_secret, HtlcError::InvalidSecret);

        htlc.amount = 0u64;
        msg!("reveal: secret verified, closing PDA to owner");
        Ok(())
    }

    pub fn timeout(ctx: Context<TimeoutCtx>) -> Result<()> {
        let htlc = &ctx.accounts.htlcInfo;

        let current_slot = Clock::get()?.slot;
        msg!("timeout: current_slot: {}", current_slot);
        msg!("timeout: stored_reveal_timeout: {}", htlc.reveal_timeout);

        // allow timeout strictly after the deadline
        require!(current_slot > htlc.reveal_timeout, HtlcError::TooEarlyTimeout);

        msg!("timeout: deadline reached, closing PDA to verifier");
        Ok(())
    }
}

#[derive(Accounts)]
#[instruction(hashed_secret: [u8;32], delay: u64, amount: u64)]
pub struct InitializeCtx<'info> {
    #[account(mut)]
    pub owner: Signer<'info>,

    /// CHECK: Verified by PDA seeds and stored in state
    pub verifier: UncheckedAccount<'info>,

    #[account(
        init,
        payer = owner,
        space = 8 + 32 + 32 + 32 + 8 + 8,
        seeds = [owner.key().as_ref(), verifier.key().as_ref()],
        bump
    )]
    pub htlcInfo: Account<'info, HtlcPda>,

    pub system_program: Program<'info, System>,
}

#[derive(Accounts)]
pub struct RevealCtx<'info> {
    #[account(mut)]
    pub owner: Signer<'info>,

    /// CHECK: Verified by PDA seeds and stored in state
    pub verifier: UncheckedAccount<'info>,

    #[account(
        mut,
        seeds = [owner.key().as_ref(), verifier.key().as_ref()],
        bump,
        has_one = owner,
        has_one = verifier,
        close = owner
    )]
    pub htlcInfo: Account<'info, HtlcPda>,
}

#[derive(Accounts)]
pub struct TimeoutCtx<'info> {
    #[account(mut)]
    pub verifier: Signer<'info>,

    /// CHECK: Verified by PDA seeds and stored in state
    pub owner: UncheckedAccount<'info>,

    #[account(
        mut,
        seeds = [owner.key().as_ref(), verifier.key().as_ref()],
        bump,
        has_one = owner,
        has_one = verifier,
        close = verifier
    )]
    pub htlcInfo: Account<'info, HtlcPda>,
}

#[account]
pub struct HtlcPda {
    pub owner: Pubkey,
    pub verifier: Pubkey,
    pub hashed_secret: [u8; 32],
    pub reveal_timeout: u64,
    pub amount: u64,
}

#[error_code]
pub enum HtlcError {
    #[msg("Locked amount must be greater than zero")]
    ZeroAmount,
    #[msg("Reveal attempted after timeout")]
    RevealAfterTimeout,
    #[msg("Provided secret does not match commitment")]
    InvalidSecret,
    #[msg("Timeout called before deadline")]
    TooEarlyTimeout,
    #[msg("Arithmetic overflow")]
    Overflow,
}
