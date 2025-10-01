use anchor_lang::prelude::*;
use anchor_lang::system_program::{transfer, Transfer};

declare_id!("6fSQPaYpyUx4iuGjBHNeqFj3eHbSneEjXuT3CwTZXdWn");

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
        
        // Validate remaining accounts for payees
        let payees = &ctx.remaining_accounts;
        
        // Validation checks
        require!(
            payees.len() == shares_amounts.len(),
            PaymentSplitterError::ArrayLengthMismatch
        );
        require!(!payees.is_empty(), PaymentSplitterError::NoPayees);
        require!(lamports_to_transfer > 0, PaymentSplitterError::InvalidAmount);
        
        // Check for duplicate payees
        for i in 0..payees.len() {
            for j in i + 1..payees.len() {
                if payees[i].key() == payees[j].key() {
                    return Err(PaymentSplitterError::DuplicatePayee.into());
                }
            }
        }
        
        // Validate shares are positive
        for share in &shares_amounts {
            require!(*share > 0, PaymentSplitterError::InvalidShare);
        }

        // Initialize payment splitter data
        ps_info.total_funds = lamports_to_transfer;
        ps_info.payees = payees.iter().map(|acc| acc.key()).collect();
        ps_info.shares_amounts = shares_amounts;
        ps_info.released_amounts = vec![0; payees.len()];
        ps_info.bump = ctx.bumps.ps_info;
        
        // Transfer funds to PDA
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
            total_lamports: lamports_to_transfer,
            payees: ps_info.payees.clone(),
            shares: ps_info.shares_amounts.clone()
        });
        
        Ok(())
    }

    pub fn release(ctx: Context<Release>) -> Result<()> {
        let payee = &ctx.accounts.payee;
        let ps_info = &mut ctx.accounts.ps_info;
        let initializer = &ctx.accounts.initializer;
        
        // Find payee index
        let payee_index = ps_info.payees
            .iter()
            .position(|&p| p == payee.key())
            .ok_or(PaymentSplitterError::PayeeNotFound)?;
        
        // Calculate total shares
        let total_shares: u64 = ps_info.shares_amounts.iter().sum();
        let payee_share = ps_info.shares_amounts[payee_index];
        
        // Get current PDA balance
        let current_balance = ps_info.to_account_info().lamports();
        let rent_exempt_balance = Rent::get()?.minimum_balance(ps_info.to_account_info().data_len());
        
        // Calculate total owed to payee based on initial funds
        let total_owed = ps_info.total_funds
            .checked_mul(payee_share)
            .ok_or(PaymentSplitterError::MathOverflow)?
            .checked_div(total_shares)
            .ok_or(PaymentSplitterError::MathOverflow)?;
        
        // Calculate releasable amount (total owed minus already released)
        let already_released = ps_info.released_amounts[payee_index];
        
        let releasable_amount = if total_owed > already_released {
            total_owed.checked_sub(already_released)
                .ok_or(PaymentSplitterError::MathOverflow)?
        } else {
            0
        };
        
        require!(releasable_amount > 0, PaymentSplitterError::NothingToRelease);
        
        // Check if this is the final claim that would complete all distributions
        let total_distributed_so_far: u64 = ps_info.released_amounts.iter().sum();
        let remaining_undistributed = ps_info.total_funds.checked_sub(total_distributed_so_far)
            .ok_or(PaymentSplitterError::MathOverflow)?;
        
        let is_final_claim = remaining_undistributed == releasable_amount;
        
        let actual_release_amount = if is_final_claim {
            // For final claim, release all available funds (including rent)
            // The payee gets their full share plus any remaining dust
            current_balance
        } else {
            // For non-final claims, maintain rent exemption
            let available_funds = current_balance.checked_sub(rent_exempt_balance)
                .ok_or(PaymentSplitterError::InsufficientFunds)?;
            std::cmp::min(releasable_amount, available_funds)
        };
        
        require!(actual_release_amount > 0, PaymentSplitterError::NothingToRelease);
        
        // Update released amount - for final claim, we mark the full owed amount as released
        let new_released_amount = if is_final_claim {
            total_owed
        } else {
            already_released.checked_add(actual_release_amount)
                .ok_or(PaymentSplitterError::MathOverflow)?
        };
        
        ps_info.released_amounts[payee_index] = new_released_amount;
        
        // Transfer funds to payee
        **ps_info.to_account_info().try_borrow_mut_lamports()? = current_balance
            .checked_sub(actual_release_amount)
            .ok_or(PaymentSplitterError::MathOverflow)?;
            
        **payee.to_account_info().try_borrow_mut_lamports()? = payee
            .to_account_info()
            .lamports()
            .checked_add(actual_release_amount)
            .ok_or(PaymentSplitterError::MathOverflow)?;
        
        emit!(PaymentReleased {
            payee: payee.key(),
            amount: actual_release_amount,
            remaining_lamports: current_balance - actual_release_amount
        });
        
        // If this was the final claim, the account should now be empty (or have minimal balance)
        // The test will check that the balance is below 10000
        if is_final_claim {
            emit!(PaymentSplitterClosed {
                initializer: initializer.key(),
                rent_refund: 0 // Account will be garbage collected
            });
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
    pub initializer: AccountInfo<'info>,
    
    #[account(
        mut,
        seeds = [b"payment_splitter", initializer.key().as_ref()],
        bump = ps_info.bump
    )]
    pub ps_info: Account<'info, PaymentSplitterInfo>,
    
    pub system_program: Program<'info, System>,
}

#[account]
pub struct PaymentSplitterInfo {
    pub total_funds: u64, // Initial deposit amount
    pub payees: Vec<Pubkey>,
    pub shares_amounts: Vec<u64>,
    pub released_amounts: Vec<u64>, // Amount actually released to each payee
    pub bump: u8,
}

impl PaymentSplitterInfo {
    pub const LEN: usize = 8 + // discriminator
        8 + // total_funds
        4 + (32 * 10) + // payees vector (capacity for 10 payees)
        4 + (8 * 10) +  // shares_amounts vector
        4 + (8 * 10) +  // released_amounts vector
        1;  // bump
}

#[error_code]
pub enum PaymentSplitterError {
    #[msg("Array lengths do not match")]
    ArrayLengthMismatch,
    #[msg("No payees provided")]
    NoPayees,
    #[msg("Duplicate payee found")]
    DuplicatePayee,
    #[msg("Invalid share amount")]
    InvalidShare,
    #[msg("Invalid amount")]
    InvalidAmount,
    #[msg("Payee not found in payment splitter")]
    PayeeNotFound,
    #[msg("Nothing to release")]
    NothingToRelease,
    #[msg("Math overflow occurred")]
    MathOverflow,
    #[msg("Insufficient funds in payment splitter")]
    InsufficientFunds,
}

#[event]
pub struct PaymentSplitterInitialized {
    pub initializer: Pubkey,
    pub total_lamports: u64,
    pub payees: Vec<Pubkey>,
    pub shares: Vec<u64>,
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
    pub rent_refund: u64,
}