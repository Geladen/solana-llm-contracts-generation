use anchor_lang::prelude::*;
use anchor_lang::solana_program::{
    program::invoke_signed,
    system_instruction,
    sysvar::rent::Rent,
};
declare_id!("B3C6UdHU96uL37r4CYAyEon8XMaDgjTNuW7VP9J8Ax24");




#[program]
pub mod payment_splitter {
    use super::*;

    pub fn initialize(
        ctx: Context<InitializeCtx>,
        lamports_to_transfer: u64,
        shares_amounts: Vec<u64>,
    ) -> Result<()> {
        // Validate payees via remaining accounts
        let payees_accs = &ctx.remaining_accounts;
        let num_payees = payees_accs.len();
        require!(num_payees > 0, PaymentSplitterError::NoPayeesProvided);
        require!(
            shares_amounts.len() == num_payees,
            PaymentSplitterError::SharesPayeesLengthMismatch
        );

        let mut payee_keys: Vec<Pubkey> = Vec::with_capacity(num_payees);
        for acc in payees_accs.iter() {
            let pk: Pubkey = *acc.key;
            require!(!payee_keys.contains(&pk), PaymentSplitterError::DuplicatePayee);
            payee_keys.push(pk);
        }

        // Derive PDAs and bumps
        let (_ps_pda, ps_bump) = Pubkey::find_program_address(
            &[b"payment_splitter".as_ref(), ctx.accounts.initializer.key.as_ref()],
            ctx.program_id,
        );
        let (vault_pda, vault_bump) = Pubkey::find_program_address(
            &[
                b"payment_splitter".as_ref(),
                ctx.accounts.initializer.key.as_ref(),
                b"vault".as_ref(),
            ],
            ctx.program_id,
        );

        // Create vault as a system-owned account (owner = system_program::ID) with no data.
        let rent = Rent::get()?;
        let lamports_needed = rent.minimum_balance(0);

        let create_ix = system_instruction::create_account(
            ctx.accounts.initializer.key,
            &vault_pda,
            lamports_needed,
            0, // space = 0 (data-less)
            &anchor_lang::solana_program::system_program::ID,
        );

        let vault_seeds: &[&[u8]] = &[
            b"payment_splitter".as_ref(),
            ctx.accounts.initializer.key.as_ref(),
            b"vault".as_ref(),
            &[vault_bump],
        ];
        let signer_seeds_for_create: &[&[&[u8]]] = &[vault_seeds];

        invoke_signed(
            &create_ix,
            &[
                ctx.accounts.initializer.to_account_info(),
                ctx.accounts.vault.to_account_info(),
                ctx.accounts.system_program.to_account_info(),
            ],
            signer_seeds_for_create,
        )
        .map_err(|_| error!(PaymentSplitterError::VaultCreateFailed))?;

        // Optionally fund the vault with the requested lamports_to_transfer
        if lamports_to_transfer > 0 {
            let transfer_ix = system_instruction::transfer(ctx.accounts.initializer.key, &vault_pda, lamports_to_transfer);
            invoke_signed(
                &transfer_ix,
                &[
                    ctx.accounts.initializer.to_account_info(),
                    ctx.accounts.vault.to_account_info(),
                    ctx.accounts.system_program.to_account_info(),
                ],
                signer_seeds_for_create,
            )
            .map_err(|_| error!(PaymentSplitterError::VaultFundFailed))?;
        }

        // Initialize program-owned PaymentSplitterInfo state
        let ps = &mut ctx.accounts.ps_info;
        ps.bump = ps_bump;
        ps.vault_bump = vault_bump;
        ps.payees = payee_keys;
        ps.shares_amounts = shares_amounts;
        ps.released_amounts = vec![0u64; ps.shares_amounts.len()];
        ps.current_lamports = ctx.accounts.vault.to_account_info().lamports();

        Ok(())
    }

