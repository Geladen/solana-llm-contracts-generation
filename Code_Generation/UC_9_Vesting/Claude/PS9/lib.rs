#![allow(unexpected_cfgs)]
use anchor_lang::prelude::*;

declare_id!("75iGkEMwSdRuoFRa2ZHnPrviHK3g8WrCQQ9WbgHefc8J");

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
        let current_slot = clock.slot;

        // Validate that start slot is not in the past
        require!(start_slot >= current_slot, VestingError::InvalidStartSlot);

        // Validate duration is positive
        require!(duration > 0, VestingError::InvalidDuration);

        // Validate amount is positive
        require!(lamports_amount > 0, VestingError::InvalidAmount);

        let vesting_info = &mut ctx.accounts.vesting_info;

        // Initialize vesting account data
        vesting_info.released = 0;
        vesting_info.funder = ctx.accounts.funder.key();
        vesting_info.beneficiary = ctx.accounts.beneficiary.key();
        vesting_info.start_slot = start_slot;
        vesting_info.duration = duration;

        // Transfer lamports from funder to vesting PDA
        let transfer_instruction = anchor_lang::system_program::Transfer {
            from: ctx.accounts.funder.to_account_info(),
            to: ctx.accounts.vesting_info.to_account_info(),
        };

        let cpi_ctx = CpiContext::new(
            ctx.accounts.system_program.to_account_info(),
            transfer_instruction,
        );

        anchor_lang::system_program::transfer(cpi_ctx, lamports_amount)?;

        msg!(
            "Vesting initialized: {} SOL over {} slots starting at slot {}",
            lamports_amount as f64 / 1_000_000_000.0,
            duration,
            start_slot
        );

        Ok(())
    }

    pub fn release(ctx: Context<ReleaseCtx>) -> Result<()> {
        let clock = Clock::get()?;
        let current_slot = clock.slot;

        // Read vesting info data first to avoid borrowing conflicts
        let start_slot = ctx.accounts.vesting_info.start_slot;
        let duration = ctx.accounts.vesting_info.duration;
        let already_released = ctx.accounts.vesting_info.released;
        let current_lamports = ctx.accounts.vesting_info.to_account_info().lamports();

        // Calculate total amount (current balance + already released)
        let total_amount = current_lamports + already_released;

        // Calculate total vested amount based on current slot
        let total_vested = calculate_vested_amount(
            start_slot,
            duration,
            current_slot,
            total_amount,
        );

        // Calculate releasable amount (total vested minus already released)
        let releasable = total_vested.saturating_sub(already_released);

        msg!(
            "Total vested: {}, Already released: {}, Releasable: {}",
            total_vested,
            already_released,
            releasable
        );

        // If no funds to release, return early
        if releasable == 0 {
            msg!("No funds available for release at this time");
            return Ok(());
        }

        // Check if this release would fully vest all funds
        let is_fully_vested = total_vested >= total_amount;

        if is_fully_vested {
            // Full release - transfer vested funds to beneficiary and account rent to funder
            let vesting_account_info = ctx.accounts.vesting_info.to_account_info();
            let funder_account_info = ctx.accounts.funder.to_account_info();
            
            // Get the minimum rent for the account (this is what should be returned to funder)
            let rent = Rent::get()?;
            let account_rent = rent.minimum_balance(vesting_account_info.data_len());
            
            // Calculate the actual vested funds (total lamports minus account rent)
            let vested_funds = current_lamports.saturating_sub(account_rent);
            
            // Transfer vested funds to beneficiary
            **ctx.accounts.beneficiary.try_borrow_mut_lamports()? = ctx
                .accounts
                .beneficiary
                .lamports()
                .checked_add(vested_funds)
                .ok_or(VestingError::ArithmeticOverflow)?;
            
            // Transfer account rent back to funder
            **funder_account_info.try_borrow_mut_lamports()? = funder_account_info
                .lamports()
                .checked_add(account_rent)
                .ok_or(VestingError::ArithmeticOverflow)?;
            
            // Zero out the vesting account
            **vesting_account_info.try_borrow_mut_lamports()? = 0;

            msg!("Vesting completed: {} SOL released to beneficiary, {} SOL rent returned to funder", 
                 vested_funds as f64 / 1_000_000_000.0,
                 account_rent as f64 / 1_000_000_000.0);
        } else {
            // Partial release
            **ctx.accounts.vesting_info.to_account_info().try_borrow_mut_lamports()? = current_lamports
                .checked_sub(releasable)
                .ok_or(VestingError::InsufficientFunds)?;

            **ctx.accounts.beneficiary.try_borrow_mut_lamports()? = ctx
                .accounts
                .beneficiary
                .lamports()
                .checked_add(releasable)
                .ok_or(VestingError::ArithmeticOverflow)?;

            // Update released amount
            ctx.accounts.vesting_info.released = already_released
                .checked_add(releasable)
                .ok_or(VestingError::ArithmeticOverflow)?;

            msg!("Partial release: {} SOL released to beneficiary", 
                 releasable as f64 / 1_000_000_000.0);
        }

        Ok(())
    }
}

