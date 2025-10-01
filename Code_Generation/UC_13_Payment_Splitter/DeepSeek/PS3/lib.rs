use anchor_lang::prelude::*;
use anchor_lang::system_program::{transfer, Transfer};

declare_id!("B1v4xeGT3ejpxM5NER1csF335DZypz38byeXy9MWusE3");

#[program]
pub mod payment_splitter {
    use super::*;

    pub fn initialize(
        ctx: Context<Initialize>,
        lamports_to_transfer: u64,
        shares_amounts: Vec<u64>,
    ) -> Result<()> {
        // Validate inputs
        require!(lamports_to_transfer > 0, PaymentSplitterError::InvalidAmount);
        require!(!shares_amounts.is_empty(), PaymentSplitterError::InvalidShares);
        
        let ps_info = &mut ctx.accounts.ps_info;
        let initializer = &ctx.accounts.initializer;
        
        // Get payees from remaining accounts
        let payees: Vec<Pubkey> = ctx.remaining_accounts
            .iter()
            .map(|account| account.key())
            .collect();
            
        // Validate payees and shares
        require!(payees.len() == shares_amounts.len(), PaymentSplitterError::ArrayLengthMismatch);
        require!(payees.len() > 0, PaymentSplitterError::NoPayees);
        
        // Check for duplicate payees
        let mut unique_payees = payees.clone();
        unique_payees.sort();
        unique_payees.dedup();
        require!(unique_payees.len() == payees.len(), PaymentSplitterError::DuplicatePayees);
        
        // Validate shares are positive
        let total_shares: u64 = shares_amounts.iter().sum();
        require!(total_shares > 0, PaymentSplitterError::InvalidShares);
        
        // Initialize payment splitter data
        ps_info.initializer = initializer.key();
        ps_info.payees = payees;
        ps_info.shares_amounts = shares_amounts;
        ps_info.released_amounts = vec![0; ps_info.payees.len()];
        ps_info.current_lamports = lamports_to_transfer;
        ps_info.total_shares = total_shares;
        ps_info.bump = ctx.bumps.ps_info;
        
        // Transfer initial funds to PDA using CPI (this works because the PDA is being initialized)
        let cpi_context = CpiContext::new(
            ctx.accounts.system_program.to_account_info(),
            Transfer {
                from: initializer.to_account_info(),
                to: ps_info.to_account_info(),
            },
        );
        transfer(cpi_context, lamports_to_transfer)?;
        
        emit!(PaymentSplitterInitialized {
            initializer: initializer.key(),
            payees: ps_info.payees.clone(),
            shares: ps_info.shares_amounts.clone(),
            total_lamports: lamports_to_transfer,
            total_shares
        });
        
        Ok(())
    }

    pub fn release(ctx: Context<Release>) -> Result<()> {
        let ps_info = &mut ctx.accounts.ps_info;
        let payee = &ctx.accounts.payee;
        let initializer = &ctx.accounts.initializer;
        
        // Find payee index
        let payee_index = ps_info.payees
            .iter()
            .position(|p| p == payee.key)
            .ok_or(PaymentSplitterError::PayeeNotFound)?;
            
        // Calculate releasable amount
        let total_received = ps_info.current_lamports + ps_info.total_released();
        let total_share_value = total_received
            .checked_mul(ps_info.shares_amounts[payee_index])
            .and_then(|v| v.checked_div(ps_info.total_shares))
            .ok_or(PaymentSplitterError::CalculationError)?;
            
        let already_released = ps_info.released_amounts[payee_index];
        let releasable_amount = total_share_value
            .checked_sub(already_released)
            .ok_or(PaymentSplitterError::CalculationError)?;
            
        require!(releasable_amount > 0, PaymentSplitterError::NothingToRelease);
        
        // Store the amount to release before updating state
        let amount_to_release = releasable_amount;
        let remaining_lamports_after_release = ps_info.current_lamports
            .checked_sub(amount_to_release)
            .ok_or(PaymentSplitterError::CalculationError)?;
        
        // Update state
        ps_info.released_amounts[payee_index] = ps_info.released_amounts[payee_index]
            .checked_add(amount_to_release)
            .ok_or(PaymentSplitterError::CalculationError)?;
        ps_info.current_lamports = remaining_lamports_after_release;
        
        // Transfer funds to payee using direct lamports manipulation
        **ps_info.to_account_info().try_borrow_mut_lamports()? = ps_info
            .to_account_info()
            .lamports()
            .checked_sub(amount_to_release)
            .ok_or(PaymentSplitterError::CalculationError)?;
            
        **payee.try_borrow_mut_lamports()? = payee
            .lamports()
            .checked_add(amount_to_release)
            .ok_or(PaymentSplitterError::CalculationError)?;
        
        emit!(PaymentReleased {
            payee: payee.key(),
            amount: amount_to_release,
            remaining_lamports: remaining_lamports_after_release
        });

        // Check if this was the final release and close the account if empty
        if ps_info.current_lamports == 0 && ps_info.all_funds_distributed() {
            // Close the account and return all lamports (including rent) to initializer
            let ps_info_account = ctx.accounts.ps_info.to_account_info();
            let initializer_account = ctx.accounts.initializer.to_account_info();
            
            let account_lamports = ps_info_account.lamports();
            
            // Transfer all lamports back to initializer
            **ps_info_account.try_borrow_mut_lamports()? = 0;
            **initializer_account.try_borrow_mut_lamports()? = initializer_account
                .lamports()
                .checked_add(account_lamports)
                .ok_or(PaymentSplitterError::CalculationError)?;
            
            emit!(PaymentSplitterClosed {
                initializer: initializer.key(),
                rent_returned: account_lamports
            });
        }
        
        Ok(())
    }

