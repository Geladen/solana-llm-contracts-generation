use anchor_lang::prelude::*;
use std::collections::HashSet;

declare_id!("E2VAU6nnFX4o9VQHtC8E3m7Fiezv2BM9K85wKh4ZsWi2");

// --- configuration / sizes ---
const MAX_PAYEES: usize = 16; // safety limit to avoid unbounded account sizes
const DISCRIMINATOR_LEN: usize = 8;
const U64_LEN: usize = 8;
const PUBKEY_LEN: usize = 32;
const VEC_PREFIX_LEN: usize = 4;

// computed space for the account (discriminator + fields)
// PaymentSplitterInfo stores:
// - current_lamports: u64
// - payees: Vec<Pubkey>
// - shares_amounts: Vec<u64>
// - released_amounts: Vec<u64>
const PAYMENT_SPLITTER_INFO_SPACE: usize = DISCRIMINATOR_LEN
    + U64_LEN // current_lamports
    + (VEC_PREFIX_LEN + MAX_PAYEES * PUBKEY_LEN) // payees vec
    + (VEC_PREFIX_LEN + MAX_PAYEES * U64_LEN) // shares_amounts vec
    + (VEC_PREFIX_LEN + MAX_PAYEES * U64_LEN); // released_amounts vec

#[program]
pub mod payment_splitter {
    use super::*;

    /// Initialize payment splitter PDA and deposit initial funds.
    /// - `lamports_to_transfer`: lamports initializer wants to deposit into PDA (in addition to rent paid by init)
    /// - `shares_amounts`: vector of shares, must match the number of payees passed as remaining accounts
pub fn initialize(
    ctx: Context<InitializeCtx>,
    lamports_to_transfer: u64,
    shares_amounts: Vec<u64>,
) -> Result<()> {
    // Clone AccountInfo objects we will need in CPIs BEFORE we immutably/mutably borrow accounts.
    // This prevents borrowing conflicts with later &mut ctx.accounts.ps_info.
    let ps_info_ai = ctx.accounts.ps_info.to_account_info();
    let initializer_ai = ctx.accounts.initializer.to_account_info();
    let system_program_ai = ctx.accounts.system_program.to_account_info();

    // remaining_accounts contain payee AccountInfo entries (their pubkeys are used)
    let payees_count = ctx.remaining_accounts.len();
    require!(payees_count > 0, PaymentSplitterError::NoPayeesProvided);
    require!(
        shares_amounts.len() == payees_count,
        PaymentSplitterError::MismatchedLengths
    );
    require!(
        payees_count <= MAX_PAYEES,
        PaymentSplitterError::TooManyPayees
    );

    // collect payees pubkeys and ensure no duplicates
    let mut seen = std::collections::HashSet::with_capacity(payees_count);
    let mut payees: Vec<Pubkey> = Vec::with_capacity(payees_count);
    for ai in ctx.remaining_accounts.iter() {
        let pk = ai.key();
        if !seen.insert(pk) {
            return err!(PaymentSplitterError::DuplicatePayee);
        }
        payees.push(pk);
    }

    // ensure total shares > 0
    let total_shares: u128 = shares_amounts.iter().map(|&s| s as u128).sum();
    require!(total_shares > 0, PaymentSplitterError::ZeroSharesTotal);

    // Now mutably borrow the account data and initialize it
    let ps = &mut ctx.accounts.ps_info;
    ps.current_lamports = lamports_to_transfer;
    ps.payees = payees;
    ps.shares_amounts = shares_amounts;
    ps.released_amounts = vec![0u64; ps.payees.len()];

    // Transfer lamports from initializer to PDA using CPI (system_program::transfer).
    // Use the cloned AccountInfo objects (ps_info_ai and initializer_ai).
    anchor_lang::system_program::transfer(
        CpiContext::new(system_program_ai, anchor_lang::system_program::Transfer {
            from: initializer_ai,
            to: ps_info_ai,
        }),
        lamports_to_transfer,
    )?;

    msg!(
        "PaymentSplitter initialized: payees={}, deposited={}",
        ps.payees.len(),
        lamports_to_transfer
    );
    Ok(())
}

pub fn release(ctx: Context<ReleaseCtx>) -> Result<()> {
    // Clone AccountInfos we will mutate BEFORE borrowing ps mutably to avoid borrow conflicts.
    let ps_info_ai = ctx.accounts.ps_info.to_account_info();
    let payee_ai = ctx.accounts.payee.to_account_info();
    let initializer_ai = ctx.accounts.initializer.to_account_info();

    // Now borrow the on-chain state mutably.
    let ps = &mut ctx.accounts.ps_info;

    // find payee index
    let payee_key = ctx.accounts.payee.key();
    let idx = ps
        .payees
        .iter()
        .position(|k| k == &payee_key)
        .ok_or(PaymentSplitterError::PayeeNotFound)?;

    // compute totals
    let total_shares: u128 = ps.shares_amounts.iter().map(|&s| s as u128).sum();
    require!(total_shares > 0, PaymentSplitterError::ZeroSharesTotal);

    let total_released: u128 = ps.released_amounts.iter().map(|&r| r as u128).sum();
    // total_received = funds currently in PDA dedicated for splitting + already released amounts
    let total_received: u128 = (ps.current_lamports as u128)
        .checked_add(total_released)
        .ok_or(PaymentSplitterError::MathOverflow)?;

    // compute payee entitlement using integer arithmetic
    let payee_shares = ps.shares_amounts[idx] as u128;
    let payee_entitlement = total_received
        .checked_mul(payee_shares)
        .ok_or(PaymentSplitterError::MathOverflow)?
        .checked_div(total_shares)
        .ok_or(PaymentSplitterError::MathOverflow)?;

    let already_released = ps.released_amounts[idx] as u128;
    let releasable_u128 = payee_entitlement.saturating_sub(already_released);

    let releasable: u64 = u64::try_from(releasable_u128)
        .map_err(|_| PaymentSplitterError::MathOverflow)?;
    require!(releasable > 0, PaymentSplitterError::NothingToRelease);

    // === transfer lamports safely ===
    {
        let mut from_lamports = ps_info_ai.try_borrow_mut_lamports()?;
        require!(**from_lamports >= releasable, PaymentSplitterError::InsufficientFunds);

        let new_from = (**from_lamports)
            .checked_sub(releasable)
            .ok_or(PaymentSplitterError::InsufficientFunds)?;
        **from_lamports = new_from;
    }

    {
        let mut to_lamports = payee_ai.try_borrow_mut_lamports()?;
        let new_to = (**to_lamports)
            .checked_add(releasable)
            .ok_or(PaymentSplitterError::MathOverflow)?;
        **to_lamports = new_to;
    }

    // === update bookkeeping ===
    ps.released_amounts[idx] = ps.released_amounts[idx]
        .checked_add(releasable)
        .ok_or(PaymentSplitterError::MathOverflow)?;
    ps.current_lamports = ps
        .current_lamports
        .checked_sub(releasable)
        .ok_or(PaymentSplitterError::MathOverflow)?;

    msg!(
        "Released {} lamports to {} (index {})",
        releasable,
        payee_key,
        idx
    );

    // If we have exhausted distributable funds, close the PDA to return rent to initializer.
    if ps.current_lamports == 0 {
        ps.close(initializer_ai)?;
        msg!("PaymentSplitter PDA closed and rent returned to initializer");
    }

    Ok(())
}


}

