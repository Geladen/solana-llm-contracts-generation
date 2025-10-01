use anchor_lang::prelude::*;

declare_id!("BVqNomcMrYteUX5h4F5nmf6EeM6r6CSLAZtT79ZEdNFo");

#[program]
pub mod payment_splitter {
    use super::*;

    pub fn initialize(
        ctx: Context<InitializeCtx>,
        lamports_to_transfer: u64,
        shares_amounts: Vec<u64>,
    ) -> Result<()> {
        // Prepare clones and read-only values before any mutable borrow
        let initializer_ai = ctx.accounts.initializer.to_account_info();
        let system_program_ai = ctx.accounts.system_program.to_account_info();
        let ps_info_ai_clone = ctx.accounts.ps_info.to_account_info().clone();
        let ps_key = ps_info_ai_clone.key();

        let payee_infos = &ctx.remaining_accounts;
        require!(!payee_infos.is_empty(), PaymentSplitterError::NoPayeesProvided);
        require!(
            payee_infos.len() == shares_amounts.len(),
            PaymentSplitterError::SharesAndPayeesLengthMismatch
        );

        // Validate duplicates and collect Pubkeys
        let mut payees: Vec<Pubkey> = Vec::with_capacity(payee_infos.len());
        for acc in payee_infos.iter() {
            let pk = acc.key();
            require!(!payees.contains(&pk), PaymentSplitterError::DuplicatePayee);
            payees.push(pk);
        }

        // Now mutably borrow ps_info and initialize stored state
        let ps = &mut ctx.accounts.ps_info;
        ps.current_lamports = 0u64;
        ps.payees = payees;
        ps.shares_amounts = shares_amounts;
        ps.released_amounts = vec![0u64; ps.shares_amounts.len()];

        // Transfer lamports from initializer -> PDA with CPI
        if lamports_to_transfer > 0 {
            let ix = anchor_lang::solana_program::system_instruction::transfer(
                &ctx.accounts.initializer.key(),
                &ps_key,
                lamports_to_transfer,
            );
            let account_infos = [
                initializer_ai.clone(),
                ps_info_ai_clone.clone(),
                system_program_ai.clone(),
            ];
            anchor_lang::solana_program::program::invoke(&ix, &account_infos)?;
            ps.current_lamports = ps
                .current_lamports
                .checked_add(lamports_to_transfer)
                .ok_or(PaymentSplitterError::MathOverflow)?;
        }

        Ok(())
    }

    pub fn release(ctx: Context<ReleaseCtx>) -> Result<()> {
        // Clone AccountInfos before any mutable borrow of ps_info
        let ps_info_ai_clone = ctx.accounts.ps_info.to_account_info().clone();
        let payee_ai_clone = ctx.accounts.payee.to_account_info().clone();
        let initializer_ai_clone = ctx.accounts.initializer.to_account_info().clone();

        // Mutably borrow program state
        let ps = &mut ctx.accounts.ps_info;

        // Find payee index
        let payee_key = ctx.accounts.payee.key();
        let payee_index = ps
            .payees
            .iter()
            .position(|p| p == &payee_key)
            .ok_or(PaymentSplitterError::PayeeNotFound)?;

        // Compute totals using u128
        let total_shares: u128 = ps.shares_amounts.iter().map(|s| *s as u128).sum();
        require!(total_shares > 0, PaymentSplitterError::ZeroTotalShares);

        let total_released: u128 = ps.released_amounts.iter().map(|r| *r as u128).sum();
        let total_balance: u128 = ps.current_lamports as u128;
        let initial_total: u128 = total_balance
            .checked_add(total_released)
            .ok_or(PaymentSplitterError::MathOverflow)?;

        let payee_share: u128 = ps.shares_amounts[payee_index] as u128;
        let entitlement: u128 = (initial_total
            .checked_mul(payee_share)
            .ok_or(PaymentSplitterError::MathOverflow)?)
            / total_shares;

        let already_released: u128 = ps.released_amounts[payee_index] as u128;
        if entitlement <= already_released {
            return Err(error!(PaymentSplitterError::NothingToRelease));
        }
        let releasable: u128 = entitlement
            .checked_sub(already_released)
            .ok_or(PaymentSplitterError::MathOverflow)?;
        let releasable_u64 = u64::try_from(releasable).map_err(|_| PaymentSplitterError::MathOverflow)?;

        require!(
            ps.current_lamports >= releasable_u64,
            PaymentSplitterError::InsufficientPdaFunds
        );

        // Do lamport transfer by using fresh clones (no move-after-borrow)
        {
            let src = ps_info_ai_clone.clone();
            let dst = payee_ai_clone.clone();

            **src.try_borrow_mut_lamports()? = src
                .lamports()
                .checked_sub(releasable_u64)
                .ok_or(PaymentSplitterError::InsufficientPdaFunds)?;
            **dst.try_borrow_mut_lamports()? = dst
                .lamports()
                .checked_add(releasable_u64)
                .ok_or(PaymentSplitterError::MathOverflow)?;
        }

        // Update bookkeeping
        ps.released_amounts[payee_index] = ps.released_amounts[payee_index]
            .checked_add(releasable_u64)
            .ok_or(PaymentSplitterError::MathOverflow)?;
        ps.current_lamports = ps
            .current_lamports
            .checked_sub(releasable_u64)
            .ok_or(PaymentSplitterError::MathOverflow)?;

        // If fully distributed, transfer leftovers to initializer and zero account data
        let remaining_total_released: u128 = ps.released_amounts.iter().map(|r| *r as u128).sum();
        if remaining_total_released == initial_total {
            // Move leftover lamports defensively
            let remaining = ps_info_ai_clone.lamports();
            if remaining > 0 {
                let src = ps_info_ai_clone.clone();
                let dest = initializer_ai_clone.clone();
                **src.try_borrow_mut_lamports()? = src
                    .lamports()
                    .checked_sub(remaining)
                    .ok_or(PaymentSplitterError::MathOverflow)?;
                **dest.try_borrow_mut_lamports()? = dest
                    .lamports()
                    .checked_add(remaining)
                    .ok_or(PaymentSplitterError::MathOverflow)?;
            }

            // Zero account data safely using a borrow from the cloned AccountInfo
            let mut data = ps_info_ai_clone.try_borrow_mut_data()?;
            for b in data.iter_mut() {
                *b = 0;
            }
        }

        Ok(())
    }
}

