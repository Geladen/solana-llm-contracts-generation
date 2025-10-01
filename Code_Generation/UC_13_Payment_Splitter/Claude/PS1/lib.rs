use anchor_lang::prelude::*;

declare_id!("GNNh1Xmay2yiCUHGXgwtDuuXQCoDXemLAqGDfZoeWcuk");

#[program]
pub mod payment_splitter {
    use super::*;

    /// Initialize the payment splitter with payees and their shares
    pub fn initialize(
        ctx: Context<InitializeCtx>,
        lamports_to_transfer: u64,
        shares_amounts: Vec<u64>,
    ) -> Result<()> {
        let remaining_accounts = &ctx.remaining_accounts;
        
        // Validate input parameters
        require!(
            !remaining_accounts.is_empty(),
            PaymentSplitterError::NoPayeesProvided
        );
        require!(
            remaining_accounts.len() == shares_amounts.len(),
            PaymentSplitterError::PayeesSharesMismatch
        );
        require!(
            shares_amounts.iter().all(|&share| share > 0),
            PaymentSplitterError::InvalidShares
        );
        require!(
            lamports_to_transfer > 0,
            PaymentSplitterError::InvalidTransferAmount
        );

        // Extract payee addresses and validate for duplicates
        let mut payees = Vec::new();
        for account in remaining_accounts.iter() {
            let payee_key = account.key();
            require!(
                !payees.contains(&payee_key),
                PaymentSplitterError::DuplicatePayee
            );
            payees.push(payee_key);
        }

        // Initialize released_amounts vector with zeros
        let released_amounts = vec![0u64; payees.len()];

        // Transfer initial funds from initializer to PDA using system program transfer
        let transfer_ctx = CpiContext::new(
            ctx.accounts.system_program.to_account_info(),
            anchor_lang::system_program::Transfer {
                from: ctx.accounts.initializer.to_account_info(),
                to: ctx.accounts.ps_info.to_account_info(),
            },
        );
        anchor_lang::system_program::transfer(transfer_ctx, lamports_to_transfer)?;

        // Initialize the PaymentSplitterInfo account
        let ps_info = &mut ctx.accounts.ps_info;
        ps_info.current_lamports = lamports_to_transfer;
        ps_info.payees = payees;
        ps_info.shares_amounts = shares_amounts;
        ps_info.released_amounts = released_amounts;

        Ok(())
    }

    /// Release payment to a payee
    pub fn release(ctx: Context<ReleaseCtx>) -> Result<()> {
        let payee_key = ctx.accounts.payee.key();

        // Find payee index and calculate releasable amount
        let (payee_index, releasable_amount) = {
            let ps_info = &ctx.accounts.ps_info;
            
            let payee_index = ps_info
                .payees
                .iter()
                .position(|&p| p == payee_key)
                .ok_or(PaymentSplitterError::PayeeNotFound)?;

            let releasable_amount = calculate_releasable_amount(
                ps_info,
                payee_index,
            )?;

            require!(
                releasable_amount > 0,
                PaymentSplitterError::NoFundsToRelease
            );

            (payee_index, releasable_amount)
        };

        // Prepare seeds for PDA signing
        let initializer_key = ctx.accounts.initializer.key();
        let seeds = &[
            b"payment_splitter".as_ref(),
            initializer_key.as_ref(),
            &[ctx.bumps.ps_info],
        ];
        let signer_seeds = &[&seeds[..]];

        // Transfer funds to payee using manual lamport transfer
        **ctx.accounts.ps_info.to_account_info().try_borrow_mut_lamports()? -= releasable_amount;
        **ctx.accounts.payee.to_account_info().try_borrow_mut_lamports()? += releasable_amount;

        // Update state after successful transfer
        let ps_info = &mut ctx.accounts.ps_info;
        
        // Update released amounts
        ps_info.released_amounts[payee_index] = ps_info.released_amounts[payee_index]
            .checked_add(releasable_amount)
            .ok_or(PaymentSplitterError::ArithmeticOverflow)?;

        // Update current lamports
        ps_info.current_lamports = ps_info.current_lamports
            .checked_sub(releasable_amount)
            .ok_or(PaymentSplitterError::InsufficientFunds)?;

        // Check if all funds have been released and close account if needed
        if ps_info.current_lamports == 0 {
            // All tracked funds released, close the account and transfer remaining rent to payee
            let remaining_lamports = ctx.accounts.ps_info.to_account_info().lamports();
            
            if remaining_lamports > 0 {
                // Transfer all remaining lamports (rent) to the current payee
                **ctx.accounts.ps_info.to_account_info().try_borrow_mut_lamports()? = 0;
                **ctx.accounts.payee.to_account_info().try_borrow_mut_lamports()? += remaining_lamports;
            }
            
            msg!("Payment splitter account closed - all funds distributed");
        }

        Ok(())
    }
}

