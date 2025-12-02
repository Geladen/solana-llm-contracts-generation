use anchor_lang::prelude::*;

declare_id!("BWT4qFrQbuTE1QNoPW1wCVJojXs9dmmwgUQMYWvaHqv7");

// Limits and space calculations
const MAX_CAMPAIGN_NAME_LEN: usize = 64; // bytes
const CAMPAIGN_SIZE: usize = 8 + 4 + MAX_CAMPAIGN_NAME_LEN + 32 + 8 + 8; // discriminator + string prefix + name + owner + end_slot + goal
const DEPOSIT_SIZE: usize = 8 + 8; // discriminator + u64

#[program]
pub mod crowdfund {
    use super::*;

    /// Initialize a new campaign PDA. Must be signed by the campaign owner.
    /// Seeds: [campaign_name]
    pub fn initialize(
        ctx: Context<InitializeCtx>,
        campaign_name: String,
        end_donate_slot: u64,
        goal_in_lamports: u64,
    ) -> Result<()> {
        // Validate name length
        if campaign_name.len() > MAX_CAMPAIGN_NAME_LEN {
            return err!(ErrorCode::CampaignNameTooLong);
        }

        let clock = Clock::get()?;
        // Reject if end slot is before current slot
        if end_donate_slot < clock.slot {
            return err!(ErrorCode::EndSlotInPast);
        }

        if goal_in_lamports == 0 {
            return err!(ErrorCode::GoalMustBeNonZero);
        }

        let campaign = &mut ctx.accounts.campaign_pda;
        campaign.campaign_name = campaign_name;
        campaign.campaign_owner = *ctx.accounts.campaign_owner.key;
        campaign.end_donate_slot = end_donate_slot;
        campaign.goal_in_lamports = goal_in_lamports;

        Ok(())
    }

    /// Donate lamports to a campaign. Donor must sign. Creates deposit PDA for donor if needed.
    /// Seeds for deposit: [b"deposit", campaign_name, donor_pubkey]
    pub fn donate(
        ctx: Context<DonateCtx>,
        _campaign_name: String,
        donated_lamports: u64,
    ) -> Result<()> {
        if donated_lamports == 0 {
            return err!(ErrorCode::InvalidDonationAmount);
        }

        let clock = Clock::get()?;
        if clock.slot > ctx.accounts.campaign_pda.end_donate_slot {
            return err!(ErrorCode::CampaignEnded);
        }

        // Transfer lamports from donor -> campaign PDA using CPI (donor is signer)
        let cpi_accounts = anchor_lang::system_program::Transfer {
            from: ctx.accounts.donor.to_account_info(),
            to: ctx.accounts.campaign_pda.to_account_info(),
        };
        let cpi_ctx = CpiContext::new(ctx.accounts.system_program.to_account_info(), cpi_accounts);
        anchor_lang::system_program::transfer(cpi_ctx, donated_lamports)?;

        // Update deposit PDA
        let deposit = &mut ctx.accounts.deposit_pda;
        deposit
            .total_donated
            .checked_add(donated_lamports)
            .ok_or(error!(ErrorCode::NumericalOverflow))
            .map(|v| deposit.total_donated = v)?;

        Ok(())
    }

    /// Withdraw all donations to the campaign owner if goal was reached. Must be signed by owner.
    /// This transfers all lamports _above_ the rent-exempt minimum for the campaign account to owner,
    /// leaving the campaign PDA with exactly the rent-exempt minimum so the account and its state remain.
    pub fn withdraw(ctx: Context<WithdrawCtx>, _campaign_name: String) -> Result<()> {
        let clock = Clock::get()?;
        if clock.slot <= ctx.accounts.campaign_pda.end_donate_slot {
            return err!(ErrorCode::CampaignStillActive);
        }

        // Determine how many lamports are available (exclude rent-exempt minimum for the campaign account)
        let rent = Rent::get()?;
        let min_balance = rent.minimum_balance(CAMPAIGN_SIZE);

        let campaign_ai = ctx.accounts.campaign_pda.to_account_info();
        let campaign_balance = **campaign_ai.lamports.borrow();

        // Available funds that can be withdrawn
        let available = campaign_balance
            .checked_sub(min_balance)
            .ok_or(error!(ErrorCode::InsufficientFundsInCampaign))?;

        if available < ctx.accounts.campaign_pda.goal_in_lamports {
            return err!(ErrorCode::GoalNotReached);
        }

        // Perform lamports move: campaign_pda -> campaign_owner
        **campaign_ai.lamports.borrow_mut() = campaign_balance - available; // should equal min_balance
        **ctx.accounts.campaign_owner.to_account_info().lamports.borrow_mut() += available;

        Ok(())
    }

