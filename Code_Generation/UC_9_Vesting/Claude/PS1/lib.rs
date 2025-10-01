use anchor_lang::prelude::*;
use anchor_lang::system_program;

declare_id!("FFnpU5vbpnaj6JmUR5VtkzAgS7wyTnzojxz3SV2Ckqm8");

#[program]
pub mod vesting_program {
    use super::*;

    /// Initialize a new vesting schedule
    /// Only the funder can call this instruction
    pub fn initialize(
        ctx: Context<InitializeCtx>,
        start_slot: u64,
        duration: u64,
        lamports_amount: u64,
    ) -> Result<()> {
        let vesting_info = &mut ctx.accounts.vesting_info;
        let funder = &ctx.accounts.funder;
        let beneficiary = &ctx.accounts.beneficiary;

        // Validate parameters
        require!(duration > 0, VestingError::InvalidDuration);
        require!(lamports_amount > 0, VestingError::InvalidAmount);

        // Initialize vesting info
        vesting_info.released = 0;
        vesting_info.funder = funder.key();
        vesting_info.beneficiary = beneficiary.key();
        vesting_info.start_slot = start_slot;
        vesting_info.duration = duration;

        // Transfer lamports to vesting PDA
        let transfer_instruction = system_program::Transfer {
            from: funder.to_account_info(),
            to: vesting_info.to_account_info(),
        };

        let cpi_ctx = CpiContext::new(
            ctx.accounts.system_program.to_account_info(),
            transfer_instruction,
        );

        system_program::transfer(cpi_ctx, lamports_amount)?;

        emit!(VestingInitialized {
            funder: funder.key(),
            beneficiary: beneficiary.key(),
            start_slot,
            duration,
            amount: lamports_amount,
        });

        Ok(())
    }

    /// Release vested funds to beneficiary
    /// Only the beneficiary can call this instruction
    pub fn release(ctx: Context<ReleaseCtx>) -> Result<()> {
        let vesting_info = &mut ctx.accounts.vesting_info;
        let beneficiary = &ctx.accounts.beneficiary;
        let current_slot = Clock::get()?.slot;

        // Get account rent requirement
        let rent_exempt_amount = Rent::get()?.minimum_balance(VestingInfo::LEN + 8);
        let current_balance = vesting_info.to_account_info().lamports();
        
        // Calculate the original vesting amount (current balance + already released - rent)
        let total_vesting_amount = current_balance + vesting_info.released - rent_exempt_amount;
        
        let total_vested = calculate_vested_amount(
            vesting_info.start_slot,
            vesting_info.duration,
            current_slot,
            total_vesting_amount,
        )?;

        // Calculate releasable amount (total vested - already released)
        let releasable_amount = total_vested
            .checked_sub(vesting_info.released)
            .ok_or(VestingError::CalculationOverflow)?;

        require!(releasable_amount > 0, VestingError::NoFundsToRelease);

        // Check if vesting period is complete
        let end_slot = vesting_info.start_slot.checked_add(vesting_info.duration).ok_or(VestingError::CalculationOverflow)?;
        let is_fully_vested = current_slot >= end_slot;
        
        // Calculate transfer amount - don't transfer rent unless fully vested
        let available_for_vesting = current_balance.saturating_sub(rent_exempt_amount);
        let transfer_amount = std::cmp::min(releasable_amount, available_for_vesting);

        require!(transfer_amount > 0, VestingError::InsufficientFunds);

        // Update released amount
        vesting_info.released = vesting_info
            .released
            .checked_add(transfer_amount)
            .ok_or(VestingError::CalculationOverflow)?;

        // Transfer vested amount to beneficiary
        **vesting_info.to_account_info().try_borrow_mut_lamports()? -= transfer_amount;
        **beneficiary.to_account_info().try_borrow_mut_lamports()? += transfer_amount;

        emit!(FundsReleased {
            beneficiary: beneficiary.key(),
            amount: transfer_amount,
            total_released: vesting_info.released,
        });

        // If fully vested, close account and return rent to funder
        if is_fully_vested {
            let remaining_balance = vesting_info.to_account_info().lamports();
            if remaining_balance > 0 {
                **vesting_info.to_account_info().try_borrow_mut_lamports()? = 0;
                **ctx.accounts.funder.to_account_info().try_borrow_mut_lamports()? += remaining_balance;

                emit!(VestingCompleted {
                    beneficiary: beneficiary.key(),
                    final_amount: remaining_balance,
                });
            }
        }

        Ok(())
    }
}

