use anchor_lang::prelude::*;

declare_id!("2gdoPE6XceVVVo3EeB1DmmgLfnSoNmQqHEJYmXJADKT6");

#[program]
pub mod vesting {
    use super::*;

    /// Initialize a vesting schedule (called by funder).
    /// Creates the vesting PDA (seeds = [beneficiary.key().as_ref()]) and deposits lamports_amount
    /// (in addition to the rent-exempt balance created by the init).
    pub fn initialize(
        ctx: Context<InitializeCtx>,
        start_slot: u64,
        duration: u64,
        lamports_amount: u64,
    ) -> Result<()> {
        // Basic checks
        if duration == 0 {
            return err!(VestingError::DurationMustBeNonZero);
        }
        if lamports_amount == 0 {
            return err!(VestingError::MustDepositNonZero);
        }

        // Fill vesting info
        let vesting = &mut ctx.accounts.vesting_info;
        vesting.released = 0;
        vesting.funder = ctx.accounts.funder.key();
        vesting.beneficiary = ctx.accounts.beneficiary.key();
        vesting.start_slot = start_slot;
        vesting.duration = duration;

        // Transfer lamports_amount from funder to the PDA account (vesting_info),
        // in addition to the rent-exempt lamports provided by the `init` CPI.
        // Use the system program CPI to transfer lamports.
        let cpi_program = ctx.accounts.system_program.to_account_info();
        let cpi_accounts = anchor_lang::system_program::Transfer {
            from: ctx.accounts.funder.to_account_info(),
            to: ctx.accounts.vesting_info.to_account_info(),
        };
        let cpi_ctx = CpiContext::new(cpi_program, cpi_accounts);
        anchor_lang::system_program::transfer(cpi_ctx, lamports_amount)?;

        Ok(())
    }

    /// Release vested lamports to the beneficiary (called by beneficiary).
    /// - computes vested amount based on current slot
    /// - transfers releasable lamports to beneficiary
    /// - updates `released`
    /// - closes/deallocates the vesting account when fully released (returns rent -> funder)
    pub fn release(ctx: Context<ReleaseCtx>) -> Result<()> {
        // First: get AccountInfo before mut borrow
        let vesting_info_ai = ctx.accounts.vesting_info.to_account_info();

        // Rent-exempt math
        let rent = Rent::get()?;
        let vesting_lamports: u64 = **vesting_info_ai.lamports.borrow();
        let min_rent = rent.minimum_balance(VestingInfo::LEN);
        let locked_excluding_rent = vesting_lamports.saturating_sub(min_rent);

        // We also need released, start_slot, duration etc — snapshot them before.
        let released_so_far = ctx.accounts.vesting_info.released;
        let start_slot = ctx.accounts.vesting_info.start_slot;
        let duration = ctx.accounts.vesting_info.duration;

        let original_total = locked_excluding_rent
            .checked_add(released_so_far)
            .ok_or(error!(VestingError::ArithmeticOverflow))?;

        if original_total == 0 {
            return Err(error!(VestingError::NothingToRelease));
        }

        // Calculate vested
        let clock = Clock::get()?;
        let now_slot = clock.slot;
        let vested_amount: u128 = if now_slot < start_slot {
            0
        } else {
            let elapsed = now_slot - start_slot;
            if elapsed >= duration {
                original_total as u128
            } else {
                (original_total as u128)
                    .checked_mul(elapsed as u128)
                    .ok_or(error!(VestingError::ArithmeticOverflow))?
                    .checked_div(duration as u128)
                    .ok_or(error!(VestingError::ArithmeticOverflow))?
            }
        };

        let already_released = released_so_far as u128;
        if vested_amount <= already_released {
            return Err(error!(VestingError::NoReleasableAmount));
        }
        let releasable: u64 = (vested_amount - already_released)
            .try_into()
            .map_err(|_| error!(VestingError::ArithmeticOverflow))?;

        if releasable > locked_excluding_rent {
            return Err(error!(VestingError::InsufficientLockedLamports));
        }

        // Transfer lamports (double-deref trick)
        let beneficiary_ai = ctx.accounts.beneficiary.to_account_info();
        {
            let mut from_lamports = vesting_info_ai.lamports.borrow_mut();
            let mut to_lamports = beneficiary_ai.lamports.borrow_mut();

            let new_from = (**from_lamports)
                .checked_sub(releasable)
                .ok_or(error!(VestingError::ArithmeticOverflow))?;
            let new_to = (**to_lamports)
                .checked_add(releasable)
                .ok_or(error!(VestingError::ArithmeticOverflow))?;

            **from_lamports = new_from;
            **to_lamports = new_to;
        }

        // NOW it’s safe to take mutable borrow of vesting_info again
        let vesting = &mut ctx.accounts.vesting_info;
        vesting.released = vesting
            .released
            .checked_add(releasable)
            .ok_or(error!(VestingError::ArithmeticOverflow))?;

        if vesting.released == original_total {
            vesting.close(ctx.accounts.funder.to_account_info())?;
        }

        Ok(())
    }
}