    /// Reclaim donated funds for a donor after the campaign ended and goal NOT reached.
    /// Closes the deposit PDA (rent returned to donor) and transfers the donated amount from campaign PDA -> donor.
    pub fn reclaim(ctx: Context<ReclaimCtx>, _campaign_name: String) -> Result<()> {
        let clock = Clock::get()?;
        if clock.slot <= ctx.accounts.campaign_pda.end_donate_slot {
            return err!(ErrorCode::CampaignStillActive);
        }

        let rent = Rent::get()?;
        let min_balance = rent.minimum_balance(CAMPAIGN_SIZE);

        let campaign_ai = ctx.accounts.campaign_pda.to_account_info();
        let campaign_balance = **campaign_ai.lamports.borrow();

        // Calculate total donated held by campaign (excluding rent)
        let total_held = campaign_balance.saturating_sub(min_balance);

        if total_held >= ctx.accounts.campaign_pda.goal_in_lamports {
            return err!(ErrorCode::GoalReached);
        }

        let refund_amount = ctx.accounts.deposit_pda.total_donated;
        if refund_amount == 0 {
            return err!(ErrorCode::NothingToReclaim);
        }

        // Ensure campaign has enough to refund (be conservative)
        if campaign_balance < refund_amount + min_balance {
            return err!(ErrorCode::InsufficientFundsInCampaign);
        }

        // Transfer lamports: campaign_pda -> donor
        **campaign_ai.lamports.borrow_mut() = campaign_balance - refund_amount;
        **ctx.accounts.donor.to_account_info().lamports.borrow_mut() += refund_amount;

        // deposit_pda has `close = donor` in accounts struct --- Anchor will close it and send the rent to donor
        Ok(())
    }
}

// -------------------- Accounts & State --------------------

#[derive(Accounts)]
#[instruction(campaign_name: String, end_donate_slot: u64, goal_in_lamports: u64)]
pub struct InitializeCtx<'info> {
    /// Campaign owner (payer & signer)
    #[account(mut)]
    pub campaign_owner: Signer<'info>,

    /// Campaign PDA storing state and holding funds
    /// seeds = [campaign_name.as_ref()]
    #[account(
        init,
        payer = campaign_owner,
        space = CAMPAIGN_SIZE,
        seeds = [campaign_name.as_ref()],
        bump
    )]
    pub campaign_pda: Account<'info, CampaignPDA>,

    pub system_program: Program<'info, System>,
}

#[derive(Accounts)]
#[instruction(_campaign_name: String, donated_lamports: u64)]
pub struct DonateCtx<'info> {
    /// Donor must sign and pay for deposit PDA if created
    #[account(mut)]
    pub donor: Signer<'info>,

    /// Campaign PDA (must exist)
    #[account(mut, seeds = [ _campaign_name.as_ref() ], bump)]
    pub campaign_pda: Account<'info, CampaignPDA>,

    /// Per-donor deposit PDA tracking donor's total contributions
    /// seeds = [b"deposit", campaign_name.as_ref(), donor.key().as_ref()]
    #[account(
        init_if_needed,
        payer = donor,
        space = DEPOSIT_SIZE,
        seeds = [b"deposit", _campaign_name.as_bytes(), donor.key().as_ref()],
        bump
    )]
    pub deposit_pda: Account<'info, DepositPDA>,


    pub system_program: Program<'info, System>,
}

#[derive(Accounts)]
#[instruction(_campaign_name: String)]
pub struct WithdrawCtx<'info> {
    /// Campaign owner must sign
    #[account(mut)]
    pub campaign_owner: Signer<'info>,

    /// Campaign PDA (must be owned by program and reference the owner)
    #[account(mut, seeds = [ _campaign_name.as_ref() ], bump, has_one = campaign_owner)]
    pub campaign_pda: Account<'info, CampaignPDA>,
}

#[derive(Accounts)]
#[instruction(_campaign_name: String)]
pub struct ReclaimCtx<'info> {
    /// Donor must sign
    #[account(mut)]
    pub donor: Signer<'info>,

    /// Campaign PDA
    #[account(mut, seeds = [ _campaign_name.as_ref() ], bump)]
    pub campaign_pda: Account<'info, CampaignPDA>,

    /// Deposit PDA (closed on success, rent transferred to donor)
    #[account(
        mut,
        seeds = [b"deposit", _campaign_name.as_bytes(), donor.key().as_ref()],
        bump,
        close = donor
    )]
    pub deposit_pda: Account<'info, DepositPDA>,

}

// -------------------- State structs --------------------

#[account]
pub struct CampaignPDA {
    pub campaign_name: String,
    pub campaign_owner: Pubkey,
    pub end_donate_slot: u64,
    pub goal_in_lamports: u64,
}

#[account]
pub struct DepositPDA {
    pub total_donated: u64,
}

// -------------------- Errors --------------------

#[error_code]
pub enum ErrorCode {
    #[msg("End slot cannot be before the current slot")] 
    EndSlotInPast,

    #[msg("Campaign name exceeds maximum length")] 
    CampaignNameTooLong,

    #[msg("Goal must be non-zero")]
    GoalMustBeNonZero,

    #[msg("Donation amount must be greater than zero")]
    InvalidDonationAmount,

    #[msg("The campaign donation period has already ended")]
    CampaignEnded,

    #[msg("Campaign is still active; cannot withdraw/reclaim until after end slot")]
    CampaignStillActive,

    #[msg("Goal has not been reached yet")]
    GoalNotReached,

    #[msg("Goal was reached; cannot reclaim funds")]
    GoalReached,

    #[msg("Nothing to reclaim in this deposit account")]
    NothingToReclaim,

    #[msg("Numerical overflow")] 
    NumericalOverflow,

    #[msg("Campaign has insufficient funds")]
    InsufficientFundsInCampaign,
}
