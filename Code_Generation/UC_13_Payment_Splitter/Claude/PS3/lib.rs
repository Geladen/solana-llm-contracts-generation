use anchor_lang::prelude::*;
use anchor_lang::system_program;

declare_id!("CW7CtxykEeLsqHvqEqTBjc9uEzogZwTeJVjNcCrTP357");

#[program]
pub mod payment_splitter {
    use super::*;

    pub fn initialize(
        ctx: Context<InitializeCtx>,
        lamports_to_transfer: u64,
        shares_amounts: Vec<u64>,
    ) -> Result<()> {
        let initializer = &ctx.accounts.initializer;
        let ps_info = &mut ctx.accounts.ps_info;
        let system_program = &ctx.accounts.system_program;

        // Extract payee addresses from remaining accounts
        let payees: Vec<Pubkey> = ctx.remaining_accounts
            .iter()
            .map(|account| account.key())
            .collect();

        // Validate inputs
        require!(payees.len() > 0, PaymentSplitterError::NoPayees);
        require!(
            payees.len() == shares_amounts.len(),
            PaymentSplitterError::MismatchedArrayLengths
        );
        require!(
            shares_amounts.iter().all(|&share| share > 0),
            PaymentSplitterError::InvalidShares
        );

        // Check for duplicate payees
        for i in 0..payees.len() {
            for j in i + 1..payees.len() {
                require!(
                    payees[i] != payees[j],
                    PaymentSplitterError::DuplicatePayee
                );
            }
        }

        // Transfer initial funds from initializer to PDA
        if lamports_to_transfer > 0 {
            let transfer_instruction = system_program::Transfer {
                from: initializer.to_account_info(),
                to: ps_info.to_account_info(),
            };
            let cpi_context = CpiContext::new(
                system_program.to_account_info(),
                transfer_instruction,
            );
            system_program::transfer(cpi_context, lamports_to_transfer)?;
        }

        // Initialize the payment splitter info
        ps_info.current_lamports = lamports_to_transfer;
        ps_info.payees = payees;
        ps_info.shares_amounts = shares_amounts;
        ps_info.released_amounts = vec![0; ps_info.payees.len()];

        Ok(())
    }

    pub fn release(ctx: Context<ReleaseCtx>) -> Result<()> {
        let payee = &ctx.accounts.payee;
        let initializer = &ctx.accounts.initializer;
        let ps_info = &mut ctx.accounts.ps_info;

        // Find payee index
        let payee_index = ps_info
            .payees
            .iter()
            .position(|&p| p == payee.key())
            .ok_or(PaymentSplitterError::PayeeNotFound)?;

        // Calculate total shares
        let total_shares: u64 = ps_info.shares_amounts.iter().sum();

        // Calculate total received (current balance + already released amounts)
        let total_released: u64 = ps_info.released_amounts.iter().sum();
        let total_received = ps_info.current_lamports + total_released;

        // Calculate payee's total share and releasable amount
        let payee_total_share = total_received
            .checked_mul(ps_info.shares_amounts[payee_index])
            .ok_or(PaymentSplitterError::ArithmeticOverflow)?
            .checked_div(total_shares)
            .ok_or(PaymentSplitterError::ArithmeticOverflow)?;

        let releasable_amount = payee_total_share
            .checked_sub(ps_info.released_amounts[payee_index])
            .ok_or(PaymentSplitterError::ArithmeticUnderflow)?;

        require!(releasable_amount > 0, PaymentSplitterError::NoFundsToRelease);

        // Ensure we don't try to transfer more than available
        let transfer_amount = std::cmp::min(releasable_amount, ps_info.current_lamports);

        // Transfer funds to payee
        if transfer_amount > 0 {
            let initializer_key = initializer.key();
            let seeds = &[
                "payment_splitter".as_ref(),
                initializer_key.as_ref(),
                &[ctx.bumps.ps_info],
            ];
            let signer_seeds = &[&seeds[..]];

            **ps_info.to_account_info().try_borrow_mut_lamports()? -= transfer_amount;
            **payee.to_account_info().try_borrow_mut_lamports()? += transfer_amount;

            // Update state
            ps_info.current_lamports = ps_info.current_lamports
                .checked_sub(transfer_amount)
                .ok_or(PaymentSplitterError::ArithmeticUnderflow)?;
            ps_info.released_amounts[payee_index] = ps_info.released_amounts[payee_index]
                .checked_add(transfer_amount)
                .ok_or(PaymentSplitterError::ArithmeticOverflow)?;
        }

        // Check if account should be closed (all funds distributed)
        if ps_info.current_lamports == 0 {
            // Return remaining rent to initializer
            let remaining_lamports = ps_info.to_account_info().lamports();
            if remaining_lamports > 0 {
                **ps_info.to_account_info().try_borrow_mut_lamports()? = 0;
                **initializer.to_account_info().try_borrow_mut_lamports()? += remaining_lamports;
            }
        }

        Ok(())
    }
}

#[derive(Accounts)]
pub struct InitializeCtx<'info> {
    #[account(mut)]
    pub initializer: Signer<'info>,

    #[account(
        init,
        payer = initializer,
        space = PaymentSplitterInfo::SPACE,
        seeds = ["payment_splitter".as_ref(), initializer.key().as_ref()],
        bump
    )]
    pub ps_info: Account<'info, PaymentSplitterInfo>,

    pub system_program: Program<'info, System>,
}

#[derive(Accounts)]
pub struct ReleaseCtx<'info> {
    #[account(mut)]
    pub payee: Signer<'info>,

    /// CHECK: This is used as a reference for PDA derivation and may receive rent refund
    #[account(mut)]
    pub initializer: UncheckedAccount<'info>,

    #[account(
        mut,
        seeds = ["payment_splitter".as_ref(), initializer.key().as_ref()],
        bump
    )]
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
    // Calculate space needed for the account
    // 8 (discriminator) + 8 (current_lamports) + 4 + (32 * max_payees) + 4 + (8 * max_payees) + 4 + (8 * max_payees)
    // Assuming max 10 payees for this example
    pub const SPACE: usize = 8 + 8 + 4 + (32 * 10) + 4 + (8 * 10) + 4 + (8 * 10);
}

#[error_code]
pub enum PaymentSplitterError {
    #[msg("No payees provided")]
    NoPayees,
    #[msg("Payees and shares arrays length mismatch")]
    MismatchedArrayLengths,
    #[msg("All shares must be greater than zero")]
    InvalidShares,
    #[msg("Duplicate payee address found")]
    DuplicatePayee,
    #[msg("Payee not found in payment splitter")]
    PayeeNotFound,
    #[msg("No funds available to release")]
    NoFundsToRelease,
    #[msg("Arithmetic overflow")]
    ArithmeticOverflow,
    #[msg("Arithmetic underflow")]
    ArithmeticUnderflow,
}