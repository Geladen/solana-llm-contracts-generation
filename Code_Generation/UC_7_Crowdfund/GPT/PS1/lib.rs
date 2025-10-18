use anchor_lang::prelude::*;
use anchor_lang::solana_program::{clock::Clock, sysvar};
use anchor_lang::solana_program::rent::Rent;

declare_id!("3W1YqaZSvYZDwRKgr8HGGSBJsfxPLaA4615KSaGZi66u");

#[program]
pub mod crowdfund {
    use super::*;

    pub fn initialize(
        ctx: Context<InitializeCtx>,
        campaign_name: String,
        end_donate_slot: u64,
        goal_in_lamports: u64,
    ) -> Result<()> {
        let clock = Clock::get()?;
        require!(end_donate_slot >= clock.slot, ErrorCode::EndSlotBeforeCurrent);
        require!(!campaign_name.is_empty(), ErrorCode::InvalidCampaignNameLength);
        require!(goal_in_lamports > 0, ErrorCode::InvalidGoalAmount);

        let campaign = &mut ctx.accounts.campaign_pda;
        campaign.campaign_name = campaign_name;
        campaign.campaign_owner = ctx.accounts.campaign_owner.key();
        campaign.end_donate_slot = end_donate_slot;
        campaign.goal_in_lamports = goal_in_lamports;

        Ok(())
    }

    pub fn donate(
        ctx: Context<DonateCtx>,
        _campaign_name: String, // provided to derive PDAs, not used directly in logic
        donated_lamports: u64,
    ) -> Result<()> {
        // signer is donor (enforced by context)
        if donated_lamports == 0 {
            return err!(ErrorCode::InvalidDonationAmount);
        }

        let clock = Clock::get()?;
        let campaign = &ctx.accounts.campaign_pda;
        if clock.slot > campaign.end_donate_slot {
            return err!(ErrorCode::DonationPeriodEnded);
        }

        // Transfer lamports from donor to campaign PDA using CPI to system_program (donor is signer)
        // Use Anchor's system_program::transfer CPI
        let cpi_accounts = anchor_lang::system_program::Transfer {
            from: ctx.accounts.donor.to_account_info(),
            to: ctx.accounts.campaign_pda.to_account_info(),
        };
        let cpi_ctx = CpiContext::new(ctx.accounts.system_program.to_account_info(), cpi_accounts);
        anchor_lang::system_program::transfer(cpi_ctx, donated_lamports)?;

        // Update or initialize deposit PDA
        let deposit = &mut ctx.accounts.deposit_pda;
        deposit.total_donated = deposit
            .total_donated
            .checked_add(donated_lamports)
            .ok_or(ErrorCode::Overflow)?;

        Ok(())
    }

    pub fn withdraw(ctx: Context<WithdrawCtx>, _campaign_name: String) -> Result<()> {
        let campaign = &ctx.accounts.campaign_pda;

        let rent = Rent::get()?;
        let rent_min = rent.minimum_balance(CampaignPDA::space());
        let campaign_total = ctx.accounts.campaign_pda.to_account_info().lamports();

        // Check only the DONATED portion against the goal
        let donated = campaign_total.saturating_sub(rent_min);
        if donated < campaign.goal_in_lamports {
            return err!(ErrorCode::GoalNotReached);
        }

        // Anchor will close the campaign_pda and transfer ALL lamports (donations + rent) to owner
        Ok(())
    }

    pub fn reclaim(ctx: Context<ReclaimCtx>, _campaign_name: String) -> Result<()> {
        let clock = Clock::get()?;
        let campaign = &ctx.accounts.campaign_pda;

        let rent = Rent::get()?;
        let rent_min = rent.minimum_balance(CampaignPDA::space());
        let campaign_total = ctx.accounts.campaign_pda.to_account_info().lamports();

        // Goal check: only donations matter
        let donated = campaign_total.saturating_sub(rent_min);
        if donated >= campaign.goal_in_lamports {
            return err!(ErrorCode::GoalReachedCannotReclaim);
        }

        if clock.slot <= campaign.end_donate_slot {
            return err!(ErrorCode::ReclaimBeforeCampaignEnd);
        }

        let deposit = &mut ctx.accounts.deposit_pda;
        let donor_contribution = deposit.total_donated;
        if donor_contribution == 0 {
            return err!(ErrorCode::NoDepositToReclaim);
        }

        let campaign_acct = &mut ctx.accounts.campaign_pda.to_account_info();
        let donor_acct = &mut ctx.accounts.donor.to_account_info();

        if campaign_acct.lamports() < donor_contribution {
            return err!(ErrorCode::InsufficientCampaignFunds);
        }

        **campaign_acct.try_borrow_mut_lamports()? -= donor_contribution;
        **donor_acct.try_borrow_mut_lamports()? += donor_contribution;

        deposit.total_donated = 0;

        // Deposit PDA will close to donor (returns rent for deposit)
        Ok(())
    }
}

/// Accounts and context definitions