    pub fn release(ctx: Context<ReleaseCtx>) -> Result<()> {
        let payee_key = ctx.accounts.payee.key();

        // Clone for computation to avoid borrow conflicts
        let payees = ctx.accounts.ps_info.payees.clone();
        let shares = ctx.accounts.ps_info.shares_amounts.clone();
        let released = ctx.accounts.ps_info.released_amounts.clone();
        let vault_lamports = ctx.accounts.vault.to_account_info().lamports();

        let index = payees
            .iter()
            .position(|k| k == &payee_key)
            .ok_or(PaymentSplitterError::PayeeNotFound)?;

        let total_released_sum: u128 = released.iter().map(|&x| x as u128).sum();
        let total_received: u128 = (vault_lamports as u128)
            .checked_add(total_released_sum)
            .ok_or(PaymentSplitterError::MathOverflow)?;
        let total_shares: u128 = shares.iter().map(|&s| s as u128).sum();
        require!(total_shares > 0, PaymentSplitterError::ZeroTotalShares);

        let payee_shares = shares[index] as u128;
        let total_due = total_received
            .checked_mul(payee_shares)
            .and_then(|v| v.checked_div(total_shares))
            .ok_or(PaymentSplitterError::MathOverflow)?;
        let already_released = released[index] as u128;
        if total_due <= already_released {
            return Err(PaymentSplitterError::NothingToRelease.into());
        }
        let releasable_u128 = total_due
            .checked_sub(already_released)
            .ok_or(PaymentSplitterError::MathOverflow)?;
        require!(
            releasable_u128 <= u128::from(vault_lamports),
            PaymentSplitterError::InsufficientPdaBalance
        );
        let releasable = releasable_u128 as u64;

        // Transfer from vault (system-owned PDA) to payee. Sign with vault PDA seeds.
        let vault_bump = ctx.accounts.ps_info.vault_bump;
        let vault_seeds: &[&[u8]] = &[
            b"payment_splitter".as_ref(),
            ctx.accounts.initializer.key.as_ref(),
            b"vault".as_ref(),
            &[vault_bump],
        ];
        let signer_seeds: &[&[&[u8]]] = &[vault_seeds];

        let (vault_pubkey, _b) = Pubkey::find_program_address(
            &[
                b"payment_splitter".as_ref(),
                ctx.accounts.initializer.key.as_ref(),
                b"vault".as_ref(),
            ],
            ctx.program_id,
        );

        let transfer_ix = system_instruction::transfer(&vault_pubkey, ctx.accounts.payee.key, releasable);
        invoke_signed(
            &transfer_ix,
            &[
                ctx.accounts.vault.to_account_info(),
                ctx.accounts.payee.to_account_info(),
                ctx.accounts.system_program.to_account_info(),
            ],
            signer_seeds,
        )
        .map_err(|_| error!(PaymentSplitterError::VaultTransferFailed))?;

        // Update bookkeeping
        let ps = &mut ctx.accounts.ps_info;
        ps.released_amounts[index] = ps.released_amounts[index]
            .checked_add(releasable)
            .ok_or(PaymentSplitterError::MathOverflow)?;
        ps.current_lamports = ctx.accounts.vault.to_account_info().lamports();

        Ok(())
    }
}

#[derive(Accounts)]
#[instruction(lamports_to_transfer: u64, shares_amounts: Vec<u64>)]
pub struct InitializeCtx<'info> {
    #[account(mut)]
    pub initializer: Signer<'info>,

    // PaymentSplitterInfo is program-owned data account
    #[account(
        init,
        payer = initializer,
        space = PaymentSplitterInfo::space_for(&shares_amounts),
        seeds = [b"payment_splitter".as_ref(), initializer.key().as_ref()],
        bump
    )]
    pub ps_info: Account<'info, PaymentSplitterInfo>,

    /// CHECK: vault is created in initialize as a system-owned account via create_account (owner=system_program)
    #[account(mut)]
    pub vault: AccountInfo<'info>,

    pub system_program: Program<'info, System>,
}

#[derive(Accounts)]
pub struct ReleaseCtx<'info> {
    #[account(mut)]
    pub payee: Signer<'info>,

    /// CHECK: initializer is only used for PDA derivation
    #[account(mut)]
    pub initializer: UncheckedAccount<'info>,

    #[account(
        mut,
        seeds = [b"payment_splitter".as_ref(), initializer.key.as_ref()],
        bump = ps_info.bump,
    )]
    pub ps_info: Account<'info, PaymentSplitterInfo>,

    /// CHECK: vault must be the system-owned PDA created during initialize
    #[account(mut)]
    pub vault: AccountInfo<'info>,

    pub system_program: Program<'info, System>,
}

#[account]
pub struct PaymentSplitterInfo {
    pub bump: u8,
    pub vault_bump: u8,
    pub current_lamports: u64,
    pub payees: Vec<Pubkey>,
    pub shares_amounts: Vec<u64>,
    pub released_amounts: Vec<u64>,
}

impl PaymentSplitterInfo {
    pub fn space_for(shares_amounts: &Vec<u64>) -> usize {
        let mut size = 8; // discriminator
        size += 1 + 1; // bump + vault_bump
        size += 8; // current_lamports
        let num = shares_amounts.len();
        size += 4 + num * 32; // payees vec
        size += 4 + num * 8; // shares_amounts
        size += 4 + num * 8; // released_amounts
        size += 8; // padding
        size
    }
}

#[error_code]
pub enum PaymentSplitterError {
    #[msg("No payees provided")]
    NoPayeesProvided,
    #[msg("Shares and payees length mismatch")]
    SharesPayeesLengthMismatch,
    #[msg("Duplicate payee")]
    DuplicatePayee,
    #[msg("Payee not found")]
    PayeeNotFound,
    #[msg("Nothing to release")]
    NothingToRelease,
    #[msg("Insufficient PDA balance")]
    InsufficientPdaBalance,
    #[msg("Total shares is zero")]
    ZeroTotalShares,
    #[msg("Math overflow")]
    MathOverflow,
    #[msg("Failed to create vault")]
    VaultCreateFailed,
    #[msg("Failed to fund vault")]
    VaultFundFailed,
    #[msg("Failed to transfer from vault")]
    VaultTransferFailed,
}
