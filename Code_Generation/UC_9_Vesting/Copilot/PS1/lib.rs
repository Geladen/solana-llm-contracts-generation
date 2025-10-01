use anchor_lang::prelude::*;
use anchor_lang::solana_program::{
    self,
    program::invoke_signed,
    system_instruction,
    rent::Rent,
    system_program,
};

declare_id!("HnMx5Gexk6o1VSrd9h9umj7bfyHXAokvS3RYDqxNkbyL");

const VAULT_SEED: &[u8] = b"vault";

#[program]
pub mod vesting_copilot {
    use super::*;

    /// Initialize vesting metadata and create a system-owned vault PDA funded with (rent + lamports_amount).
    /// signer: funder
    pub fn initialize(
        ctx: Context<InitializeCtx>,
        start_slot: u64,
        duration: u64,
        lamports_amount: u64,
    ) -> Result<()> {
        require!(duration > 0, VestingError::InvalidDuration);
        require!(lamports_amount > 0, VestingError::InvalidAmount);

        // populate VestingInfo
        let vesting = &mut ctx.accounts.vesting_info;
        vesting.released = 0;
        vesting.funder = ctx.accounts.funder.key();
        vesting.beneficiary = ctx.accounts.beneficiary.key();
        vesting.start_slot = start_slot;
        vesting.duration = duration;

        // derive vault PDA and ensure passed account matches
        let beneficiary_key = ctx.accounts.beneficiary.key();
        let (vault_pda, _vault_bump) =
            Pubkey::find_program_address(&[beneficiary_key.as_ref(), VAULT_SEED], ctx.program_id);
        require_keys_eq!(vault_pda, ctx.accounts.vault.key(), VestingError::InvalidVault);

        // create the vault system account with total lamports = rent + deposit
        let rent = Rent::get()?;
        let vault_space: usize = 0; // vault holds no data
        let rent_lamports = rent.minimum_balance(vault_space);
        let total_lamports = rent_lamports
            .checked_add(lamports_amount)
            .ok_or(VestingError::MathOverflow)?;

        let create_ix = system_instruction::create_account(
            &ctx.accounts.funder.key(),
            &vault_pda,
            total_lamports,
            vault_space as u64,
            &system_program::ID,
        );

        anchor_lang::solana_program::program::invoke(
            &create_ix,
            &[
                ctx.accounts.funder.to_account_info(),
                ctx.accounts.vault.to_account_info(),
                ctx.accounts.system_program.to_account_info(),
            ],
        )?;

        Ok(())
    }

    /// Release vested tokens to beneficiary. signer: beneficiary
    pub fn release(ctx: Context<ReleaseCtx>) -> Result<()> {
        // validate beneficiary matches vesting info
        let beneficiary_key = ctx.accounts.beneficiary.key();
        require!(
            ctx.accounts.vesting_info.beneficiary == beneficiary_key,
            VestingError::InvalidBeneficiary
        );

        let clock = Clock::get()?;
        let current_slot = clock.slot;

        // Bind accountinfos locally
        let vault_ai = ctx.accounts.vault.to_account_info();
        let vesting_info_ai = ctx.accounts.vesting_info.to_account_info();

        // total deposit is simply vault lamports + released (released is already sent out)
        let vault_lamports = **vault_ai.lamports.borrow();
        let released = ctx.accounts.vesting_info.released;
        let total_deposit = vault_lamports
            .checked_add(released)
            .ok_or(VestingError::MathOverflow)?;

        // compute vested amount linearly by slots
        let vested_amount = if current_slot < ctx.accounts.vesting_info.start_slot {
            0u128
        } else {
            let elapsed = current_slot
                .saturating_sub(ctx.accounts.vesting_info.start_slot)
                .min(ctx.accounts.vesting_info.duration);
            let vested = (total_deposit as u128)
                .checked_mul(elapsed as u128)
                .and_then(|v| v.checked_div(ctx.accounts.vesting_info.duration as u128))
                .ok_or(VestingError::MathOverflow)?;
            vested
        } as u64;

        // releasable = vested - released
        let releasable = vested_amount.checked_sub(released).unwrap_or(0);
        require!(releasable > 0, VestingError::NothingToRelease);

        // derive vault PDA and bump, ensure match
        let (vault_pda, vault_bump) = Pubkey::find_program_address(
            &[beneficiary_key.as_ref(), VAULT_SEED],
            ctx.program_id,
        );
        require_keys_eq!(vault_pda, ctx.accounts.vault.key(), VestingError::InvalidVault);

        // prepare signer seeds
        let beneficiary_ref: &[u8] = beneficiary_key.as_ref();
        let vault_seed_bytes: &[u8] = VAULT_SEED;
        let bump_arr: [u8; 1] = [vault_bump];
        let seeds: &[&[u8]] = &[beneficiary_ref, vault_seed_bytes, &bump_arr];
        let signer_seeds = &[seeds];

        // transfer from vault -> beneficiary
        let ix = system_instruction::transfer(&vault_pda, &ctx.accounts.beneficiary.key(), releasable);
        invoke_signed(
            &ix,
            &[
                vault_ai.clone(),
                ctx.accounts.beneficiary.to_account_info().clone(),
                ctx.accounts.system_program.to_account_info().clone(),
            ],
            signer_seeds,
        )?;

        // update released
        ctx.accounts.vesting_info.released = ctx.accounts.vesting_info.released
            .checked_add(releasable)
            .ok_or(VestingError::MathOverflow)?;

        // if vault now empty, zero vesting_info data to render inert (and effectively closed)
        let remaining_vault_lamports = **vault_ai.lamports.borrow();
        if remaining_vault_lamports == 0 {
            let mut data = vesting_info_ai.data.borrow_mut();
            for byte in data.iter_mut() {
                *byte = 0;
            }
        }

        Ok(())
    }
}