// Helper function to calculate vested amount based on linear vesting
fn calculate_vested_amount(
    start_slot: u64,
    duration: u64,
    current_slot: u64,
    total_amount: u64,
) -> u64 {
    // If before start, nothing is vested
    if current_slot < start_slot {
        return 0;
    }

    // If after vesting period, everything is vested
    let end_slot = start_slot.saturating_add(duration);
    if current_slot >= end_slot {
        return total_amount;
    }

    // Calculate linear vesting progress
    let elapsed_slots = current_slot.saturating_sub(start_slot);
    
    // Use checked arithmetic to prevent overflow
    let vested_amount = (total_amount as u128)
        .checked_mul(elapsed_slots as u128)
        .and_then(|x| x.checked_div(duration as u128))
        .and_then(|x| x.try_into().ok())
        .unwrap_or(0);

    vested_amount
}

#[derive(Accounts)]
pub struct InitializeCtx<'info> {
    #[account(mut)]
    pub funder: Signer<'info>,
    
    /// CHECK: This account is only used as a reference for the beneficiary
    pub beneficiary: UncheckedAccount<'info>,
    
    #[account(
        init,
        payer = funder,
        space = 8 + VestingInfo::INIT_SPACE,
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
    
    /// CHECK: This account is used to verify funder and potentially return rent
    #[account(mut)]
    pub funder: UncheckedAccount<'info>,
    
    #[account(
        mut,
        seeds = [beneficiary.key().as_ref()],
        bump,
        constraint = vesting_info.beneficiary == beneficiary.key() @ VestingError::InvalidBeneficiary,
        constraint = vesting_info.funder == funder.key() @ VestingError::InvalidFunder
    )]
    pub vesting_info: Account<'info, VestingInfo>,
}

#[account]
#[derive(InitSpace)]
pub struct VestingInfo {
    pub released: u64,      // Amount already released to beneficiary
    pub funder: Pubkey,     // Address of the funder
    pub beneficiary: Pubkey, // Address of the beneficiary
    pub start_slot: u64,    // Slot when vesting starts
    pub duration: u64,      // Duration of vesting in slots
}

#[error_code]
pub enum VestingError {
    #[msg("Start slot cannot be in the past")]
    InvalidStartSlot,
    
    #[msg("Duration must be greater than zero")]
    InvalidDuration,
    
    #[msg("Amount must be greater than zero")]
    InvalidAmount,
    
    #[msg("Invalid beneficiary")]
    InvalidBeneficiary,
    
    #[msg("Invalid funder")]
    InvalidFunder,
    
    #[msg("Insufficient funds in vesting account")]
    InsufficientFunds,
    
    #[msg("Arithmetic overflow")]
    ArithmeticOverflow,
}