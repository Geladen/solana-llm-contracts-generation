use anchor_lang::prelude::*;
use anchor_lang::solana_program::{program::invoke_signed, system_instruction};

declare_id!("HDdmtz5XGU6cN7EzLDi6VSD3UVBRJFt5x9ZctbmQSPtt");

// set max number of payees the PDA can store
pub const MAX_PAYEES: usize = 10;

#[program]
pub mod payment_splitter {
    use super::*;

    pub fn initialize(
        ctx: Context<InitializeCtx>,
        lamports_to_transfer: u64,
        shares_amounts: Vec<u64>,
    ) -> Result<()> {
        let payee_ais = &ctx.remaining_accounts;

        require!(
            !payee_ais.is_empty(),
            PaymentSplitterError::NoPayeesProvided
        );
        require!(
            payee_ais.len() == shares_amounts.len(),
            PaymentSplitterError::PayeesSharesLengthMismatch
        );
        require!(
            payee_ais.len() <= MAX_PAYEES,
            PaymentSplitterError::TooManyPayees
        );

        let mut payees: Vec<Pubkey> = Vec::with_capacity(payee_ais.len());
        for ai in payee_ais.iter() {
            let pk = *ai.key;
            if payees.contains(&pk) {
                return err!(PaymentSplitterError::DuplicatePayee);
            }
            payees.push(pk);
        }

        let ps = &mut ctx.accounts.ps_info;
        ps.current_lamports = lamports_to_transfer;
        ps.payees = payees;
        ps.shares_amounts = shares_amounts.clone();
        ps.released_amounts = vec![0u64; shares_amounts.len()];

        // Transfer lamports into PDA
        if lamports_to_transfer > 0 {
            let ix = system_instruction::transfer(
                &ctx.accounts.initializer.key(),
                &ps.to_account_info().key(),
                lamports_to_transfer,
            );
            invoke_signed(
                &ix,
                &[
                    ctx.accounts.initializer.to_account_info(),
                    ps.to_account_info(),
                    ctx.accounts.system_program.to_account_info(),
                ],
                &[],
            )?;
        }

        Ok(())
    }

    pub fn release(ctx: Context<ReleaseCtx>) -> Result<()> {
        let payee_key = ctx.accounts.payee.key();
        let initializer_key = ctx.accounts.initializer.key();
        let ps = &mut ctx.accounts.ps_info;

        let idx = ps
            .payees
            .iter()
            .position(|k| k == &payee_key)
            .ok_or(PaymentSplitterError::PayeeNotFound)?;

        let total_shares: u128 = ps.shares_amounts.iter().map(|s| *s as u128).sum();
        let total_released: u128 = ps.released_amounts.iter().map(|r| *r as u128).sum();
        let total_received: u128 = (ps.current_lamports as u128)
            .checked_add(total_released)
            .ok_or(PaymentSplitterError::Overflow)?;
        let entitled = total_received
            .checked_mul(ps.shares_amounts[idx] as u128)
            .ok_or(PaymentSplitterError::Overflow)?
            .checked_div(total_shares)
            .ok_or(PaymentSplitterError::DivideByZero)?;
        let already_released = ps.released_amounts[idx] as u128;
        if entitled <= already_released {
            return err!(PaymentSplitterError::NothingToRelease);
        }
        let to_release = (entitled - already_released) as u64;

        require!(
            ps.current_lamports >= to_release,
            PaymentSplitterError::InsufficientPdaBalance
        );

        // -------------------------------
        // Manual lamports transfer
        // -------------------------------
        let ps_ai = ps.to_account_info(); // ✅ use mutable ref
        let payee_ai = ctx.accounts.payee.to_account_info();

        **ps_ai.try_borrow_mut_lamports()? -= to_release;
        **payee_ai.try_borrow_mut_lamports()? += to_release;

        ps.released_amounts[idx] = ps
            .released_amounts[idx]
            .checked_add(to_release)
            .ok_or(PaymentSplitterError::Overflow)?;
        ps.current_lamports = ps
            .current_lamports
            .checked_sub(to_release)
            .ok_or(PaymentSplitterError::Overflow)?;

        Ok(())
    }
}

// ---------------------------------
// Account Contexts
// ---------------------------------

#[derive(Accounts)]
pub struct InitializeCtx<'info> {
    #[account(mut)]
    pub initializer: Signer<'info>,

    #[account(
        init,
        payer = initializer,
        space = PaymentSplitterInfo::calc_space_for(MAX_PAYEES),
        seeds = [b"payment_splitter".as_ref(), initializer.key().as_ref()],
        bump
    )]
    pub ps_info: Account<'info, PaymentSplitterInfo>,

    pub system_program: Program<'info, System>,
}

#[derive(Accounts)]
pub struct ReleaseCtx<'info> {
    #[account(mut)]
    pub payee: Signer<'info>,

    /// CHECK: used as PDA derivation and close destination
    #[account(mut)] // <-- mark mutable so lamports can be added on close
    pub initializer: UncheckedAccount<'info>,

    #[account(
        mut,
        seeds = [b"payment_splitter".as_ref(), initializer.key().as_ref()],
        bump,
        close = initializer
    )]
    pub ps_info: Account<'info, PaymentSplitterInfo>,

    pub system_program: Program<'info, System>,
}

// ---------------------------------
// Account Data
// ---------------------------------

#[account]
pub struct PaymentSplitterInfo {
    pub current_lamports: u64,
    pub payees: Vec<Pubkey>,
    pub shares_amounts: Vec<u64>,
    pub released_amounts: Vec<u64>,
}

impl PaymentSplitterInfo {
    pub fn calc_space_for(n: usize) -> usize {
        let discr = 8; // anchor discriminator
        let lamports = 8;
        let payees = 4 + n * 32;
        let shares = 4 + n * 8;
        let released = 4 + n * 8;
        discr + lamports + payees + shares + released
    }
}

// ---------------------------------
// Errors
// ---------------------------------

#[error_code]
pub enum PaymentSplitterError {
    #[msg("No payees provided.")]
    NoPayeesProvided,
    #[msg("Payees and shares length mismatch.")]
    PayeesSharesLengthMismatch,
    #[msg("Too many payees (exceeds MAX_PAYEES).")]
    TooManyPayees,
    #[msg("Duplicate payee detected.")]
    DuplicatePayee,
    #[msg("Payee not found.")]
    PayeeNotFound,
    #[msg("Total shares is zero.")]
    TotalSharesZero,
    #[msg("Nothing to release.")]
    NothingToRelease,
    #[msg("Overflow error.")]
    Overflow,
    #[msg("Divide by zero.")]
    DivideByZero,
    #[msg("Insufficient PDA balance.")]
    InsufficientPdaBalance,
}