// ----------------------------- Accounts contexts -----------------------------

#[derive(Accounts)]
pub struct InitializeCtx<'info> {
    /// Payer / Funder - must sign
    #[account(mut)]
    pub funder: Signer<'info>,

    /// CHECK: This is just a reference used to derive the PDA seed and stored in VestingInfo.
    /// No data is read or written, so no further checks are required.
    pub beneficiary: UncheckedAccount<'info>,

    /// Vesting PDA (created here). PDA seeds MUST be exactly: [beneficiary.key().as_ref()]
    /// space respects the VestingInfo size.
    #[account(
        init,
        payer = funder,
        space = VestingInfo::LEN,
        seeds = [beneficiary.key().as_ref()],
        bump
    )]
    pub vesting_info: Account<'info, VestingInfo>,

    pub system_program: Program<'info, System>,
}

#[derive(Accounts)]
pub struct ReleaseCtx<'info> {
    /// Beneficiary must sign to release vested funds
    #[account(mut)]
    pub beneficiary: Signer<'info>,

    /// CHECK: This is only used for address comparison (`has_one = funder`)
    /// and as the recipient of rent when closing the PDA.
    /// No data is accessed, so no further checks are required.
    #[account(mut)]
    pub funder: UncheckedAccount<'info>,

    /// Vesting PDA - seeds must match exactly [beneficiary.key().as_ref()]
    /// Validated by has_one constraints.
    #[account(
        mut,
        seeds = [beneficiary.key().as_ref()],
        bump,
        has_one = beneficiary,
        has_one = funder
    )]
    pub vesting_info: Account<'info, VestingInfo>,
}


// ----------------------------- State -----------------------------

/// Vesting information stored in the PDA.
/// Fields:
/// - released: amount (u64) already released to beneficiary
/// - funder: pubkey of funder
/// - beneficiary: pubkey of beneficiary
/// - start_slot: slot at which vesting starts
/// - duration: vesting duration in slots
#[account]
pub struct VestingInfo {
    pub released: u64,
    pub funder: Pubkey,
    pub beneficiary: Pubkey,
    pub start_slot: u64,
    pub duration: u64,
}

impl VestingInfo {
    // discriminator 8 + fields
    // released: 8
    // funder: 32
    // beneficiary: 32
    // start_slot: 8
    // duration: 8
    // total = 8 + 8 + 32 + 32 + 8 + 8 = 96
    pub const LEN: usize = 8 + 8 + 32 + 32 + 8 + 8;
}

// ----------------------------- Errors -----------------------------

#[error_code]
pub enum VestingError {
    #[msg("Vesting duration must be non-zero.")]
    DurationMustBeNonZero,

    #[msg("Must deposit a non-zero amount to vest.")]
    MustDepositNonZero,

    #[msg("Arithmetic overflow.")]
    ArithmeticOverflow,

    #[msg("No releasable amount at this time.")]
    NoReleasableAmount,

    #[msg("Insufficient locked lamports in vesting account.")]
    InsufficientLockedLamports,

    #[msg("Nothing to release.")]
    NothingToRelease,
}
