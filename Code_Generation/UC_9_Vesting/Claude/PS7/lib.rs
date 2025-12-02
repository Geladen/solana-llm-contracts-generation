use anchor_lang::prelude::*;

declare_id!("AZYjsnU6hSftgHNxXxbb5UY4pm33oHZigDDCw3mTCt5L");

#[program]
pub mod vesting_program {
    use super::*;

    /// Initialize a new vesting schedule
    /// Only the funder can call this function
    pub fn initialize(
        ctx: Context<InitializeCtx>,
        start_slot: u64,
        duration: u64,
        lamports_amount: u64,
    ) -> Result<()> {
        // Validate parameters
        require!(duration > 0, VestingError::InvalidDuration);
        require!(lamports_amount > 0, VestingError::InvalidAmount);
        require!(
            start_slot >= Clock::get()?.slot,
            VestingError::InvalidStartSlot
        );

        // Transfer lamports from funder to vesting PDA first
        let transfer_ix = anchor_lang::system_program::Transfer {
            from: ctx.accounts.funder.to_account_info(),
            to: ctx.accounts.vesting_info.to_account_info(),
        };

        anchor_lang::system_program::transfer(
            CpiContext::new(
                ctx.accounts.system_program.to_account_info(),
                transfer_ix,
            ),
            lamports_amount,
        )?;

        // Initialize vesting account after transfer
        let vesting_info = &mut ctx.accounts.vesting_info;
        vesting_info.released = 0;
        vesting_info.funder = ctx.accounts.funder.key();
        vesting_info.beneficiary = ctx.accounts.beneficiary.key();
        vesting_info.start_slot = start_slot;
        vesting_info.duration = duration;

        msg!(
            "Vesting initialized: beneficiary={}, start_slot={}, duration={}, amount={}",
            vesting_info.beneficiary,
            start_slot,
            duration,
            lamports_amount
        );

        Ok(())
    }

    /// Release vested funds to the beneficiary
    /// Only the beneficiary can call this function
    pub fn release(ctx: Context<ReleaseCtx>) -> Result<()> {
        let current_slot = Clock::get()?.slot;
        let rent_exempt_reserve = Rent::get()?.minimum_balance(VestingInfo::INIT_SPACE + 8);

        // Calculate total balance and vestable amount (excluding rent reserve)
        let current_balance = ctx.accounts.vesting_info.to_account_info().lamports();
        let total_balance = current_balance + ctx.accounts.vesting_info.released;
        let vestable_amount = total_balance
            .checked_sub(rent_exempt_reserve)
            .ok_or(VestingError::ArithmeticOverflow)?;

        // Calculate vested amount based on vestable amount only
        let vested_amount = calculate_vested_amount(
            vestable_amount,
            ctx.accounts.vesting_info.start_slot,
            ctx.accounts.vesting_info.duration,
            current_slot,
        )?;

        // Calculate releasable amount
        let releasable_amount = vested_amount
            .checked_sub(ctx.accounts.vesting_info.released)
            .ok_or(VestingError::ArithmeticOverflow)?;

        require!(releasable_amount > 0, VestingError::NoFundsToRelease);

        // Update released amount
        ctx.accounts.vesting_info.released = ctx.accounts.vesting_info.released
            .checked_add(releasable_amount)
            .ok_or(VestingError::ArithmeticOverflow)?;

        // Transfer vested lamports to beneficiary
        **ctx.accounts.vesting_info.to_account_info().try_borrow_mut_lamports()? -= releasable_amount;
        **ctx.accounts.beneficiary.to_account_info().try_borrow_mut_lamports()? += releasable_amount;

        msg!(
            "Released {} lamports to beneficiary {}",
            releasable_amount,
            ctx.accounts.beneficiary.key()
        );

        // Check if all vestable funds have been released and close account if needed
        let remaining_balance = ctx.accounts.vesting_info.to_account_info().lamports();
        
        msg!("Remaining balance: {}, Rent exempt reserve: {}, Released: {}, Vestable: {}", 
             remaining_balance, rent_exempt_reserve, ctx.accounts.vesting_info.released, vestable_amount);
        
        // Check if all vestable funds have been released
        if ctx.accounts.vesting_info.released >= vestable_amount {
            msg!("All vestable funds released. Closing account - transferring {} lamports to funder", remaining_balance);
            // Close the vesting account and return remaining lamports (rent reserve) to funder
            **ctx.accounts.vesting_info.to_account_info().try_borrow_mut_lamports()? = 0;
            **ctx.accounts.funder.to_account_info().try_borrow_mut_lamports()? += remaining_balance;
            
            msg!("Vesting completed. Account closed. {} lamports returned to funder.", remaining_balance);
        }

        Ok(())
    }
}

/// Calculate the vested amount based on linear vesting schedule
fn calculate_vested_amount(
    total_amount: u64,
    start_slot: u64,
    duration: u64,
    current_slot: u64,
) -> Result<u64> {
    // If vesting hasn't started yet
    if current_slot < start_slot {
        return Ok(0);
    }

    // If vesting period is complete
    let end_slot = start_slot
        .checked_add(duration)
        .ok_or(VestingError::ArithmeticOverflow)?;
    
    if current_slot >= end_slot {
        return Ok(total_amount);
    }

    // Linear vesting calculation
    let elapsed_slots = current_slot
        .checked_sub(start_slot)
        .ok_or(VestingError::ArithmeticOverflow)?;

    let vested_amount = (total_amount as u128)
        .checked_mul(elapsed_slots as u128)
        .ok_or(VestingError::ArithmeticOverflow)?
        .checked_div(duration as u128)
        .ok_or(VestingError::ArithmeticOverflow)?;

    Ok(vested_amount as u64)
}

#[derive(Accounts)]
pub struct InitializeCtx<'info> {
    #[account(mut)]
    pub funder: Signer<'info>,
    
    /// CHECK: This account is only used as a reference for PDA derivation
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
    
    /// CHECK: This account is validated through has_one constraint and needs to be mutable for rent return
    #[account(mut)]
    pub funder: UncheckedAccount<'info>,
    
    #[account(
        mut,
        seeds = [beneficiary.key().as_ref()],
        bump,
        has_one = beneficiary,
        has_one = funder,
    )]
    pub vesting_info: Account<'info, VestingInfo>,
}

#[account]
pub struct VestingInfo {
    pub released: u64,      // 8 bytes
    pub funder: Pubkey,     // 32 bytes
    pub beneficiary: Pubkey, // 32 bytes
    pub start_slot: u64,    // 8 bytes
    pub duration: u64,      // 8 bytes
}

impl VestingInfo {
    pub const INIT_SPACE: usize = 8 + 32 + 32 + 8 + 8; // 88 bytes
}

#[error_code]
pub enum VestingError {
    #[msg("Invalid duration: must be greater than 0")]
    InvalidDuration,
    
    #[msg("Invalid amount: must be greater than 0")]
    InvalidAmount,
    
    #[msg("Invalid start slot: cannot be in the past")]
    InvalidStartSlot,
    
    #[msg("No funds available for release")]
    NoFundsToRelease,
    
    #[msg("Arithmetic overflow occurred")]
    ArithmeticOverflow,
}