/// Calculate the total vested amount based on linear vesting schedule
fn calculate_vested_amount(
    start_slot: u64,
    duration: u64,
    current_slot: u64,
    total_amount: u64,
) -> Result<u64> {
    // If current slot is before start, nothing is vested
    if current_slot < start_slot {
        return Ok(0);
    }

    // If current slot is after end, everything is vested
    let end_slot = start_slot.checked_add(duration).ok_or(VestingError::CalculationOverflow)?;
    if current_slot >= end_slot {
        return Ok(total_amount);
    }

    // Calculate linear vesting: (elapsed_slots / total_duration) * total_amount
    let elapsed_slots = current_slot.checked_sub(start_slot).ok_or(VestingError::CalculationOverflow)?;
    
    // Use u128 for intermediate calculation to prevent overflow
    let vested_amount = (total_amount as u128)
        .checked_mul(elapsed_slots as u128)
        .ok_or(VestingError::CalculationOverflow)?
        .checked_div(duration as u128)
        .ok_or(VestingError::CalculationOverflow)?;

    // Convert back to u64, ensuring it doesn't exceed total_amount
    let vested_amount_u64 = std::cmp::min(vested_amount as u64, total_amount);
    
    Ok(vested_amount_u64)
}

#[derive(Accounts)]
pub struct InitializeCtx<'info> {
    #[account(mut)]
    pub funder: Signer<'info>,

    /// CHECK: Beneficiary is validated by PDA seeds
    pub beneficiary: AccountInfo<'info>,

    #[account(
        init,
        payer = funder,
        space = 8 + VestingInfo::LEN,
        seeds = [beneficiary.key().as_ref()],
        bump
    )]
    pub vesting_info: Account<'info, VestingInfo>,

    pub system_program: Program<'info, System>,
}

#[derive(Accounts)]
pub struct ReleaseCtx<'info> {
    #[account(mut)]
    pub beneficiary: Signer<'info>,

    /// CHECK: Funder validation is handled by the vesting_info account data
    #[account(mut)]
    pub funder: AccountInfo<'info>,

    #[account(
        mut,
        seeds = [beneficiary.key().as_ref()],
        bump,
        constraint = vesting_info.beneficiary == beneficiary.key() @ VestingError::InvalidBeneficiary,
        constraint = vesting_info.funder == funder.key() @ VestingError::InvalidFunder,
    )]
    pub vesting_info: Account<'info, VestingInfo>,
}

#[account]
pub struct VestingInfo {
    pub released: u64,      // Amount already released to beneficiary
    pub funder: Pubkey,     // Address of the funder
    pub beneficiary: Pubkey, // Address of the beneficiary
    pub start_slot: u64,    // Slot when vesting starts
    pub duration: u64,      // Duration in slots for full vesting
}

impl VestingInfo {
    pub const LEN: usize = 8 + 32 + 32 + 8 + 8; // 88 bytes
}

#[event]
pub struct VestingInitialized {
    pub funder: Pubkey,
    pub beneficiary: Pubkey,
    pub start_slot: u64,
    pub duration: u64,
    pub amount: u64,
}

#[event]
pub struct FundsReleased {
    pub beneficiary: Pubkey,
    pub amount: u64,
    pub total_released: u64,
}

#[event]
pub struct VestingCompleted {
    pub beneficiary: Pubkey,
    pub final_amount: u64,
}

#[error_code]
pub enum VestingError {
    #[msg("Invalid duration: must be greater than 0")]
    InvalidDuration,
    
    #[msg("Invalid amount: must be greater than 0")]
    InvalidAmount,
    
    #[msg("No funds available to release")]
    NoFundsToRelease,
    
    #[msg("Insufficient funds in vesting account")]
    InsufficientFunds,
    
    #[msg("Invalid beneficiary")]
    InvalidBeneficiary,
    
    #[msg("Invalid funder")]
    InvalidFunder,
    
    #[msg("Calculation overflow")]
    CalculationOverflow,
}