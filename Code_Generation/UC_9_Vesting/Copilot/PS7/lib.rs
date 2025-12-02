use anchor_lang::prelude::*;
use anchor_lang::solana_program::system_instruction;

declare_id!("3uyPGz5hu8qVaWkFqikYxeUT4GEXneoiu1E7kSar8bH2");

#[program]
pub mod vesting {
    use super::*;

    pub fn initialize(
        ctx: Context<InitializeCtx>,
        start_slot: u64,
        duration: u64,
        lamports_amount: u64,
    ) -> Result<()> {
        require!(duration > 0, VestingError::ZeroDuration);

        let vest = &mut ctx.accounts.vesting_info;
        vest.released = 0;
        vest.funder = ctx.accounts.funder.key();
        vest.beneficiary = ctx.accounts.beneficiary.key();
        vest.start_slot = start_slot;
        vest.duration = duration;

        // Transfer lamports from funder to PDA (funder signs)
        let ix = system_instruction::transfer(
            &ctx.accounts.funder.key(),
            &ctx.accounts.vesting_info.to_account_info().key(),
            lamports_amount,
        );
        anchor_lang::solana_program::program::invoke(
            &ix,
            &[
                ctx.accounts.funder.to_account_info(),
                ctx.accounts.vesting_info.to_account_info(),
                ctx.accounts.system_program.to_account_info(),
            ],
        )?;

        Ok(())
    }

    pub fn release(ctx: Context<ReleaseCtx>) -> Result<()> {
        // Bind AccountInfo early so temporaries live long enough
        let vesting_ai = ctx.accounts.vesting_info.to_account_info();
        let beneficiary_ai = ctx.accounts.beneficiary.to_account_info();

        // Snapshot lamports before mutable data borrow
        let pda_lamports_before = vesting_ai.lamports();

        // Mutable borrow of account data
        let vest_info = &mut ctx.accounts.vesting_info;

        // Total original funding = released + current PDA lamports (snapshot)
        let total_original: u128 = vest_info.released as u128 + pda_lamports_before as u128;
        require!(total_original > 0, VestingError::NothingToRelease);

        let clock = Clock::get()?;
        let current_slot = clock.slot;

        // Linear vesting: vested_total = total_original * elapsed_capped / duration
        let vested_total: u128 = if current_slot <= vest_info.start_slot {
            0u128
        } else {
            let elapsed = current_slot.saturating_sub(vest_info.start_slot);
            let elapsed_capped = std::cmp::min(elapsed, vest_info.duration);
            (total_original.saturating_mul(elapsed_capped as u128))
                .checked_div(vest_info.duration as u128)
                .unwrap_or(0u128)
        };

        let released_so_far = vest_info.released as u128;
        let releasable_u128 = vested_total.saturating_sub(released_so_far);
        require!(releasable_u128 > 0, VestingError::NothingVestedYet);

        // Cap releasable to PDA lamports snapshot
        let releasable = std::cmp::min(releasable_u128, pda_lamports_before as u128) as u64;
        require!(releasable > 0, VestingError::InsufficientPdaFunds);

        // Verify PDA derived exactly from seed [beneficiary.key().as_ref()]
        let (derived_pda, _bump) =
            Pubkey::find_program_address(&[ctx.accounts.beneficiary.key.as_ref()], ctx.program_id);
        require_keys_eq!(derived_pda, vesting_ai.key(), VestingError::InvalidPda);

        // Transfer releasable lamports from PDA to beneficiary by mutating lamports (allowed for program-owned PDA)
        {
            let mut from_ref = vesting_ai.try_borrow_mut_lamports()?;
            let mut to_ref = beneficiary_ai.try_borrow_mut_lamports()?;

            **from_ref = (**from_ref)
                .checked_sub(releasable)
                .ok_or(VestingError::InsufficientPdaFunds)?;
            **to_ref = (**to_ref)
                .checked_add(releasable)
                .ok_or(VestingError::MathOverflow)?;
        }

        // Update released amount in account data
        vest_info.released = vest_info
            .released
            .checked_add(releasable)
            .ok_or(VestingError::MathOverflow)?;

        // Do NOT close or zero the account here. Closing is explicit via `close` instruction.
        Ok(())
    }

