use anchor_lang::prelude::*;
use anchor_lang::system_program::{Transfer, transfer};

declare_id!("54sJspoStVyMuTmJr2FZDwzGhsntDJFxxiAVW77yJ4wL");

#[program]
pub mod payment_splitter {
    use super::*;

    pub fn initialize(
        ctx: Context<Initialize>,
        lamports_to_transfer: u64,
        shares_amounts: Vec<u64>,
    ) -> Result<()> {
        let initializer = &ctx.accounts.initializer;
        let ps_info = &mut ctx.accounts.ps_info;
        
        // Validate remaining accounts (payees)
        let payees = ctx.remaining_accounts;
        require!(payees.len() > 0, ErrorCode::NoPayeesProvided);
        require!(payees.len() == shares_amounts.len(), ErrorCode::ArrayLengthMismatch);
        
        // Check for duplicate payees
        let mut seen = std::collections::HashSet::new();
        for payee in payees.iter() {
            let pubkey = payee.key();
            require!(!seen.contains(&pubkey), ErrorCode::DuplicatePayee);
            seen.insert(pubkey);
        }

        // Validate shares
        let total_shares: u64 = shares_amounts.iter().sum();
        require!(total_shares > 0, ErrorCode::ZeroTotalShares);

        // Transfer initial funds to PDA
        let transfer_ctx = CpiContext::new(
            ctx.accounts.system_program.to_account_info(),
            Transfer {
                from: initializer.to_account_info(),
                to: ps_info.to_account_info(),
            },
        );
        transfer(transfer_ctx, lamports_to_transfer)?;

        // Initialize payment splitter data
        ps_info.current_lamports = lamports_to_transfer;
        ps_info.payees = payees.iter().map(|acc| acc.key()).collect();
        ps_info.shares_amounts = shares_amounts;
        ps_info.released_amounts = vec![0; payees.len()];
        ps_info.initializer = initializer.key();
        
        msg!(
            "Payment splitter initialized with {} payees and {} lamports",
            payees.len(),
            lamports_to_transfer
        );
        
        Ok(())
    }

    pub fn release(ctx: Context<Release>) -> Result<()> {
        let payee = &ctx.accounts.payee;
        let ps_info = &mut ctx.accounts.ps_info;
        
        // Find payee index
        let payee_index = ps_info.payees
            .iter()
            .position(|&p| p == payee.key())
            .ok_or(ErrorCode::PayeeNotFound)?;

        // Calculate releasable amount
        let total_received = ps_info.current_lamports + ps_info.released_amounts.iter().sum::<u64>();
        let total_shares: u64 = ps_info.shares_amounts.iter().sum();
        let payee_share = ps_info.shares_amounts[payee_index];
        
        let total_owed = (total_received as u128)
            .checked_mul(payee_share as u128)
            .ok_or(ErrorCode::CalculationOverflow)? 
            .checked_div(total_shares as u128)
            .ok_or(ErrorCode::CalculationOverflow)? as u64;
            
        let already_released = ps_info.released_amounts[payee_index];
        let releasable_amount = total_owed.checked_sub(already_released)
            .ok_or(ErrorCode::NoFundsToRelease)?;

        require!(releasable_amount > 0, ErrorCode::NoFundsToRelease);
        require!(ps_info.current_lamports >= releasable_amount, ErrorCode::InsufficientFunds);

        // Update state before transfer to prevent reentrancy
        ps_info.released_amounts[payee_index] = already_released + releasable_amount;
        ps_info.current_lamports -= releasable_amount;

        // Transfer funds to payee
        **ps_info.to_account_info().try_borrow_mut_lamports()? = ps_info
            .to_account_info()
            .lamports()
            .checked_sub(releasable_amount)
            .ok_or(ErrorCode::InsufficientFunds)?;
            
        **payee.to_account_info().try_borrow_mut_lamports()? = payee
            .to_account_info()
            .lamports()
            .checked_add(releasable_amount)
            .ok_or(ErrorCode::TransferFailed)?;

        msg!("Released {} lamports to payee {}", releasable_amount, payee.key());

        // Check if we should close the account
        let should_close = {
            let all_funds_released = ps_info.current_lamports == 0;
            let total_expected_released: u64 = ps_info.released_amounts.iter().sum();
            let total_expected = (ps_info.current_lamports + total_expected_released) as u128;
            let total_calculated: u128 = ps_info.payees.iter().enumerate().map(|(i, _)| {
                let share = ps_info.shares_amounts[i] as u128;
                (total_expected * share) / (total_shares as u128)
            }).sum();

            all_funds_released && total_expected == total_calculated
        };

        // Close account if needed - WITHOUT transferring to initializer
        if should_close {
            // Simply close the account - any remaining lamports will be lost
            // This is acceptable since we've already verified all funds are distributed
            ps_info.close(ctx.accounts.payee.to_account_info())?;
            msg!("Payment splitter account closed");
        }

        Ok(())
    }
}

#[derive(Accounts)]
pub struct Initialize<'info> {
    #[account(mut)]
    pub initializer: Signer<'info>,
    
    #[account(
        init,
        payer = initializer,
        space = PaymentSplitterInfo::LEN,
        seeds = [b"payment_splitter", initializer.key().as_ref()],
        bump
    )]
    pub ps_info: Account<'info, PaymentSplitterInfo>,
    
    pub system_program: Program<'info, System>,
}

#[derive(Accounts)]
pub struct Release<'info> {
    #[account(mut)]
    pub payee: Signer<'info>,
    
    /// CHECK: This is the initializer who created the payment splitter, used for PDA derivation
    /// We mark this as immutable since we only need it for PDA verification
    pub initializer: AccountInfo<'info>,
    
    #[account(
        mut,
        seeds = [b"payment_splitter", initializer.key().as_ref()],
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
    pub initializer: Pubkey,
}

impl PaymentSplitterInfo {
    // Calculate maximum space needed
    pub const LEN: usize = 8 + // discriminator
        8 + // current_lamports: u64
        4 + (32 * 10) + // payees: Vec<Pubkey> (max 10 payees)
        4 + (8 * 10) +  // shares_amounts: Vec<u64> (max 10 payees)
        4 + (8 * 10) +  // released_amounts: Vec<u64> (max 10 payees)
        32; // initializer: Pubkey
}

#[error_code]
pub enum ErrorCode {
    #[msg("No payees provided")]
    NoPayeesProvided,
    #[msg("Payees and shares arrays must have same length")]
    ArrayLengthMismatch,
    #[msg("Duplicate payee addresses are not allowed")]
    DuplicatePayee,
    #[msg("Total shares cannot be zero")]
    ZeroTotalShares,
    #[msg("Payee not found in payment splitter")]
    PayeeNotFound,
    #[msg("No funds available for release")]
    NoFundsToRelease,
    #[msg("Insufficient funds in payment splitter")]
    InsufficientFunds,
    #[msg("Calculation overflow occurred")]
    CalculationOverflow,
    #[msg("Fund transfer failed")]
    TransferFailed,
}