#[derive(Accounts)]
#[instruction(campaign_name: String)]
pub struct InitializeCtx<'info> {
    #[account(mut)]
    pub campaign_owner: Signer<'info>,

    // ✅ Anchor will create and fund the PDA with rent-exempt lamports
    #[account(
        init,
        payer = campaign_owner,
        space = CampaignPDA::space(),
        seeds = [campaign_name.as_ref()],
        bump
    )]
    pub campaign_pda: Account<'info, CampaignPDA>,

    pub system_program: Program<'info, System>,
}

#[derive(Accounts)]
#[instruction(_campaign_name: String)]
pub struct DonateCtx<'info> {
    /// Donor who signs and pays for deposit account if init
    #[account(mut)]
    pub donor: Signer<'info>,

    /// The campaign PDA: must already exist
    #[account(
        mut,
        seeds = [ _campaign_name.as_ref() ],
        bump
    )]
    pub campaign_pda: Account<'info, CampaignPDA>,

    /// Deposit PDA for donor; init_if_needed so donors can donate multiple times
    /// seeds = ["deposit".as_ref(), campaign_name.as_ref(), donor.key().as_ref()]
    #[account(
        init_if_needed,
        payer = donor,
        space = DepositPDA::space(),
        seeds = [ b"deposit".as_ref(), _campaign_name.as_ref(), donor.key().as_ref() ],
        bump
    )]
    pub deposit_pda: Account<'info, DepositPDA>,

    pub system_program: Program<'info, System>,
}

#[derive(Accounts)]
#[instruction(_campaign_name: String)]
pub struct WithdrawCtx<'info> {
    /// Campaign owner (must sign)
    #[account(mut)]
    pub campaign_owner: Signer<'info>,

    /// Campaign PDA (will be closed and lamports returned to campaign_owner)
    /// seeds = [campaign_name.as_ref()]
    #[account(
        mut,
        seeds = [ _campaign_name.as_ref() ],
        bump,
        has_one = campaign_owner,
        // close to campaign_owner — Anchor will transfer all lamports and close the account if checks pass
        close = campaign_owner
    )]
    pub campaign_pda: Account<'info, CampaignPDA>,
}

#[derive(Accounts)]
#[instruction(_campaign_name: String)]
pub struct ReclaimCtx<'info> {
    /// Donor (signer) who will receive lamports back and will receive the deposit PDA rent on close.
    #[account(mut)]
    pub donor: Signer<'info>,

    /// Campaign PDA (holds funds)
    #[account(
        mut,
        seeds = [ _campaign_name.as_ref() ],
        bump,
    )]
    pub campaign_pda: Account<'info, CampaignPDA>,

    /// Deposit PDA for donor: we will close this to the donor (returns rent).
    /// seeds = ["deposit".as_ref(), campaign_name.as_ref(), donor.key().as_ref()]
    #[account(
        mut,
        seeds = [ b"deposit".as_ref(), _campaign_name.as_ref(), donor.key().as_ref() ],
        bump,
        close = donor
    )]
    pub deposit_pda: Account<'info, DepositPDA>,
}

/// Persistent account structs

#[account]
pub struct CampaignPDA {
    pub campaign_name: String,
    pub campaign_owner: Pubkey,
    pub end_donate_slot: u64,
    pub goal_in_lamports: u64,
}

impl CampaignPDA {
    // conservative maximums:
    pub const MAX_NAME_LEN: usize = 64;
    pub fn space() -> usize {
        // Anchor: 8 discriminator + 4 string len + MAX_NAME_LEN + 32 pubkey + 8 + 8
        8 + 4 + Self::MAX_NAME_LEN + 32 + 8 + 8
    }
}

#[account]
pub struct DepositPDA {
    pub total_donated: u64,
}

impl DepositPDA {
    pub fn space() -> usize {
        // 8 discriminator + 8 u64
        8 + 8
    }
}

/// Errors

#[error_code]
pub enum ErrorCode {
    #[msg("The provided end slot is before the current slot")]
    EndSlotBeforeCurrent,
    #[msg("Campaign name must be non-empty and not exceed max length")]
    InvalidCampaignNameLength,
    #[msg("Goal must be > 0")]
    InvalidGoalAmount,
    #[msg("Donation amount must be > 0")]
    InvalidDonationAmount,
    #[msg("The donation period has already ended")]
    DonationPeriodEnded,
    #[msg("Math overflow")]
    Overflow,
    #[msg("Campaign goal not reached yet")]
    GoalNotReached,
    #[msg("Campaign goal has been reached; cannot reclaim")]
    GoalReachedCannotReclaim,
    #[msg("Cannot reclaim before campaign end")]
    ReclaimBeforeCampaignEnd,
    #[msg("Donor has no deposit to reclaim")]
    NoDepositToReclaim,
    #[msg("Campaign has insufficient funds to fulfill reclaim")]
    InsufficientCampaignFunds,
    #[msg("Invalid PDA derived for the campaign")]
    InvalidPDA,
}