#[derive(Accounts)]
#[instruction(lamports_to_transfer: u64, shares_amounts: Vec<u64>)]
pub struct InitializeCtx<'info> {
    #[account(mut)]
    pub initializer: Signer<'info>,

    /// PDA account that stores state and holds funds.
    /// Seeds required exactly as: ["payment_splitter".as_ref(), initializer.key().as_ref()]
    #[account(
        init,
        payer = initializer,
        space = PaymentSplitterInfo::MAX_SPACE,
        seeds = [b"payment_splitter".as_ref(), initializer.key().as_ref()],
        bump
    )]
    pub ps_info: Account<'info, PaymentSplitterInfo>,

    pub system_program: Program<'info, System>,
    // remaining_accounts: used as payee accounts (pubkeys only)
}

#[derive(Accounts)]
pub struct ReleaseCtx<'info> {
    #[account(mut)]
    pub payee: Signer<'info>,

    /// CHECK: This account is used only as the PDA seed reference and as the destination for leftover lamports when closing.
    /// We do not require additional type-level checks because we only read its Pubkey and transfer lamports to it explicitly.
    /// The signer for release is the payee; initializer does not need to be a signer here.
    #[account(mut)]
    pub initializer: UncheckedAccount<'info>,

    /// PDA account (mutable)
    #[account(mut, seeds = [b"payment_splitter".as_ref(), initializer.key().as_ref()], bump)]
    pub ps_info: Account<'info, PaymentSplitterInfo>,

    pub system_program: Program<'info, System>,
}

#[account]
pub struct PaymentSplitterInfo {
    pub current_lamports: u64,
    pub payees: Vec<Pubkey>,
    pub shares_amounts: Vec<u64>,
    pub released_amounts: Vec<u64>,
}

impl PaymentSplitterInfo {
    pub const MAX_PAYEES: usize = 10;
    pub const MAX_SPACE: usize = 8  // discriminator
        + 8 // current_lamports
        + 4 + Self::MAX_PAYEES * 32 // payees vec
        + 4 + Self::MAX_PAYEES * 8 // shares_amounts vec
        + 4 + Self::MAX_PAYEES * 8 // released_amounts vec
        + 32; // padding
}

#[error_code]
pub enum PaymentSplitterError {
    #[msg("No payees provided in remaining accounts")]
    NoPayeesProvided,
    #[msg("Shares vector length must match payees count")]
    SharesAndPayeesLengthMismatch,
    #[msg("Duplicate payee provided")]
    DuplicatePayee,
    #[msg("Math overflow")]
    MathOverflow,
    #[msg("Payee not found in this payment splitter")]
    PayeeNotFound,
    #[msg("Nothing to release for this payee")]
    NothingToRelease,
    #[msg("PDA has insufficient funds to release the calculated amount")]
    InsufficientPdaFunds,
    #[msg("Total shares sum to zero")]
    ZeroTotalShares,
}