    /// Explicit close instruction callable by the funder to reclaim remaining lamports and deallocate the PDA.
    /// Funder must sign and vesting must be fully released before close.
    pub fn close(ctx: Context<CloseCtx>) -> Result<()> {
        let vest_info = &ctx.accounts.vesting_info;
        let pda_lamports = ctx.accounts.vesting_info.to_account_info().lamports();
        // total original funding equals released + whatever still sits in PDA
        let total_original: u128 = vest_info.released as u128 + pda_lamports as u128;
        // require that released >= total_original (no outstanding vested funds)
        require!(
            (vest_info.released as u128) >= total_original,
            VestingError::NotFullyReleased
        );
        // Anchor will perform the actual close (close = funder in CloseCtx)
        Ok(())
    }
}

#[derive(Accounts)]
#[instruction(start_slot: u64, duration: u64, lamports_amount: u64)]
pub struct InitializeCtx<'info> {
    /// Funder must sign and fund the PDA
    #[account(mut)]
    pub funder: Signer<'info>,

    /// CHECK: plain pubkey used as PDA seed and stored in VestingInfo
    pub beneficiary: UncheckedAccount<'info>,

    /// Vesting PDA account: seeds = [beneficiary.key().as_ref()]
    #[account(init, payer = funder, space = 8 + VestingInfo::SIZE, seeds = [beneficiary.key().as_ref()], bump)]
    pub vesting_info: Account<'info, VestingInfo>,

    pub system_program: Program<'info, System>,
}

#[derive(Accounts)]
pub struct ReleaseCtx<'info> {
    /// Beneficiary must sign and is mutable to receive lamports
    #[account(mut)]
    pub beneficiary: Signer<'info>,

    /// CHECK: funder is a reference here (used only when calling explicit close)
    pub funder: UncheckedAccount<'info>,

    /// Vesting PDA account (seeds = [beneficiary.key().as_ref()])
    #[account(mut, seeds = [beneficiary.key().as_ref()], bump)]
    pub vesting_info: Account<'info, VestingInfo>,

    pub system_program: Program<'info, System>,
}

#[derive(Accounts)]
pub struct CloseCtx<'info> {
    /// Funder must sign to receive remaining lamports and allow deallocation
    #[account(mut, signer)]
    pub funder: Signer<'info>,

    /// Vesting PDA account to close; Anchor will transfer lamports to `funder` and deallocate it
    #[account(mut, seeds = [beneficiary.key().as_ref()], bump, close = funder)]
    pub vesting_info: Account<'info, VestingInfo>,

    /// CHECK: beneficiary provided for PDA derivation consistency
    pub beneficiary: UncheckedAccount<'info>,

    pub system_program: Program<'info, System>,
}

#[account]
pub struct VestingInfo {
    pub released: u64,
    pub funder: Pubkey,
    pub beneficiary: Pubkey,
    pub start_slot: u64,
    pub duration: u64,
}

impl VestingInfo {
    // u64 + Pubkey + Pubkey + u64 + u64
    pub const SIZE: usize = 8 + 32 + 32 + 8 + 8;
}

#[error_code]
pub enum VestingError {
    #[msg("Duration must be > 0")]
    ZeroDuration,
    #[msg("Nothing vested yet")]
    NothingVestedYet,
    #[msg("No funds present in PDA")]
    NothingToRelease,
    #[msg("PDA has insufficient lamports for requested release")]
    InsufficientPdaFunds,
    #[msg("Math overflow")]
    MathOverflow,
    #[msg("Invalid PDA derived from seeds")]
    InvalidPda,
    #[msg("Vesting not yet fully released")]
    NotFullyReleased,
}
