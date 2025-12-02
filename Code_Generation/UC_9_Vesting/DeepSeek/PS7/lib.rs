use anchor_lang::prelude::*;
use anchor_lang::system_program;

declare_id!("EnzDngVco8awhAY8QiKdGdmCCnMpq4B9Q7HQ8zsz5qWR");

#[program]
pub mod vesting {
    use super::*;

    pub fn initialize(
        ctx: Context<InitializeCtx>,
        start_slot: u64,
        duration: u64,
        lamports_amount: u64,
    ) -> Result<()> {
        require!(duration > 0, VestingError::InvalidDuration);
        require!(lamports_amount > 0, VestingError::InvalidAmount);

        let vesting_info = &mut ctx.accounts.vesting_info;
        
        vesting_info.released = 0;
        vesting_info.funder = ctx.accounts.funder.key();
        vesting_info.beneficiary = ctx.accounts.beneficiary.key();
        vesting_info.start_slot = start_slot;
        vesting_info.duration = duration;
        vesting_info.total_amount = lamports_amount;

        let cpi_context = CpiContext::new(
            ctx.accounts.system_program.to_account_info(),
            system_program::Transfer {
                from: ctx.accounts.funder.to_account_info(),
                to: ctx.accounts.vesting_info.to_account_info(),
            },
        );
        system_program::transfer(cpi_context, lamports_amount)?;

        msg!(
            "Vesting initialized: {} lamports vested over {} slots starting at slot {}",
            lamports_amount,
            duration,
            start_slot
        );

        Ok(())
    }

    pub fn release(ctx: Context<ReleaseCtx>) -> Result<()> {
        let current_slot = Clock::get()?.slot;
        
        // Get account info references first
        let vesting_account_info = &mut ctx.accounts.vesting_info.to_account_info();
        let beneficiary_account_info = &mut ctx.accounts.beneficiary.to_account_info();
        let funder_account_info = &mut ctx.accounts.funder.to_account_info();
        
        // Extract data needed for calculation
        let (start_slot, duration, released, total_amount) = {
            let vesting_info = &ctx.accounts.vesting_info;
            (
                vesting_info.start_slot,
                vesting_info.duration,
                vesting_info.released,
                vesting_info.total_amount,
            )
        };

        let current_balance = vesting_account_info.lamports();

        // Calculate vested amount based on the original total amount
        let vested_amount = calculate_vested_amount(
            start_slot,
            duration,
            current_slot,
            total_amount,
        )?;

        // Calculate releasable amount
        require!(vested_amount > released, VestingError::NothingToRelease);
        let releasable_amount = vested_amount - released;

        // Check if this is the final release
        let is_final_release = vested_amount == total_amount;

        // Transfer the vested funds to beneficiary
        **vesting_account_info.try_borrow_mut_lamports()? -= releasable_amount;
        **beneficiary_account_info.try_borrow_mut_lamports()? += releasable_amount;

        // Update the released amount in the account data
        let vesting_info = &mut ctx.accounts.vesting_info;
        vesting_info.released += releasable_amount;

        msg!(
            "Released {} lamports to beneficiary. Total released: {}",
            releasable_amount,
            vesting_info.released
        );

        // If this is the final release, close the account and return rent to funder
        if is_final_release {
            let remaining_balance = vesting_account_info.lamports();
            
            // Transfer any remaining balance (rent) to funder
            if remaining_balance > 0 {
                **vesting_account_info.try_borrow_mut_lamports()? -= remaining_balance;
                **funder_account_info.try_borrow_mut_lamports()? += remaining_balance;
            }
            
            // Close the account
            let vesting_info_account = &mut ctx.accounts.vesting_info;
            vesting_info_account.close(funder_account_info.clone())?;
            msg!("Vesting account closed successfully, rent returned to funder");
        }

        Ok(())
    }
}

#[derive(Accounts)]
#[instruction(start_slot: u64, duration: u64, lamports_amount: u64)]
pub struct InitializeCtx<'info> {
    #[account(mut)]
    pub funder: Signer<'info>,
    
    /// CHECK: This is the beneficiary account, no need to mutate
    pub beneficiary: AccountInfo<'info>,
    
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
    
    /// CHECK: Funder account for validation and rent reclamation
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
#[derive(InitSpace)]
pub struct VestingInfo {
    pub released: u64,
    pub funder: Pubkey,
    pub beneficiary: Pubkey,
    pub start_slot: u64,
    pub duration: u64,
    pub total_amount: u64, // Store the original total amount
}

fn calculate_vested_amount(start_slot: u64, duration: u64, current_slot: u64, total_amount: u64) -> Result<u64> {
    if current_slot <= start_slot {
        return Ok(0);
    }
    
    if current_slot >= start_slot + duration {
        return Ok(total_amount);
    }
    
    let elapsed_slots = current_slot - start_slot;
    let vested_amount = total_amount
        .checked_mul(elapsed_slots)
        .and_then(|v| v.checked_div(duration))
        .ok_or(VestingError::CalculationError)?;
    
    Ok(vested_amount)
}

#[error_code]
pub enum VestingError {
    #[msg("Invalid duration: must be greater than 0")]
    InvalidDuration,
    #[msg("Invalid amount: must be greater than 0")]
    InvalidAmount,
    #[msg("No vested funds available for release")]
    NothingToRelease,
    #[msg("Error in vesting calculation")]
    CalculationError,
}