/// Calculate the amount that can be released to a specific payee
fn calculate_releasable_amount(
    ps_info: &PaymentSplitterInfo,
    payee_index: usize,
) -> Result<u64> {
    let total_shares: u64 = ps_info.shares_amounts.iter().sum();
    let payee_shares = ps_info.shares_amounts[payee_index];
    let already_released = ps_info.released_amounts[payee_index];

    // Calculate total amount that should be available to this payee
    let total_received = calculate_total_received(ps_info)?;
    let payee_total_allocation = total_received
        .checked_mul(payee_shares)
        .ok_or(PaymentSplitterError::ArithmeticOverflow)?
        .checked_div(total_shares)
        .ok_or(PaymentSplitterError::ArithmeticOverflow)?;

    // Calculate releasable amount
    let releasable = payee_total_allocation
        .checked_sub(already_released)
        .ok_or(PaymentSplitterError::ArithmeticOverflow)?;

    Ok(releasable)
}

/// Calculate total amount received by the payment splitter
fn calculate_total_received(ps_info: &PaymentSplitterInfo) -> Result<u64> {
    let total_released: u64 = ps_info.released_amounts.iter().sum();
    let total_received = ps_info.current_lamports
        .checked_add(total_released)
        .ok_or(PaymentSplitterError::ArithmeticOverflow)?;

    Ok(total_received)
}

#[derive(Accounts)]
pub struct InitializeCtx<'info> {
    #[account(mut)]
    pub initializer: Signer<'info>,

    #[account(
        init,
        payer = initializer,
        space = PaymentSplitterInfo::calculate_space(10), // Max 10 payees for space estimation
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

    /// CHECK: This is used only as a reference for PDA derivation
    pub initializer: UncheckedAccount<'info>,

    #[account(
        mut,
        seeds = [b"payment_splitter".as_ref(), initializer.key().as_ref()],
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
    pub fn calculate_space(max_payees: usize) -> usize {
        8 + // discriminator
        8 + // current_lamports
        4 + (32 * max_payees) + // payees vector
        4 + (8 * max_payees) + // shares_amounts vector
        4 + (8 * max_payees) // released_amounts vector
    }
}

#[error_code]
pub enum PaymentSplitterError {
    #[msg("No payees provided")]
    NoPayeesProvided,
    #[msg("Payees and shares arrays length mismatch")]
    PayeesSharesMismatch,
    #[msg("Invalid shares: all shares must be greater than 0")]
    InvalidShares,
    #[msg("Invalid transfer amount: must be greater than 0")]
    InvalidTransferAmount,
    #[msg("Duplicate payee found")]
    DuplicatePayee,
    #[msg("Payee not found in the payment splitter")]
    PayeeNotFound,
    #[msg("No funds available to release")]
    NoFundsToRelease,
    #[msg("Insufficient funds in the payment splitter")]
    InsufficientFunds,
    #[msg("Arithmetic overflow occurred")]
    ArithmeticOverflow,
}