use anchor_lang::prelude::*;
use anchor_lang::solana_program::{
    keccak::{hash as keccak256},
    system_program,
    sysvar::clock::Clock,
};

declare_id!("5uHu3tW5fQ9gypiCpuQ6fCjbjNfjTQAaxUYd65ks7mrs");

#[program]
pub mod htlc {
    use super::*;

    /// Initialize an HTLC by locking funds and committing to a secret
    pub fn initialize(
        ctx: Context<InitializeCtx>,
        hashed_secret: [u8; 32],
        delay: u64,
        amount: u64,
    ) -> Result<()> {
        let htlc = &mut ctx.accounts.htlc_info;

        // Ensure the amount matches the lamports transferred
        require!(
            **ctx.accounts.owner.lamports.borrow() >= amount,
            HTLCError::InsufficientFunds
        );

        let clock = Clock::get()?;

        htlc.owner = *ctx.accounts.owner.key;
        htlc.verifier = *ctx.accounts.verifier.key;
        htlc.hashed_secret = hashed_secret;
        htlc.reveal_timeout = clock.slot + delay;
        htlc.amount = amount;

        // Transfer lamports from committer to HTLC PDA
        let cpi_accounts = anchor_lang::solana_program::system_instruction::transfer(
            ctx.accounts.owner.key,
            ctx.accounts.htlc_info.to_account_info().key,
            amount,
        );
        anchor_lang::solana_program::program::invoke(
            &cpi_accounts,
            &[
                ctx.accounts.owner.to_account_info(),
                ctx.accounts.htlc_info.to_account_info(),
                ctx.accounts.system_program.to_account_info(),
            ],
        )?;

        Ok(())
    }

    /// Reveal the secret and redeem funds by committer
    pub fn reveal(ctx: Context<RevealCtx>, secret: String) -> Result<()> {
        let htlc = &mut ctx.accounts.htlc_info;

        // Hash the provided secret
        let secret_hash = keccak256(secret.as_bytes()).0;

        // Verify the hash matches
        require!(
            secret_hash == htlc.hashed_secret,
            HTLCError::InvalidSecret
        );

        let htlc_lamports = **htlc.to_account_info().lamports.borrow();

        // Transfer all HTLC funds to committer
        **htlc.to_account_info().try_borrow_mut_lamports()? -= htlc_lamports;
        **ctx.accounts.owner.to_account_info().try_borrow_mut_lamports()? += htlc_lamports;

        Ok(())
    }

    /// Timeout claim by verifier after reveal_timeout
    pub fn timeout(ctx: Context<TimeoutCtx>) -> Result<()> {
        let htlc = &mut ctx.accounts.htlc_info;
        let clock = Clock::get()?;

        // DEBUG: log current slot and reveal_timeout
        msg!("Current slot: {}", clock.slot);
        msg!("Reveal timeout slot: {}", htlc.reveal_timeout);

        // Ensure timeout has passed
        if clock.slot < htlc.reveal_timeout {
            return err!(HTLCError::TimeoutNotReached);
        }

        let htlc_lamports = **htlc.to_account_info().lamports.borrow();

        // Transfer all HTLC funds to verifier
        **htlc.to_account_info().try_borrow_mut_lamports()? -= htlc_lamports;
        **ctx.accounts.verifier.to_account_info().try_borrow_mut_lamports()? += htlc_lamports;

        Ok(())
    }
}

#[derive(Accounts)]
#[instruction(hashed_secret: [u8; 32], delay: u64, amount: u64)]
pub struct InitializeCtx<'info> {
    #[account(mut)]
    pub owner: Signer<'info>,

    /// CHECK: Verifier account, only used for reference
    pub verifier: UncheckedAccount<'info>,

    #[account(
        init,
        payer = owner,
        space = 8 + 32 + 32 + 32 + 8 + 8, // 120 bytes
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

    /// CHECK: Verifier account, only used for reference
    pub verifier: UncheckedAccount<'info>,

    #[account(
        mut,
        seeds = [owner.key().as_ref(), verifier.key().as_ref()],
        bump
    )]
    pub htlc_info: Account<'info, HtlcPDA>,
}

#[derive(Accounts)]
pub struct TimeoutCtx<'info> {
    #[account(mut)]
    pub verifier: Signer<'info>,

    /// CHECK: Owner account, only used for reference
    pub owner: UncheckedAccount<'info>,

    #[account(
        mut,
        seeds = [owner.key().as_ref(), verifier.key().as_ref()],
        bump
    )]
    pub htlc_info: Account<'info, HtlcPDA>,
}

#[account]
pub struct HtlcPDA {
    pub owner: Pubkey,
    pub verifier: Pubkey,
    pub hashed_secret: [u8; 32],
    pub reveal_timeout: u64,
    pub amount: u64,
}

#[error_code]
pub enum HTLCError {
    #[msg("Insufficient funds to initialize HTLC")]
    InsufficientFunds,
    #[msg("Provided secret does not match commitment")]
    InvalidSecret,
    #[msg("Timeout has not been reached yet")]
    TimeoutNotReached,
}