// ------------------------ Accounts Contexts ------------------------

#[derive(Accounts)]
pub struct InitializeCtx<'info> {
    /// The initializer who pays for account creation and the initial deposit.
    #[account(mut)]
    pub initializer: Signer<'info>,

    /// PaymentSplitter PDA — must be created with seeds:
    /// seeds = [b"payment_splitter", initializer.key().as_ref()]
    #[account(
        init,
        payer = initializer,
        space = PAYMENT_SPLITTER_INFO_SPACE,
        seeds = [b"payment_splitter", initializer.key().as_ref()],
        bump
    )]
    pub ps_info: Account<'info, PaymentSplitterInfo>,

    /// System program (transfer CPI)
    pub system_program: Program<'info, System>,
}

#[derive(Accounts)]
pub struct ReleaseCtx<'info> {
    /// Payee claiming funds (must sign)
    #[account(mut)]
    pub payee: Signer<'info>,

    /// Reference to initializer (used in PDA derivation; not required to be signer here)
    #[account(mut)]
    pub initializer: SystemAccount<'info>,

    /// PaymentSplitter PDA (validated by seeds exactly as required)
    #[account(mut, seeds = [b"payment_splitter", initializer.key().as_ref()], bump)]
    pub ps_info: Account<'info, PaymentSplitterInfo>,

    /// System program (included to match CPI pattern / Anchor best-practices even though we mutate lamports directly)
    pub system_program: Program<'info, System>,
}

// ------------------------ Stored Account ------------------------

/// The on-chain state (exact fields requested)
#[account]
pub struct PaymentSplitterInfo {
    pub current_lamports: u64,
    pub payees: Vec<Pubkey>,
    pub shares_amounts: Vec<u64>,
    pub released_amounts: Vec<u64>,
}

// ------------------------ Errors ------------------------

#[error_code]
pub enum PaymentSplitterError {
    #[msg("Duplicate payees are not allowed")]
    DuplicatePayee,
    #[msg("Number of payees and shares arrays must match")]
    MismatchedLengths,
    #[msg("Too many payees (exceeds MAX_PAYEES)")]
    TooManyPayees,
    #[msg("Total shares must be greater than zero")]
    ZeroSharesTotal,
    #[msg("Insufficient funds in PDA for requested release")]
    InsufficientFunds,
    #[msg("Caller is not registered as a payee")]
    PayeeNotFound,
    #[msg("Nothing to release for this payee")]
    NothingToRelease,
    #[msg("Math overflow or checked arithmetic failed")]
    MathOverflow,
    #[msg("No payees provided in remaining accounts")]
    NoPayeesProvided,
}
