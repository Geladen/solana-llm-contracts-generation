use anchor_lang::prelude::*;

declare_id!("BQCCmgjNjSNMT9RtG5TmW13sqjQ3ma7e2pq6hgZiopdx");

#[program]
pub mod payment_splitter {
    use super::*;

    /// Initialize the payment splitter with payees and their shares
    pub fn initialize(
        ctx: Context<InitializeCtx>,
        lamports_to_transfer: u64,
        shares_amounts: Vec<u64>,
    ) -> Result<()> {
        let initializer = &ctx.accounts.initializer;
        let ps_info = &mut ctx.accounts.ps_info;

        // Extract payees from remaining accounts
        let payees: Vec<Pubkey> = ctx.remaining_accounts
            .iter()
            .map(|account| account.key())
            .collect();

        // Validation checks
        require!(
            payees.len() > 0,
            PaymentSplitterError::NoPayees
        );
        
        require!(
            payees.len() == shares_amounts.len(),
            PaymentSplitterError::PayeesSharesMismatch
        );

        // Check for duplicate payees
        for i in 0..payees.len() {
            for j in (i + 1)..payees.len() {
                require!(
                    payees[i] != payees[j],
                    PaymentSplitterError::DuplicatePayee
                );
            }
        }

        // Validate shares are not zero
        for &share in &shares_amounts {
            require!(
                share > 0,
                PaymentSplitterError::ZeroShares
            );
        }

        // Initialize the payment splitter info first
        ps_info.current_lamports = 0; // Will be updated after transfer
        ps_info.payees = payees;
        ps_info.shares_amounts = shares_amounts;
        ps_info.released_amounts = vec![0u64; ps_info.payees.len()];

        // Transfer initial funds from initializer to PDA if any
        if lamports_to_transfer > 0 {
            let transfer_ix = anchor_lang::system_program::Transfer {
                from: initializer.to_account_info(),
                to: ps_info.to_account_info(),
            };
            
            anchor_lang::system_program::transfer(
                CpiContext::new(
                    ctx.accounts.system_program.to_account_info(),
                    transfer_ix,
                ),
                lamports_to_transfer,
            )?;
            
            // Update current_lamports to reflect the business balance only
            ps_info.current_lamports = lamports_to_transfer;
        }

        emit!(PaymentSplitterInitialized {
            initializer: initializer.key(),
            total_shares: ps_info.shares_amounts.iter().sum::<u64>(),
            initial_lamports: lamports_to_transfer,
            payee_count: ps_info.payees.len() as u64,
        });

        Ok(())
    }

