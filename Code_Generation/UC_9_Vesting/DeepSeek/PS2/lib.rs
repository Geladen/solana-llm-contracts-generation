use anchor_lang::prelude::*;
use anchor_lang::system_program;

declare_id!("7dv7wbw15PBEFHSNDZLnPmx7oPkRKrdVybNBbHKn1BQB");

#[program]
pub mod vesting {
    use super::*;

    pub fn initialize(
        ctx: Context<Initialize>,
        start_slot: u64,
        duration: u64,
        lamports_amount: u64,
    ) -> Result<()> {
        require!(duration > 0, VestingError::InvalidDuration);
        require!(lamports_amount > 0, VestingError::InvalidAmount);

        let vesting_info = &mut ctx.accounts.vesting_info;
        vesting_info.funder = ctx.accounts.funder.key();
        vesting_info.beneficiary = ctx.accounts.beneficiary.key();
        vesting_info.start_slot = start_slot;
        vesting_info.duration = duration;
        vesting_info.released = 0;

        // Calculate total amount needed (vesting amount + rent)
        let rent = Rent::get()?;
        let rent_exempt_balance = rent.minimum_balance(VestingInfo::SIZE);
        let total_transfer_amount = lamports_amount
            .checked_add(rent_exempt_balance)
            .ok_or(VestingError::MathOverflow)?;

        // Transfer lamports to vesting PDA (vesting amount + rent)
        let cpi_context = CpiContext::new(
            ctx.accounts.system_program.to_account_info(),
            system_program::Transfer {
                from: ctx.accounts.funder.to_account_info(),
                to: ctx.accounts.vesting_info.to_account_info(),
            },
        );
        system_program::transfer(cpi_context, total_transfer_amount)?;

        Ok(())
    }

    pub fn release(ctx: Context<Release>) -> Result<()> {
        let current_slot = Clock::get()?.slot;
        
        // Get account info and balances before mutable borrow
        let vesting_account_info = &ctx.accounts.vesting_info.to_account_info();
        let available_balance = vesting_account_info.lamports();
        
        // Calculate the original vested amount (excluding rent)
        let rent_exempt_balance = Rent::get()?.minimum_balance(VestingInfo::SIZE);
        let original_vested_amount = available_balance
            .checked_add(ctx.accounts.vesting_info.released)
            .ok_or(VestingError::MathOverflow)?
            .checked_sub(rent_exempt_balance)
            .ok_or(VestingError::MathOverflow)?;

        // Calculate vested amount based on time
        let releasable_amount = calculate_vested_amount(
            ctx.accounts.vesting_info.start_slot,
            ctx.accounts.vesting_info.duration,
            ctx.accounts.vesting_info.released,
            current_slot,
            original_vested_amount,
        )?;

        require!(releasable_amount > 0, VestingError::NoVestedTokens);

        // Ensure we don't try to release more than available (excluding rent)
        let available_for_vesting = available_balance.saturating_sub(rent_exempt_balance);
        let actual_release_amount = std::cmp::min(releasable_amount, available_for_vesting);
        require!(actual_release_amount > 0, VestingError::InsufficientFunds);

        // Now we can mutably borrow vesting_info to update it
        let vesting_info = &mut ctx.accounts.vesting_info;
        
        // Update released amount
        vesting_info.released = vesting_info
            .released
            .checked_add(actual_release_amount)
            .ok_or(VestingError::MathOverflow)?;

        // Check if this is the final release (fully vested and all funds released)
        let all_funds_vested = current_slot >= vesting_info.start_slot + vesting_info.duration;
        let all_funds_released = vesting_info.released >= original_vested_amount;
        let should_close_account = all_funds_vested && all_funds_released;

        if should_close_account {
            // Transfer ALL remaining balance (including rent) to FUNDER
            let total_remaining = vesting_account_info.lamports();
            
            **vesting_account_info.try_borrow_mut_lamports()? = 0;
            **ctx.accounts.funder.try_borrow_mut_lamports()? = ctx
                .accounts
                .funder
                .lamports()
                .checked_add(total_remaining)
                .ok_or(VestingError::MathOverflow)?;
        } else {
            // Normal release - transfer vested amount to beneficiary
            **vesting_account_info.try_borrow_mut_lamports()? = available_balance
                .checked_sub(actual_release_amount)
                .ok_or(VestingError::InsufficientFunds)?;

            **ctx.accounts.beneficiary.to_account_info().try_borrow_mut_lamports()? = ctx
                .accounts
                .beneficiary
                .to_account_info()
                .lamports()
                .checked_add(actual_release_amount)
                .ok_or(VestingError::MathOverflow)?;
        }

        Ok(())
    }
}

#[derive(Accounts)]
pub struct Initialize<'info> {
    #[account(mut)]
    pub funder: Signer<'info>,
    /// CHECK: This is the beneficiary who will receive vested funds
    pub beneficiary: AccountInfo<'info>,
    #[account(
        init,
        payer = funder,
        space = VestingInfo::SIZE,
        seeds = [beneficiary.key().as_ref()],
        bump
    )]
    pub vesting_info: Account<'info, VestingInfo>,
    pub system_program: Program<'info, System>,
}

#[derive(Accounts)]
pub struct Release<'info> {
    #[account(mut)]
    pub beneficiary: Signer<'info>,
    /// CHECK: The original funder who receives rent back when account is closed
    #[account(mut)]
    pub funder: AccountInfo<'info>,
    #[account(
        mut,
        seeds = [beneficiary.key().as_ref()],
        bump,
        has_one = beneficiary,
        has_one = funder
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
    pub const SIZE: usize = 8 + // discriminator
        8 + // released: u64
        32 + // funder: Pubkey
        32 + // beneficiary: Pubkey  
        8 + // start_slot: u64
        8; // duration: u64
}

#[error_code]
pub enum VestingError {
    #[msg("Vesting duration must be greater than zero")]
    InvalidDuration,
    #[msg("Amount must be greater than zero")]
    InvalidAmount,
    #[msg("No vested tokens available for release")]
    NoVestedTokens,
    #[msg("Insufficient funds in vesting account")]
    InsufficientFunds,
    #[msg("Math overflow occurred")]
    MathOverflow,
    #[msg("Vesting has not started yet")]
    VestingNotStarted,
}

// Fixed vested amount calculation
fn calculate_vested_amount(
    start_slot: u64,
    duration: u64,
    already_released: u64,
    current_slot: u64,
    total_amount: u64,
) -> Result<u64> {
    if current_slot < start_slot {
        // Vesting hasn't started yet
        return Ok(0);
    }

    let elapsed_slots = current_slot
        .checked_sub(start_slot)
        .ok_or(VestingError::MathOverflow)?;

    // Calculate vested fraction
    let vested_amount = if elapsed_slots >= duration {
        // Fully vested - return remaining amount
        total_amount
            .checked_sub(already_released)
            .ok_or(VestingError::MathOverflow)?
    } else {
        // Linear vesting: (elapsed_slots / duration) * total_amount
        let numerator = elapsed_slots
            .checked_mul(total_amount)
            .ok_or(VestingError::MathOverflow)?;
        
        let vested_fraction = numerator
            .checked_div(duration)
            .ok_or(VestingError::MathOverflow)?;
        
        // Only release the amount that hasn't been released yet
        if vested_fraction > already_released {
            vested_fraction
                .checked_sub(already_released)
                .ok_or(VestingError::MathOverflow)?
        } else {
            0
        }
    };

    Ok(vested_amount)
}