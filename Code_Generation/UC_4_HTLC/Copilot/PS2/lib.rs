use anchor_lang::prelude::*;
use anchor_lang::solana_program::keccak;
use anchor_lang::solana_program::sysvar::clock::Clock;
use anchor_lang::system_program;

declare_id!("3eiMGbcUdSfw3BDfjycT1cay4WYGS3m42QcCQxFYev9S");

#[program]
pub mod htlc {
    use super::*;

    pub fn initialize(
        ctx: Context<InitializeCtx>,
        hashed_secret: [u8; 32],
        delay: u64,
        amount: u64,
    ) -> Result<()> {
        msg!("initialize: owner={}", ctx.accounts.owner.key());
        msg!("initialize: verifier={}", ctx.accounts.verifier.key());
        msg!("initialize: requested amount={}", amount);

        // compute reveal_deadline with a 1-slot buffer to avoid test races
        let clock = Clock::get()?;
        let reveal_timeout = clock.slot.saturating_add(delay).saturating_add(1);
        msg!(
            "initialize: current_slot={} reveal_timeout={}",
            clock.slot,
            reveal_timeout
        );

        // Transfer lamports from owner -> PDA (htlc_pda)
        msg!("initialize: performing transfer of {} lamports", amount);
        let cpi_accounts = system_program::Transfer {
            from: ctx.accounts.owner.to_account_info(),
            to: ctx.accounts.htlc_pda.to_account_info(),
        };
        let cpi_program = ctx.accounts.system_program.to_account_info();
        let cpi_ctx = CpiContext::new(cpi_program, cpi_accounts);
        system_program::transfer(cpi_ctx, amount)?;
        msg!("initialize: transfer succeeded");

        // write state after successful transfer
        let htlc = &mut ctx.accounts.htlc_pda;
        htlc.owner = *ctx.accounts.owner.key;
        htlc.verifier = *ctx.accounts.verifier.key;
        htlc.hashed_secret = hashed_secret;
        htlc.reveal_timeout = reveal_timeout;
        htlc.amount = amount;

        msg!("initialize: state written PDA={}", ctx.accounts.htlc_pda.key());
        Ok(())
    }

    pub fn reveal(ctx: Context<RevealCtx>, secret: String) -> Result<()> {
        let clock = Clock::get()?;
        let htlc = &ctx.accounts.htlc_pda;

        // allow reveal up to and including the stored deadline
        require!(
            clock.slot <= htlc.reveal_timeout,
            HtlcError::RevealAfterTimeout
        );

        // compute keccak-256 of provided secret and compare exactly
        let secret_bytes = secret.into_bytes();
        let computed = keccak::hash(&secret_bytes).0;
        require!(computed == htlc.hashed_secret, HtlcError::InvalidSecret);

        msg!("reveal: secret verified, closing PDA to owner");
        Ok(())
    }

    pub fn timeout(ctx: Context<TimeoutCtx>) -> Result<()> {
        let clock = Clock::get()?;
        let htlc = &ctx.accounts.htlc_pda;

        // allow timeout only strictly after reveal_timeout
        require!(
            clock.slot > htlc.reveal_timeout,
            HtlcError::TimeoutNotReached
        );

        msg!("timeout: deadline passed, closing PDA to verifier");
        Ok(())
    }
}

#[account]
pub struct HtlcPDA {
    pub owner: Pubkey,
    pub verifier: Pubkey,
    pub hashed_secret: [u8; 32],
    pub reveal_timeout: u64,
    pub amount: u64,
}

pub const HTLC_PDA_SIZE: usize = 8 + 32 + 32 + 32 + 8 + 8;

#[derive(Accounts)]
#[instruction(hashed_secret: [u8; 32], delay: u64, amount: u64)]
pub struct InitializeCtx<'info> {
    /// Owner deposits funds; must sign
    #[account(mut)]
    pub owner: Signer<'info>,

    /// CHECK: verifier is only used as PDA seed and reference
    pub verifier: UncheckedAccount<'info>,

    /// PDA that holds state and lamports; created here
    #[account(
        init,
        payer = owner,
        seeds = [owner.key.as_ref(), verifier.key.as_ref()],
        bump,
        space = HTLC_PDA_SIZE
    )]
    pub htlc_pda: Account<'info, HtlcPDA>,

    /// system program for CPI transfer
    pub system_program: Program<'info, System>,
}

#[derive(Accounts)]
pub struct RevealCtx<'info> {
    /// Owner must sign
    #[account(mut)]
    pub owner: Signer<'info>,

    /// CHECK: verifier only used as PDA seed
    pub verifier: UncheckedAccount<'info>,

    /// close to owner on success
    #[account(
        mut,
        seeds = [owner.key.as_ref(), verifier.key.as_ref()],
        bump,
        close = owner
    )]
    pub htlc_pda: Account<'info, HtlcPDA>,
}

#[derive(Accounts)]
pub struct TimeoutCtx<'info> {
    /// Verifier must sign
    #[account(mut)]
    pub verifier: Signer<'info>,

    /// CHECK: owner only used as PDA seed
    pub owner: UncheckedAccount<'info>,

    /// close to verifier on success
    #[account(
        mut,
        seeds = [owner.key.as_ref(), verifier.key.as_ref()],
        bump,
        close = verifier
    )]
    pub htlc_pda: Account<'info, HtlcPDA>,
}

#[error_code]
pub enum HtlcError {
    #[msg("Secret does not match the commitment.")]
    InvalidSecret,
    #[msg("Reveal attempt after timeout.")]
    RevealAfterTimeout,
    #[msg("Timeout not yet reached.")]
    TimeoutNotReached,
}