    /// Release payment to a specific payee
    pub fn release(ctx: Context<ReleaseCtx>) -> Result<()> {
        let payee = &ctx.accounts.payee;
        let ps_info = &mut ctx.accounts.ps_info;

        // Find payee index
        let payee_index = ps_info.payees
            .iter()
            .position(|&p| p == payee.key())
            .ok_or(PaymentSplitterError::PayeeNotFound)?;

        // Calculate total received - but we need to be more careful about what we include
        // The account balance includes rent exemption, so we need to exclude that from calculations
        let rent = Rent::get()?;
        let min_rent_exempt = rent.minimum_balance(ps_info.to_account_info().data_len());
        
        // Available distributable funds = current account balance - rent exemption
        let account_balance = ps_info.to_account_info().lamports();
        let distributable_funds = account_balance.saturating_sub(min_rent_exempt);
        
        // Total that has been received for distribution = current distributable + already released
        let total_released = ps_info.released_amounts.iter().sum::<u64>();
        let total_received = distributable_funds + total_released;

        // Calculate total shares
        let total_shares: u64 = ps_info.shares_amounts.iter().sum();

        // Calculate payee's total allocation
        let payee_total_allocation = (total_received as u128 * ps_info.shares_amounts[payee_index] as u128 / total_shares as u128) as u64;

        // Calculate releasable amount
        let already_released = ps_info.released_amounts[payee_index];
        
        require!(
            payee_total_allocation >= already_released,
            PaymentSplitterError::InsufficientFunds
        );

        let releasable = payee_total_allocation - already_released;

        require!(
            releasable > 0,
            PaymentSplitterError::NoFundsToRelease
        );

        // Check against distributable funds rather than current_lamports
        require!(
            distributable_funds >= releasable,
            PaymentSplitterError::InsufficientBalance
        );

        // Update the business logic state first
        ps_info.released_amounts[payee_index] += releasable;
        // Update current_lamports to track remaining distributable funds
        ps_info.current_lamports = distributable_funds - releasable;

        // Perform the actual SOL transfer
        let account_info = ps_info.to_account_info();
        **account_info.try_borrow_mut_lamports()? -= releasable;
        **payee.to_account_info().try_borrow_mut_lamports()? += releasable;

        emit!(PaymentReleased {
            payee: payee.key(),
            amount: releasable,
            remaining_balance: ps_info.current_lamports,
        });

        // Close account if no distributable funds remaining
        if ps_info.current_lamports == 0 {
            let all_released = ps_info.released_amounts
                .iter()
                .zip(ps_info.shares_amounts.iter())
                .all(|(&released, &shares)| {
                    let total_received = ps_info.released_amounts.iter().sum::<u64>();
                    let total_shares: u64 = ps_info.shares_amounts.iter().sum();
                    if total_shares > 0 {
                        let expected = (total_received as u128 * shares as u128 / total_shares as u128) as u64;
                        released >= expected
                    } else {
                        true
                    }
                });

            if all_released {
                // Return remaining rent to initializer
                let account_info = ps_info.to_account_info();
                let remaining_lamports = account_info.lamports();
                if remaining_lamports > 0 {
                    **account_info.try_borrow_mut_lamports()? = 0;
                    **ctx.accounts.initializer.to_account_info().try_borrow_mut_lamports()? += remaining_lamports;
                }

                emit!(PaymentSplitterClosed {
                    initializer: ctx.accounts.initializer.key(),
                });
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
        space = PaymentSplitterInfo::space(10), // Accommodate up to 10 payees initially
        seeds = [b"payment_splitter", initializer.key().as_ref()],
        bump
    )]
    pub ps_info: Account<'info, PaymentSplitterInfo>,

    pub system_program: Program<'info, System>,
}

#[derive(Accounts)]
pub struct ReleaseCtx<'info> {
    #[account(mut)]
    pub payee: Signer<'info>,

    /// CHECK: This account is used only for PDA seed derivation
    pub initializer: UncheckedAccount<'info>,

    #[account(
        mut,
        seeds = [b"payment_splitter", initializer.key().as_ref()],
        bump,
        realloc = PaymentSplitterInfo::space(ps_info.payees.len()),
        realloc::payer = payee,
        realloc::zero = false,
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
    pub fn space(max_payees: usize) -> usize {
        8 + // discriminator
        8 + // current_lamports
        4 + max_payees * 32 + // payees vector
        4 + max_payees * 8 + // shares_amounts vector  
        4 + max_payees * 8   // released_amounts vector
    }
}

#[event]
pub struct PaymentSplitterInitialized {
    pub initializer: Pubkey,
    pub total_shares: u64,
    pub initial_lamports: u64,
    pub payee_count: u64,
}

#[event]
pub struct PaymentReleased {
    pub payee: Pubkey,
    pub amount: u64,
    pub remaining_balance: u64,
}

#[event]
pub struct PaymentSplitterClosed {
    pub initializer: Pubkey,
}

#[error_code]
pub enum PaymentSplitterError {
    #[msg("No payees provided")]
    NoPayees,
    #[msg("Payees and shares arrays must have the same length")]
    PayeesSharesMismatch,
    #[msg("Duplicate payee addresses are not allowed")]
    DuplicatePayee,
    #[msg("Share amounts cannot be zero")]
    ZeroShares,
    #[msg("Payee not found in the payment splitter")]
    PayeeNotFound,
    #[msg("Insufficient funds for release")]
    InsufficientFunds,
    #[msg("No funds available to release for this payee")]
    NoFundsToRelease,
    #[msg("Insufficient balance in payment splitter")]
    InsufficientBalance,
}