use anchor_lang::prelude::*;

declare_id!("Dxp12CD5ndCP3MuLCmGt4N9GaKVSgzBpHKKmSg6R8hpR");

#[program]
pub mod vesting {
    use super::*;

    pub fn initialize(
        ctx: Context<InitializeCtx>,
        start_slot: u64,
        duration: u64,
        lamports_amount: u64,
    ) -> Result<()> {
        let clock = Clock::get()?;
        require!(start_slot >= clock.slot, VestingError::StartSlotInPast);
        require!(duration > 0, VestingError::ZeroDuration);

        let vest = &mut ctx.accounts.vesting_info;
        vest.released = 0;
        vest.funder = ctx.accounts.funder.key();
        vest.beneficiary = ctx.accounts.beneficiary.key();
        vest.start_slot = start_slot;
        vest.duration = duration;

        // Transfer lamports_amount from funder to the PDA (destination may be program-owned)
        let ix = anchor_lang::solana_program::system_instruction::transfer(
            &ctx.accounts.funder.key(),
            &ctx.accounts.vesting_info.key(),
            lamports_amount,
        );
        anchor_lang::solana_program::program::invoke(
            &ix,
            &[
                ctx.accounts.funder.to_account_info(),
                ctx.accounts.vesting_info.to_account_info(),
            ],
        )?;

        Ok(())
    }

    pub fn release(ctx: Context<ReleaseCtx>) -> Result<()> {
        // Clone AccountInfo values first to avoid overlapping borrows
        let vesting_ai = ctx.accounts.vesting_info.to_account_info();
        let beneficiary_ai = ctx.accounts.beneficiary.to_account_info();
        let funder_ai = ctx.accounts.funder.to_account_info();

        // Read-only state
        let clock = Clock::get()?;
        let rent = Rent::get()?;
        let rent_min = rent.minimum_balance(VestingInfo::LEN);

        // Read current lamports before taking mutable typed borrow
        let current_balance = vesting_ai.lamports();

        // Mutable typed borrow of VestingInfo
        let vest_acc = &mut ctx.accounts.vesting_info;

        // Compute total deposited (excluding rent)
        let released_before = vest_acc.released;
        let total_deposit = current_balance
            .checked_add(released_before)
            .and_then(|v| v.checked_sub(rent_min))
            .ok_or(VestingError::ArithmeticOverflow)?;

        // Compute vested so far (128-bit intermediate)
        let vested_so_far: u128 = if clock.slot < vest_acc.start_slot {
            0
        } else if vest_acc.duration == 0
            || clock.slot >= vest_acc.start_slot.saturating_add(vest_acc.duration)
        {
            total_deposit as u128
        } else {
            let passed = clock.slot.saturating_sub(vest_acc.start_slot) as u128;
            let dur = vest_acc.duration as u128;
            (total_deposit as u128)
                .checked_mul(passed)
                .and_then(|v| v.checked_div(dur))
                .ok_or(VestingError::ArithmeticOverflow)?
        };

        let vested_so_far_u64: u64 =
            vested_so_far.try_into().map_err(|_| VestingError::ArithmeticOverflow)?;
        let releasable = vested_so_far_u64.checked_sub(released_before).unwrap_or(0);

        if releasable == 0 {
            return Ok(());
        }

        // Compute PDA available to withdraw without touching rent
        let pda_balance = vesting_ai.lamports();
        let available_to_withdraw = if pda_balance > rent_min {
            pda_balance - rent_min
        } else {
            0u64
        };

        // Determine how much to transfer to beneficiary now (never touch rent)
        let transfer_amount = if releasable > available_to_withdraw {
            available_to_withdraw
        } else {
            releasable
        };

        if transfer_amount == 0 {
            return Ok(());
        }

        // Perform the transfer by mutating lamports (program owns the PDA)
        {
            let mut from_lamports = vesting_ai.try_borrow_mut_lamports()?;
            let mut to_lamports = beneficiary_ai.try_borrow_mut_lamports()?;

            let from_val: u64 = **from_lamports;
            let to_val: u64 = **to_lamports;

            if from_val < transfer_amount {
                return Err(error!(VestingError::InsufficientFunds));
            }

            let new_from = from_val
                .checked_sub(transfer_amount)
                .ok_or(VestingError::ArithmeticOverflow)?;
            let new_to = to_val
                .checked_add(transfer_amount)
                .ok_or(VestingError::ArithmeticOverflow)?;

            **from_lamports = new_from;
            **to_lamports = new_to;
        }

        // Update released by the actual transferred amount
        vest_acc.released = vest_acc
            .released
            .checked_add(transfer_amount)
            .ok_or(VestingError::ArithmeticOverflow)?;

        // Recompute remaining_deposit = total_deposit - released
        let remaining_deposit = total_deposit
            .checked_sub(vest_acc.released)
            .ok_or(VestingError::ArithmeticOverflow)?;

        // If fully vested (i.e., nothing remains), finalize: move any remaining lamports (including rent) to funder and zero data
        if remaining_deposit == 0 {
            {
                // move all remaining lamports in PDA to funder
                let mut from_lamports = vesting_ai.try_borrow_mut_lamports()?;
                let mut to_lamports = funder_ai.try_borrow_mut_lamports()?;

                let amount: u64 = **from_lamports;
                if amount > 0 {
                    **from_lamports = 0u64;
                    **to_lamports = (**to_lamports)
                        .checked_add(amount)
                        .ok_or(VestingError::ArithmeticOverflow)?;
                }
            }

            // Zero PDA data
            {
                let mut data = vesting_ai.data.borrow_mut();
                for byte in data.iter_mut() {
                    *byte = 0;
                }
            }

            // Ensure released equals total_deposit
            vest_acc.released = total_deposit;
        }

        Ok(())
    }