    pub fn force_close(ctx: Context<Close>) -> Result<()> {
        let ps_info = &ctx.accounts.ps_info;
        
        // Verify that all funds have been distributed
        require!(ps_info.current_lamports == 0, PaymentSplitterError::FundsRemaining);
        require!(ps_info.all_funds_distributed(), PaymentSplitterError::FundsRemaining);
        
        // Emit event before closing
        emit!(PaymentSplitterClosed {
            initializer: ctx.accounts.initializer.key(),
            rent_returned: ctx.accounts.ps_info.to_account_info().lamports()
        });
        
        // The account will be automatically closed and rent returned due to the `close = initializer` constraint
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
    
    /// CHECK: This is safe because we only need to reference the initializer
    #[account(mut)]
    pub initializer: UncheckedAccount<'info>,
    
    #[account(
        mut,
        seeds = [b"payment_splitter", initializer.key().as_ref()],
        bump = ps_info.bump
    )]
    pub ps_info: Account<'info, PaymentSplitterInfo>,
    
    pub system_program: Program<'info, System>,
}

#[derive(Accounts)]
pub struct Close<'info> {
    #[account(mut)]
    pub initializer: Signer<'info>,
    
    #[account(
        mut,
        seeds = [b"payment_splitter", initializer.key().as_ref()],
        bump = ps_info.bump,
        close = initializer // This automatically handles rent return
    )]
    pub ps_info: Account<'info, PaymentSplitterInfo>,
    
    pub system_program: Program<'info, System>,
}

#[account]
pub struct PaymentSplitterInfo {
    pub initializer: Pubkey,
    pub current_lamports: u64,  // Funds still in the PDA that haven't been distributed
    pub payees: Vec<Pubkey>,
    pub shares_amounts: Vec<u64>,
    pub released_amounts: Vec<u64>,  // Funds that have been released to each payee
    pub total_shares: u64,
    pub bump: u8,
}

impl PaymentSplitterInfo {
    pub const LEN: usize = 8 + // discriminator
        32 + // initializer
        8 + // current_lamports
        4 + (32 * 10) + // payees vector (up to 10 payees)
        4 + (8 * 10) + // shares_amounts vector (up to 10 payees)
        4 + (8 * 10) + // released_amounts vector (up to 10 payees)
        8 + // total_shares
        1; // bump
    
    pub fn total_released(&self) -> u64 {
        self.released_amounts.iter().sum()
    }
    
    pub fn all_funds_distributed(&self) -> bool {
        // Check if all payees have received their full share
        let total_funds = self.total_released(); // Since current_lamports should be 0 when checking
        self.payees.iter().enumerate().all(|(i, _)| {
            let expected = (total_funds * self.shares_amounts[i]) / self.total_shares;
            self.released_amounts[i] >= expected
        })
    }
}

#[error_code]
pub enum PaymentSplitterError {
    #[msg("Invalid amount provided")]
    InvalidAmount,
    #[msg("Invalid shares distribution")]
    InvalidShares,
    #[msg("Payee and shares arrays must have same length")]
    ArrayLengthMismatch,
    #[msg("No payees provided")]
    NoPayees,
    #[msg("Duplicate payees found")]
    DuplicatePayees,
    #[msg("Payee not found in payment splitter")]
    PayeeNotFound,
    #[msg("Nothing to release for this payee")]
    NothingToRelease,
    #[msg("Calculation error occurred")]
    CalculationError,
    #[msg("Funds still remaining in payment splitter")]
    FundsRemaining,
}

#[event]
pub struct PaymentSplitterInitialized {
    pub initializer: Pubkey,
    pub payees: Vec<Pubkey>,
    pub shares: Vec<u64>,
    pub total_lamports: u64,
    pub total_shares: u64,
}

#[event]
pub struct PaymentReleased {
    pub payee: Pubkey,
    pub amount: u64,
    pub remaining_lamports: u64,
}

#[event]
pub struct PaymentSplitterClosed {
    pub initializer: Pubkey,
    pub rent_returned: u64,
}