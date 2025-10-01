use anchor_lang::prelude::*; // includes all common macros, types, and traits

declare_id!("6rP3rxQatNCUkh8Dh5Wr5qs4CyTU9k9hRRreG9gkPQya");

#[program]
pub mod vesting_gpt {
    use super::*;

    pub fn initialize(
        ctx: Context<InitializeCtx>,
        start_slot: u64,
        duration: u64,
        lamports_amount: u64,
    ) -> Result<()> {
        let clock = Clock::get()?;
        require!(start_slot > clock.slot, ErrorCode::InvalidStartSlot);
        require!(duration > 0, ErrorCode::InvalidDuration);

        let vesting = &mut ctx.accounts.vesting_info;
        vesting.funder = ctx.accounts.funder.key();
        vesting.beneficiary = ctx.accounts.beneficiary.key();
        vesting.start_slot = start_slot;
        vesting.duration = duration;
        vesting.released = 0;
        vesting.bump = ctx.bumps["vesting_info"];

        // Transfer lamports from funder to vesting PDA
        **ctx.accounts.funder.to_account_info().try_borrow_mut_lamports()? -= lamports_amount;
        **ctx.accounts.vesting_info.to_account_info().try_borrow_mut_lamports()? += lamports_amount;

        Ok(())
    }

    pub fn release(ctx: Context<ReleaseCtx>) -> Result<()> {
        let clock = Clock::get()?;
        let vesting = &mut ctx.accounts.vesting_info;
        let vesting_ai = ctx.accounts.vesting_info.to_account_info();
        let beneficiary_ai = ctx.accounts.beneficiary.to_account_info();

        let total_vesting = **vesting_ai.lamports.borrow() + vesting.released;

        let releasable = if clock.slot < vesting.start_slot {
            0
        } else if clock.slot >= vesting.start_slot + vesting.duration {
            total_vesting
        } else {
            total_vesting * (clock.slot - vesting.start_slot) / vesting.duration
        };

        let releasable = releasable.saturating_sub(vesting.released);

        if releasable == 0 {
            return Ok(());
        }

        // Transfer lamports to beneficiary
        **vesting_ai.try_borrow_mut_lamports()? -= releasable;
        **beneficiary_ai.try_borrow_mut_lamports()? += releasable;

        vesting.released += releasable;

        // Close account if fully vested
        if **vesting_ai.lamports.borrow() == 0 {
            let seeds = &[ctx.accounts.beneficiary.key().as_ref(), &[vesting.bump]];
            let signer_seeds = &[&seeds[..]];

            **ctx.accounts.funder.to_account_info().try_borrow_mut_lamports()? +=
                **vesting_ai.try_borrow_mut_lamports()?;
            // Zero out vesting account data
            let mut vesting_data = vesting_ai.try_borrow_mut_data()?;
            for byte in vesting_data.iter_mut() {
                *byte = 0;
            }
        }

        Ok(())
    }
}

#[account]
pub struct VestingInfo {
    pub funder: Pubkey,
    pub beneficiary: Pubkey,
    pub start_slot: u64,
    pub duration: u64,
    pub released: u64,
    pub bump: u8,
}

#[derive(Accounts)]
pub struct InitializeCtx<'info> {
    #[account(mut)]
    pub funder: Signer<'info>,
    /// CHECK: just a reference
    pub beneficiary: AccountInfo<'info>,
    #[account(
        init,
        payer = funder,
        space = 8 + 32 + 32 + 8 + 8 + 8 + 1,
        seeds = [beneficiary.key().as_ref()],
        bump
    )]
    pub vesting_info: Account<'info, VestingInfo>,
    pub system_program: Program<'info, System>,
}

#[derive(Accounts)]
pub struct ReleaseCtx<'info> {
    #[account(mut, signer)]
    pub beneficiary: AccountInfo<'info>,
    #[account(mut)]
    pub funder: AccountInfo<'info>,
    #[account(
        mut,
        seeds = [beneficiary.key().as_ref()],
        bump = vesting_info.bump
    )]
    pub vesting_info: Account<'info, VestingInfo>,
}

#[error_code]
pub enum ErrorCode {
    #[msg("Provided start_slot is in the past.")]
    InvalidStartSlot,
    #[msg("Duration must be > 0.")]
    InvalidDuration,
}
