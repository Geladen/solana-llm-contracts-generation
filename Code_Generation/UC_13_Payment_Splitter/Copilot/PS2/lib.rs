use anchor_lang::prelude::*;

declare_id!("E8TzTKjZjbc35cU7QH8MvntuN1myEmkKnKMUdrzLoDmB");

#[program]
pub mod payment_splitter {
    use super::*;

    pub fn initialize(
        ctx: Context<InitializeCtx>,
        lamports_to_transfer: u64,
        shares_amounts: Vec<u64>,
    ) -> Result<()> {
        // Clone AccountInfos we'll need for CPI before taking mutable borrows
        let initializer_ai = ctx.accounts.initializer.to_account_info().clone();
        let ps_info_ai = ctx.accounts.ps_info.to_account_info().clone();

        // remaining_accounts are the payee accounts supplied by the caller
        let remaining = ctx.remaining_accounts;
        let payees_count = remaining.len();

        require!(payees_count > 0, ErrorCode::NoPayeesProvided);
        require!(
            shares_amounts.len() == payees_count,
            ErrorCode::SharesPayeesLengthMismatch
        );

        // collect payee Pubkeys and check duplicates
        let mut seen: Vec<Pubkey> = Vec::with_capacity(payees_count);
        for acct in remaining.iter() {
            let k = acct.key();
            require!(!seen.contains(&k), ErrorCode::DuplicatePayeeProvided);
            seen.push(k);
        }

        // Now safely borrow mutable ps_info
        let ps = &mut ctx.accounts.ps_info;
        ps.payees = seen;
        ps.shares_amounts = shares_amounts;
        ps.released_amounts = vec![0u64; ps.payees.len()];
        ps.current_lamports = 0u64;

        // Use generated bump
        ps.bump = ctx.bumps.ps_info;

        // Transfer lamports from initializer -> PDA (initializer is system-owned)
        if lamports_to_transfer > 0 {
            let ix = anchor_lang::solana_program::system_instruction::transfer(
                &ctx.accounts.initializer.key(),
                &ps.key(),
                lamports_to_transfer,
            );
            let accounts = [initializer_ai, ps_info_ai];
            let res = anchor_lang::solana_program::program::invoke(&ix, &accounts);
            if res.is_err() {
                return Err(error!(ErrorCode::TransferToPdaFailed));
            }

            ps.current_lamports = ps
                .current_lamports
                .checked_add(lamports_to_transfer)
                .ok_or(ErrorCode::MathOverflow)?;
        }

        Ok(())
    }

    pub fn release(ctx: Context<ReleaseCtx>) -> Result<()> {
        // Acquire AccountInfo handles first
        let ps_info_ai = ctx.accounts.ps_info.to_account_info();
        let payee_ai = ctx.accounts.payee.to_account_info();
        let initializer_ai = ctx.accounts.initializer.to_account_info();

        // Borrow lamports mutably before taking &mut ctx.accounts.ps_info
        let mut from_lamports = ps_info_ai
            .try_borrow_mut_lamports()
            .map_err(|_| error!(ErrorCode::LamportsBorrowFailed))?;
        let mut to_lamports = payee_ai
            .try_borrow_mut_lamports()
            .map_err(|_| error!(ErrorCode::LamportsBorrowFailed))?;

        // Mutable borrow of ps_info state
        let ps = &mut ctx.accounts.ps_info;
        let payee_key = ctx.accounts.payee.key();

        // find payee index
        let idx = ps
            .payees
            .iter()
            .position(|k| k == &payee_key)
            .ok_or(ErrorCode::PayeeNotFound)?;

        // compute totals using checked arithmetic in wide integer space
        let total_shares: u128 = ps
            .shares_amounts
            .iter()
            .map(|&s| s as u128)
            .sum();

        require!(total_shares > 0, ErrorCode::InvalidTotalShares);

        let total_released_sum: u128 = ps
            .released_amounts
            .iter()
            .map(|&r| r as u128)
            .sum();

        let total_received: u128 = (ps.current_lamports as u128)
            .checked_add(total_released_sum)
            .ok_or(ErrorCode::MathOverflow)?;

        let share_of_payee = ps.shares_amounts[idx] as u128;
        let total_due_payee = total_received
            .checked_mul(share_of_payee)
            .ok_or(ErrorCode::MathOverflow)?
            .checked_div(total_shares)
            .ok_or(ErrorCode::MathOverflow)?;

        let already_released = ps.released_amounts[idx] as u128;
        if total_due_payee <= already_released {
            return Err(error!(ErrorCode::NothingToRelease));
        }

        let releasable = total_due_payee
            .checked_sub(already_released)
            .ok_or(ErrorCode::MathOverflow)?;

        let releasable_u64 = u64::try_from(releasable).map_err(|_| ErrorCode::MathOverflow)?;

        require!(
            ps.current_lamports >= releasable_u64,
            ErrorCode::PdaInsufficientFunds
        );

        // perform lamports move using the borrowed refs
        **from_lamports = from_lamports
            .checked_sub(releasable_u64)
            .ok_or(ErrorCode::MathOverflow)?;
        **to_lamports = to_lamports
            .checked_add(releasable_u64)
            .ok_or(ErrorCode::MathOverflow)?;

        // Update bookkeeping
        ps.released_amounts[idx] = ps.released_amounts[idx]
            .checked_add(releasable_u64)
            .ok_or(ErrorCode::MathOverflow)?;
        ps.current_lamports = ps
            .current_lamports
            .checked_sub(releasable_u64)
            .ok_or(ErrorCode::MathOverflow)?;

        // If ps_info account becomes empty of lamports and client expects close, Anchor will
        // allow closing because `close = initializer` is set on the ps_info account.

        drop(from_lamports);
        drop(to_lamports);

        // ensure initializer is writable in the transaction meta (it is marked mut in ReleaseCtx)
        // initializer_ai is present to satisfy borrow ordering; no explicit transfer here.

        Ok(())
    }
}

