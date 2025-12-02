use anchor_lang::prelude::*;
use anchor_lang::solana_program::{keccak};

declare_id!("8JTGYRpj23beoJ6JqXC3aGnSTEV6vvTDVNzQvUzYGzr9");

#[program]
pub mod htlc {
    use super::*;

    /// Initialize a new HTLC
    pub fn initialize(
        ctx: Context<InitializeCtx>,
        hashed_secret: [u8; 32],
        delay: u64,
        amount: u64,
    ) -> Result<()> {
        let htlc = &mut ctx.accounts.htlc_info;

        htlc.owner = ctx.accounts.owner.key();
        htlc.verifier = ctx.accounts.verifier.key();
        htlc.hashed_secret = hashed_secret;
        let current_slot = Clock::get()?.slot;
        htlc.reveal_timeout = current_slot + delay;
        htlc.amount = amount;

        // NO manual lamports transfer needed!
        // The `init` attribute already funds the PDA with enough lamports from `owner`
        Ok(())
    }

    /// Reveal the secret to claim HTLC funds
    pub fn reveal(ctx: Context<RevealCtx>, secret: String) -> Result<()> {
        let htlc = &mut ctx.accounts.htlc_info;

        // Verify hash
        let hash = keccak::hash(secret.as_bytes()).0;
        require!(hash == htlc.hashed_secret, HtlcError::InvalidSecret);

        let amount = htlc.amount;
        require!(amount > 0, HtlcError::AlreadyClaimed);
        htlc.amount = 0;

        // Transfer lamports from PDA to owner
        **ctx.accounts.htlc_info.to_account_info().try_borrow_mut_lamports()? -= amount;
        **ctx.accounts.owner.try_borrow_mut_lamports()? += amount;

        Ok(())
    }

    /// Timeout claim after reveal deadline
    pub fn timeout(ctx: Context<TimeoutCtx>) -> Result<()> {
        let htlc = &mut ctx.accounts.htlc_info;
        let clock = Clock::get()?;
        require!(clock.slot > htlc.reveal_timeout, HtlcError::TimeoutNotReached);

        let amount = htlc.amount;
        require!(amount > 0, HtlcError::AlreadyClaimed);
        htlc.amount = 0;

        // Transfer lamports from PDA to verifier
        **ctx.accounts.htlc_info.to_account_info().try_borrow_mut_lamports()? -= amount;
        **ctx.accounts.verifier.try_borrow_mut_lamports()? += amount;

        Ok(())
    }
}

#[derive(Accounts)]
#[instruction(hashed_secret: [u8;32], delay: u64, amount: u64)]
pub struct InitializeCtx<'info> {
    /// CHECK: The committer signing the transaction
    #[account(mut, signer)]
    pub owner: AccountInfo<'info>,

    /// CHECK: The receiver/verifier
    pub verifier: AccountInfo<'info>,

    #[account(
        init,
        seeds = [owner.key().as_ref(), verifier.key().as_ref()],
        bump,
        payer = owner,
        space = 8 + std::mem::size_of::<HtlcPDA>(), // 8 for discriminator
    )]
    pub htlc_info: Account<'info, HtlcPDA>,

    pub system_program: Program<'info, System>,
}

#[derive(Accounts)]
pub struct RevealCtx<'info> {
    /// CHECK: Must be the owner (committer) signing
    #[account(mut, signer)]
    pub owner: AccountInfo<'info>,

    /// CHECK: Verifier reference
    pub verifier: AccountInfo<'info>,

    #[account(
        mut,
        seeds = [owner.key().as_ref(), verifier.key().as_ref()],
        bump
    )]
    pub htlc_info: Account<'info, HtlcPDA>,
}

#[derive(Accounts)]
pub struct TimeoutCtx<'info> {
    /// CHECK: Must be the verifier (receiver) signing
    #[account(mut, signer)]
    pub verifier: AccountInfo<'info>,

    /// CHECK: Owner reference
    pub owner: AccountInfo<'info>,

    #[account(
        mut,
        seeds = [owner.key().as_ref(), verifier.key().as_ref()],
        bump
    )]
    pub htlc_info: Account<'info, HtlcPDA>,
}

#[account]
pub struct HtlcPDA {
    pub owner: Pubkey,            // 32
    pub verifier: Pubkey,         // 32
    pub hashed_secret: [u8; 32],  // 32
    pub reveal_timeout: u64,      // 8
    pub amount: u64,              // 8
}

#[error_code]
pub enum HtlcError {
    #[msg("The secret provided does not match the committed hash.")]
    InvalidSecret,

    #[msg("Reveal timeout has not yet been reached.")]
    TimeoutNotReached,

    #[msg("HTLC funds already claimed.")]
    AlreadyClaimed,
}