/// Accounts: keep exactly the declared accounts per instruction.

#[derive(Accounts)]
pub struct InitializeCtx<'info> {
    /// Funder must sign
    #[account(mut)]
    pub funder: Signer<'info>,

    /// CHECK: beneficiary is only used as PDA seed; stored in VestingInfo and validated on release
    pub beneficiary: UncheckedAccount<'info>,

    /// Vesting metadata PDA (program-owned) seeds = [beneficiary]
    #[account(
        init,
        payer = funder,
        space = VestingInfo::LEN,
        seeds = [beneficiary.key().as_ref()],
        bump
    )]
    pub vesting_info: Account<'info, VestingInfo>,

    /// CHECK: vault is a system-owned PDA (no data). Provided by caller and validated in code.
    /// We create the vault in initialize using create_account. Vault holds lamports only.
    #[account(mut)]
    pub vault: UncheckedAccount<'info>,

    pub system_program: Program<'info, System>,
}

#[derive(Accounts)]
pub struct ReleaseCtx<'info> {
    /// Beneficiary must sign
    #[account(mut, signer)]
    pub beneficiary: Signer<'info>,

    /// CHECK: funder is only used as the recipient of returned rent lamports on final close.
    /// No signature required; we only need its pubkey at runtime.
    #[account(mut)]
    pub funder: UncheckedAccount<'info>,

    /// Vesting metadata PDA (program-owned)
    #[account(mut, seeds=[beneficiary.key().as_ref()], bump)]
    pub vesting_info: Account<'info, VestingInfo>,

    /// CHECK: vault is a system-owned PDA (no data) that holds lamports.
    /// We validate it at runtime by deriving the expected PDA and comparing pubkeys.
    #[account(mut)]
    pub vault: UncheckedAccount<'info>,

    pub system_program: Program<'info, System>,
}

/// VestingInfo structure exactly as requested
#[account]
pub struct VestingInfo {
    pub released: u64,
    pub funder: Pubkey,
    pub beneficiary: Pubkey,
    pub start_slot: u64,
    pub duration: u64,
}

impl VestingInfo {
    // discriminator + fields
    pub const LEN: usize = 8 + 8 + 32 + 32 + 8 + 8;
}

/// Errors
#[error_code]
pub enum VestingError {
    #[msg("Duration must be > 0")]
    InvalidDuration,
    #[msg("Lamports amount must be > 0")]
    InvalidAmount,
    #[msg("Nothing is available for release")]
    NothingToRelease,
    #[msg("Math overflow occurred")]
    MathOverflow,
    #[msg("Vesting info beneficiary mismatch")]
    InvalidBeneficiary,
    #[msg("Vault PDA mismatch")]
    InvalidVault,
}