    /// Optional explicit close (keeps compatibility): ensures fully released and closes via Anchor
    pub fn close_vesting(ctx: Context<CloseCtx>) -> Result<()> {
        let vest = &ctx.accounts.vesting_info;
        let vesting_ai = ctx.accounts.vesting_info.to_account_info();
        let rent = Rent::get()?;
        let rent_min = rent.minimum_balance(VestingInfo::LEN);
        let current_balance = vesting_ai.lamports();

        let total_deposit = current_balance
            .checked_add(vest.released)
            .and_then(|v| v.checked_sub(rent_min))
            .ok_or(VestingError::ArithmeticOverflow)?;

        require!(vest.released == total_deposit, VestingError::NotFullyReleased);
        Ok(())
    }
}

#[derive(Accounts)]
#[instruction(start_slot: u64, duration: u64, lamports_amount: u64)]
pub struct InitializeCtx<'info> {
    /// Funder must sign and pay for initialization
    #[account(mut)]
    pub funder: Signer<'info>,

    /// CHECK: beneficiary is only used as a Pubkey seed for the vesting PDA and stored in VestingInfo;
    /// no data from this account is read or trusted during instruction execution, so a runtime check is unnecessary.
    pub beneficiary: UncheckedAccount<'info>,

    /// Vesting PDA with exact seed [beneficiary.key().as_ref()]
    #[account(
        init,
        seeds = [beneficiary.key().as_ref()],
        bump,
        payer = funder,
        space = VestingInfo::LEN
    )]
    pub vesting_info: Account<'info, VestingInfo>,

    pub system_program: Program<'info, System>,
}

#[derive(Accounts)]
pub struct ReleaseCtx<'info> {
    /// Beneficiary must sign to release vested funds
    #[account(mut)]
    pub beneficiary: Signer<'info>,

    /// CHECK: funder is only stored in VestingInfo and may receive funds on finalization;
    /// we don't read or trust any data from this account during release, so a runtime check is unnecessary.
    #[account(mut)]
    pub funder: UncheckedAccount<'info>,

    /// Vesting PDA; seeds exactly [beneficiary.key().as_ref()]
    #[account(
        mut,
        seeds = [beneficiary.key().as_ref()],
        bump,
        has_one = beneficiary,
        has_one = funder
    )]
    pub vesting_info: Account<'info, VestingInfo>,

    pub system_program: Program<'info, System>,
}

/// Close context uses Anchor's `close = funder` so rent is returned automatically when instruction returns.
#[derive(Accounts)]
pub struct CloseCtx<'info> {
    /// Beneficiary must sign to close (keeps same security model)
    #[account(mut)]
    pub beneficiary: Signer<'info>,

    /// CHECK: funder is the recipient of returned rent when the vesting_info account is closed.
    #[account(mut)]
    pub funder: UncheckedAccount<'info>,

    /// Vesting PDA with exact seed [beneficiary.key().as_ref()]
    #[account(
        mut,
        seeds = [beneficiary.key().as_ref()],
        bump,
        has_one = beneficiary,
        has_one = funder,
        close = funder
    )]
    pub vesting_info: Account<'info, VestingInfo>,
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
    pub const LEN: usize = 8 + 8 + 32 + 32 + 8 + 8;
}

#[error_code]
pub enum VestingError {
    #[msg("Start slot must be in the future or current slot.")]
    StartSlotInPast,
    #[msg("Zero duration not allowed.")]
    ZeroDuration,
    #[msg("Arithmetic overflow or underflow occurred.")]
    ArithmeticOverflow,
    #[msg("Insufficient funds in vesting account.")]
    InsufficientFunds,
    #[msg("Vesting not fully released")]
    NotFullyReleased,
}
