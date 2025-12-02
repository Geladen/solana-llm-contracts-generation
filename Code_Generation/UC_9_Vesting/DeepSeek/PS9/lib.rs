use anchor_lang::prelude::*;
use anchor_lang::system_program;

declare_id!("AnNzHmxtgEHyhyWb5BLqV1a7DdGTyG8c5re5VtLhJDZv");

#[program]
pub mod vesting_contract {
    use super::*;

    pub fn initialize(
        ctx: Context<Initialize>,
        start_slot: u64,
        duration: u64,
        lamports_amount: u64,
    ) -> Result<()> {
        // Validate start slot is in the future
        let current_slot = Clock::get()?.slot;
        require!(start_slot > current_slot, VestingError::InvalidStartSlot);

        // Validate duration is reasonable
        require!(duration > 0, VestingError::InvalidDuration);
        
        let vesting_info = &mut ctx.accounts.vesting_info;
        
        // Initialize vesting schedule
        vesting_info.funder = ctx.accounts.funder.key();
        vesting_info.beneficiary = ctx.accounts.beneficiary.key();
        vesting_info.start_slot = start_slot;
        vesting_info.duration = duration;
        vesting_info.released = 0;

        // Transfer funds to vesting PDA
        let cpi_context = CpiContext::new(
            ctx.accounts.system_program.to_account_info(),
            system_program::Transfer {
                from: ctx.accounts.funder.to_account_info(),
                to: ctx.accounts.vesting_info.to_account_info(),
            },
        );
        system_program::transfer(cpi_context, lamports_amount)?;

        msg!(
            "Vesting schedule created: {} lamports vested from slot {} for {} slots",
            lamports_amount,
            start_slot,
            duration
        );

        Ok(())
    }

    pub fn release(ctx: Context<Release>) -> Result<()> {
        let current_slot = Clock::get()?.slot;
        
        // Store account info references upfront to avoid borrowing issues
        let vesting_account_info = ctx.accounts.vesting_info.to_account_info();
        let beneficiary_account_info = ctx.accounts.beneficiary.to_account_info();
        let funder_account_info = ctx.accounts.funder.to_account_info();
        
        // Get values before mutable borrow of vesting_info account data
        let start_slot = ctx.accounts.vesting_info.start_slot;
        let duration = ctx.accounts.vesting_info.duration;
        let already_released = ctx.accounts.vesting_info.released;
        let current_balance = vesting_account_info.lamports();

        // Calculate total original amount (current balance + already released)
        let total_amount = current_balance
            .checked_add(already_released)
            .ok_or(VestingError::VestingCalculationOverflow)?;

        // Calculate vested amount
        let vested_amount = calculate_vested_amount(
            start_slot,
            duration,
            current_slot,
            total_amount,
        )?;

        // Calculate releasable amount (vested minus already released)
        let releasable_amount = vested_amount.saturating_sub(already_released);
        
        require!(releasable_amount > 0, VestingError::NoReleasableAmount);

        // Now update the vesting info account data
        let vesting_info = &mut ctx.accounts.vesting_info;
        vesting_info.released = vested_amount;

        // Check if this is the final release (all funds are vested)
        let is_final_release = vested_amount >= total_amount;
        
        if is_final_release {
            // Final release: transfer user funds to beneficiary and rent to funder
            
            // Calculate rent exempt amount
            let rent_exempt_balance = Rent::get()?.minimum_balance(VestingInfo::LEN);
            
            // User funds = total balance - rent exempt balance
            let user_funds = current_balance.saturating_sub(rent_exempt_balance);
            
            if user_funds > 0 {
                // Transfer user funds to beneficiary
                **vesting_account_info.try_borrow_mut_lamports()? -= user_funds;
                **beneficiary_account_info.try_borrow_mut_lamports()? += user_funds;
            }
            
            // Return rent to funder and close account
            let remaining_rent = vesting_account_info.lamports();
            if remaining_rent > 0 {
                **vesting_account_info.try_borrow_mut_lamports()? = 0;
                **funder_account_info.try_borrow_mut_lamports()? += remaining_rent;
            }
            
            msg!(
                "Final release: {} user lamports to beneficiary, {} rent lamports returned to funder. Account closed.",
                user_funds,
                remaining_rent
            );
        } else {
            // Partial release: transfer only the releasable amount to beneficiary
            **vesting_account_info.try_borrow_mut_lamports()? -= releasable_amount;
            **beneficiary_account_info.try_borrow_mut_lamports()? += releasable_amount;
            
            msg!(
                "Partial release: {} lamports to beneficiary. Total released: {}",
                releasable_amount,
                vesting_info.released
            );
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
        space = VestingInfo::LEN,
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
    
    /// CHECK: Funder account for rent return
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
    pub const LEN: usize = 8 + // discriminator
        8 + // released: u64
        32 + // funder: Pubkey
        32 + // beneficiary: Pubkey  
        8 + // start_slot: u64
        8; // duration: u64
}

#[error_code]
pub enum VestingError {
    #[msg("Start slot must be in the future")]
    InvalidStartSlot,
    #[msg("Duration must be greater than zero")]
    InvalidDuration,
    #[msg("No releasable amount available at this time")]
    NoReleasableAmount,
    #[msg("Vesting calculation overflow")]
    VestingCalculationOverflow,
}

// Helper function to calculate vested amount
fn calculate_vested_amount(
    start_slot: u64,
    duration: u64,
    current_slot: u64,
    total_amount: u64,
) -> Result<u64> {
    if current_slot < start_slot {
        return Ok(0);
    }

    if current_slot >= start_slot + duration {
        return Ok(total_amount);
    }

    let elapsed_slots = current_slot
        .checked_sub(start_slot)
        .ok_or(VestingError::VestingCalculationOverflow)?;

    let vested_amount = total_amount
        .checked_mul(elapsed_slots)
        .ok_or(VestingError::VestingCalculationOverflow)?
        .checked_div(duration)
        .ok_or(VestingError::VestingCalculationOverflow)?;

    Ok(vested_amount)
}