/// Accounts and state

#[account]
pub struct PaymentSplitterInfo {
    pub current_lamports: u64,
    pub payees: Vec<Pubkey>,
    pub shares_amounts: Vec<u64>,
    pub released_amounts: Vec<u64>,
    pub bump: u8,
}

#[derive(Accounts)]
#[instruction(lamports_to_transfer: u64, shares_amounts: Vec<u64>)]
pub struct InitializeCtx<'info> {
    #[account(mut)]
    pub initializer: Signer<'info>,

    #[account(
        init,
        payer = initializer,
        space = PaymentSplitterInfo::space_for(shares_amounts.len()),
        seeds = [b"payment_splitter".as_ref(), initializer.key().as_ref()],
        bump
    )]
    pub ps_info: Box<Account<'info, PaymentSplitterInfo>>,

    pub system_program: Program<'info, System>,
    // Remaining accounts: payee accounts (their AccountInfo must be supplied)
}

#[derive(Accounts)]
#[instruction()]
pub struct ReleaseCtx<'info> {
    #[account(mut)]
    pub payee: Signer<'info>,

    /// CHECK: initializer is used for seed derivation and to receive lamports on close
    #[account(mut)]
    pub initializer: UncheckedAccount<'info>,

    #[account(
        mut,
        seeds = [b"payment_splitter".as_ref(), initializer.key().as_ref()],
        bump = ps_info.bump,
        close = initializer
    )]
    pub ps_info: Box<Account<'info, PaymentSplitterInfo>>,

    pub system_program: Program<'info, System>,
}

/// Helper: compute required account size for the PDA given number of payees
impl PaymentSplitterInfo {
    pub fn space_for(n: usize) -> usize {
        // discriminator 8
        let discriminator = 8;
        // current_lamports u64
        let current_lamports = 8;
        // bump u8 + padding 7
        let bump = 1 + 7;
        // payees Vec<Pubkey>: 4 (len) + n * 32
        let payees = 4 + n * 32;
        // shares Vec<u64>: 4 + n * 8
        let shares = 4 + n * 8;
        // released Vec<u64>: 4 + n * 8
        let released = 4 + n * 8;
        discriminator + current_lamports + bump + payees + shares + released
    }
}

/// Errors
#[error_code]
pub enum ErrorCode {
    #[msg("No payees provided.")]
    NoPayeesProvided,
    #[msg("Shares vector length must match number of payees.")]
    SharesPayeesLengthMismatch,
    #[msg("Duplicate payee provided.")]
    DuplicatePayeeProvided,
    #[msg("Math overflow occurred.")]
    MathOverflow,
    #[msg("Transfer to PDA failed.")]
    TransferToPdaFailed,
    #[msg("Payee not found.")]
    PayeeNotFound,
    #[msg("Invalid total shares.")]
    InvalidTotalShares,
    #[msg("Nothing to release for this payee.")]
    NothingToRelease,
    #[msg("PDA has insufficient funds.")]
    PdaInsufficientFunds,
    #[msg("Transfer from PDA failed.")]
    TransferFromPdaFailed,
    #[msg("Missing bump for ps_info.")]
    MissingBump,
    #[msg("Failed to borrow lamports.")]
    LamportsBorrowFailed,
}
