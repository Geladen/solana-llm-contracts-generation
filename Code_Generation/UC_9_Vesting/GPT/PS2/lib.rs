use anchor_lang::prelude::*;

declare_id!("GuhjdLQqHJoxUwE5zbAKbnKmezRbkdZFjnsGrH6VBn1u");

#[program]
pub mod vesting_gpt {
    use super::*;

    pub fn initialize(
        ctx: Context<Initialize>,
        start_slot: u64,
        duration: u64,
        lamports_amount: u64,
    ) -> Result<()> {
        let vesting = &mut ctx.accounts.vesting_info;

        require!(start_slot > Clock::get()?.slot, VestingError::InvalidStartSlot);
        require!(duration > 0, VestingError::InvalidDuration);

        vesting.released = 0;
        vesting.funder = ctx.accounts.funder.key();
        vesting.beneficiary = ctx.accounts.beneficiary.key();
        vesting.start_slot = start_slot;
        vesting.duration = duration;

        // Transfer funds from funder to vesting PDA
        let cpi_ctx = CpiContext::new(
            ctx.accounts.system_program.to_account_info(),
            anchor_lang::system_program::Transfer {
                from: ctx.accounts.funder.to_account_info(),
                to: ctx.accounts.vesting_info.to_account_info(),
            },
        );
        anchor_lang::system_program::transfer(cpi_ctx, lamports_amount)?;

        Ok(())
    }

    pub fn release(ctx: Context<Release>) -> Result<()> {
    // Mutable reference to the vesting account
    let vesting = &mut ctx.accounts.vesting_info;

    // Determine current slot
    let clock = Clock::get()?;
    let current_slot = clock.slot;

    // Calculate how much is vested
    let total_vesting_amount = **ctx.accounts.vesting_info.to_account_info().lamports();
    let vested_amount = if current_slot >= vesting.start_slot + vesting.duration {
        total_vesting_amount
    } else if current_slot < vesting.start_slot {
        0
    } else {
        let elapsed = current_slot - vesting.start_slot;
        total_vesting_amount * elapsed / vesting.duration
    };

    // Determine how much can be released now
    let amount_to_transfer = vested_amount.checked_sub(vesting.released).unwrap_or(0);

    if amount_to_transfer == 0 {
        return Err(ErrorCode::NothingToRelease.into());
    }

    // Use temporary bindings to keep AccountInfos alive while borrowing lamports
    let vesting_ai = ctx.accounts.vesting_info.to_account_info();
    let beneficiary_ai = ctx.accounts.beneficiary.to_account_info();
    let system_program_ai = ctx.accounts.system_program.to_account_info();

    // Perform the lamports transfer via CPI
    let cpi_ctx = CpiContext::new(
        system_program_ai,
        system_program::Transfer {
            from: vesting_ai.clone(),
            to: beneficiary_ai.clone(),
        },
    );
    system_program::transfer(cpi_ctx, amount_to_transfer)?;

    // Update the released amount
    vesting.released = vesting.released.checked_add(amount_to_transfer).unwrap();

    Ok(())
}

}

#[derive(Accounts)]
pub struct Initialize<'info> {
    /// CHECK: funder signs the transaction but we don't read/write its data
    #[account(mut)]
    pub funder: Signer<'info>,

    /// CHECK: beneficiary is only used for PDA derivation and for receiving funds; no data is read/written
    pub beneficiary: UncheckedAccount<'info>,

    #[account(
        init,
        payer = funder,
        space = 8 + std::mem::size_of::<VestingInfo>(),
        seeds = [beneficiary.key().as_ref()],
        bump
    )]
    pub vesting_info: Account<'info, VestingInfo>,

    pub system_program: Program<'info, System>,
}

// Define the context for Release
#[derive(Accounts)]
pub struct Release<'info> {
    /// CHECK: the beneficiary will receive funds
    #[account(mut)]
    pub beneficiary: AccountInfo<'info>,

    /// CHECK: the funder/vesting account
    #[account(mut)]
    pub vesting_info: Account<'info, VestingInfo>,

    pub system_program: Program<'info, System>,
}



// Vesting account
#[account]
pub struct VestingInfo {
    pub beneficiary: Pubkey,
    pub start_slot: u64,
    pub duration: u64,
    pub released: u64,
}

impl VestingInfo {
    pub const LEN: usize = 8  // discriminator
        + 8                  // released
        + 32                 // funder
        + 32                 // beneficiary
        + 8                  // start_slot
        + 8;                 // duration
}

#[error_code]
pub enum VestingError {
    #[msg("Vesting start slot must be in the future")]
    InvalidStartSlot,
    #[msg("Vesting duration must be greater than 0")]
    InvalidDuration,
    #[msg("Vesting has not started yet")]
    VestingNotStarted,
    #[msg("Nothing to release")]
    NothingToRelease,
    #[msg("Overflow during calculation")]
    Overflow,
}
