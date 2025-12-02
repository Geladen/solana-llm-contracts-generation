use anchor_lang::prelude::*;
use anchor_lang::system_program;

declare_id!("4uzJfGnrZDJePha6hdcX6MMkbFqufTnWAue1ac9DB4aW");

#[program]
pub mod crowdfund {
    use super::*;

    pub fn initialize(
        ctx: Context<Initialize>,
        campaign_name: String,
        end_donate_slot: u64,
        goal_in_lamports: u64,
    ) -> Result<()> {
        // Validate campaign name length
        if campaign_name.is_empty() || campaign_name.len() > 50 {
            return Err(ErrorCode::InvalidCampaignNameLength.into());
        }

        // Validate end slot is in the future
        let current_slot = Clock::get()?.slot;
        if end_donate_slot <= current_slot {
            return Err(ErrorCode::InvalidEndSlot.into());
        }

        // Validate goal amount
        if goal_in_lamports == 0 {
            return Err(ErrorCode::InvalidGoalAmount.into());
        }

        let campaign = &mut ctx.accounts.campaign_pda;
        campaign.campaign_name = campaign_name;
        campaign.campaign_owner = ctx.accounts.campaign_owner.key();
        campaign.end_donate_slot = end_donate_slot;
        campaign.goal_in_lamports = goal_in_lamports;
        campaign.total_donated = 0;
        campaign.bump = ctx.bumps.campaign_pda;

        Ok(())
    }

    pub fn donate(
        ctx: Context<Donate>,
        campaign_name: String,
        donated_lamports: u64,
    ) -> Result<()> {
        // Validate campaign name matches
        if campaign_name != ctx.accounts.campaign_pda.campaign_name {
            return Err(ErrorCode::CampaignNameMismatch.into());
        }

        // Validate donation amount
        if donated_lamports == 0 {
            return Err(ErrorCode::InvalidDonationAmount.into());
        }

        // Check if campaign is still active
        let current_slot = Clock::get()?.slot;
        if current_slot > ctx.accounts.campaign_pda.end_donate_slot {
            return Err(ErrorCode::CampaignEnded.into());
        }

        // Transfer lamports from donor to campaign PDA using CPI
        let cpi_context = CpiContext::new(
            ctx.accounts.system_program.to_account_info(),
            system_program::Transfer {
                from: ctx.accounts.donor.to_account_info(),
                to: ctx.accounts.campaign_pda.to_account_info(),
            },
        );
        system_program::transfer(cpi_context, donated_lamports)?;

        // Update campaign total
        ctx.accounts.campaign_pda.total_donated = ctx.accounts.campaign_pda.total_donated
            .checked_add(donated_lamports)
            .ok_or(ErrorCode::NumericalOverflow)?;

        // Update donor's deposit record
        ctx.accounts.deposit_pda.total_donated = ctx.accounts.deposit_pda.total_donated
            .checked_add(donated_lamports)
            .ok_or(ErrorCode::NumericalOverflow)?;

        Ok(())
    }

    pub fn withdraw(
        ctx: Context<Withdraw>,
        campaign_name: String,
    ) -> Result<()> {
        // Validate campaign name matches
        if campaign_name != ctx.accounts.campaign_pda.campaign_name {
            return Err(ErrorCode::CampaignNameMismatch.into());
        }

        // Validate campaign owner
        if ctx.accounts.campaign_owner.key() != ctx.accounts.campaign_pda.campaign_owner {
            return Err(ErrorCode::Unauthorized.into());
        }

        // Check if campaign has ended
        let current_slot = Clock::get()?.slot;
        if current_slot <= ctx.accounts.campaign_pda.end_donate_slot {
            return Err(ErrorCode::CampaignNotEnded.into());
        }

        // Check if goal was reached
        if ctx.accounts.campaign_pda.total_donated < ctx.accounts.campaign_pda.goal_in_lamports {
            return Err(ErrorCode::GoalNotReached.into());
        }

        // Get the balance to transfer
        let campaign_balance = ctx.accounts.campaign_pda.to_account_info().lamports();
        
        // Perform the transfer safely
        **ctx.accounts.campaign_pda.to_account_info().try_borrow_mut_lamports()? = ctx.accounts.campaign_pda.to_account_info().lamports().checked_sub(campaign_balance).unwrap();
        **ctx.accounts.campaign_owner.try_borrow_mut_lamports()? = ctx.accounts.campaign_owner.lamports().checked_add(campaign_balance).unwrap();

        Ok(())
    }

    pub fn reclaim(
        ctx: Context<Reclaim>,
        campaign_name: String,
    ) -> Result<()> {
        // Validate campaign name matches
        if campaign_name != ctx.accounts.campaign_pda.campaign_name {
            return Err(ErrorCode::CampaignNameMismatch.into());
        }

        // Check if campaign has ended
        let current_slot = Clock::get()?.slot;
        if current_slot <= ctx.accounts.campaign_pda.end_donate_slot {
            return Err(ErrorCode::CampaignNotEnded.into());
        }

        // Check if goal was NOT reached (only allow reclaim if campaign failed)
        if ctx.accounts.campaign_pda.total_donated >= ctx.accounts.campaign_pda.goal_in_lamports {
            return Err(ErrorCode::GoalReached.into());
        }

        // Check if donor has actually donated
        if ctx.accounts.deposit_pda.total_donated == 0 {
            return Err(ErrorCode::NoDonationFound.into());
        }

        let refund_amount = ctx.accounts.deposit_pda.total_donated;

        // Check if campaign has enough funds
        let campaign_balance = ctx.accounts.campaign_pda.to_account_info().lamports();
        if campaign_balance < refund_amount {
            return Err(ErrorCode::InsufficientCampaignFunds.into());
        }

        // Transfer refund from campaign PDA to donor
        **ctx.accounts.campaign_pda.to_account_info().try_borrow_mut_lamports()? = campaign_balance.checked_sub(refund_amount).unwrap();
        **ctx.accounts.donor.try_borrow_mut_lamports()? = ctx.accounts.donor.lamports().checked_add(refund_amount).unwrap();

        // Close deposit PDA and return rent
        let deposit_balance = ctx.accounts.deposit_pda.to_account_info().lamports();
        **ctx.accounts.deposit_pda.to_account_info().try_borrow_mut_lamports()? = 0;
        **ctx.accounts.donor.try_borrow_mut_lamports()? = ctx.accounts.donor.lamports().checked_add(deposit_balance).unwrap();

        Ok(())
    }
}

#[derive(Accounts)]
#[instruction(campaign_name: String)]
pub struct Initialize<'info> {
    #[account(mut)]
    pub campaign_owner: Signer<'info>,

    #[account(
        init,
        payer = campaign_owner,
        space = 8 + CampaignPDA::INIT_SPACE,
        seeds = [campaign_name.as_bytes()],
        bump
    )]
    pub campaign_pda: Account<'info, CampaignPDA>,

    pub system_program: Program<'info, System>,
}

#[derive(Accounts)]
#[instruction(campaign_name: String)]
pub struct Donate<'info> {
    #[account(mut)]
    pub donor: Signer<'info>,

    #[account(
        mut,
        seeds = [campaign_name.as_bytes()],
        bump = campaign_pda.bump
    )]
    pub campaign_pda: Account<'info, CampaignPDA>,

    #[account(
        init_if_needed,
        payer = donor,
        space = 8 + DepositPDA::INIT_SPACE,
        seeds = [b"deposit", campaign_name.as_bytes(), donor.key().as_ref()],
        bump
    )]
    pub deposit_pda: Account<'info, DepositPDA>,

    pub system_program: Program<'info, System>,
}

#[derive(Accounts)]
#[instruction(campaign_name: String)]
pub struct Withdraw<'info> {
    #[account(mut)]
    pub campaign_owner: Signer<'info>,

    #[account(
        mut,
        seeds = [campaign_name.as_bytes()],
        bump = campaign_pda.bump
    )]
    pub campaign_pda: Account<'info, CampaignPDA>,
}

#[derive(Accounts)]
#[instruction(campaign_name: String)]
pub struct Reclaim<'info> {
    #[account(mut)]
    pub donor: Signer<'info>,

    #[account(
        mut,
        seeds = [campaign_name.as_bytes()],
        bump = campaign_pda.bump
    )]
    pub campaign_pda: Account<'info, CampaignPDA>,

    #[account(
        mut,
        seeds = [b"deposit", campaign_name.as_bytes(), donor.key().as_ref()],
        bump
    )]
    pub deposit_pda: Account<'info, DepositPDA>,
}

#[account]
#[derive(InitSpace)]
pub struct CampaignPDA {
    #[max_len(50)]
    pub campaign_name: String,
    pub campaign_owner: Pubkey,
    pub end_donate_slot: u64,
    pub goal_in_lamports: u64,
    pub total_donated: u64,
    pub bump: u8,
}

#[account]
#[derive(InitSpace)]
pub struct DepositPDA {
    pub total_donated: u64,
}

#[error_code]
pub enum ErrorCode {
    #[msg("Invalid campaign name length")]
    InvalidCampaignNameLength,
    #[msg("End slot must be in the future")]
    InvalidEndSlot,
    #[msg("Invalid goal amount")]
    InvalidGoalAmount,
    #[msg("Invalid donation amount")]
    InvalidDonationAmount,
    #[msg("Campaign has ended")]
    CampaignEnded,
    #[msg("Campaign has not ended yet")]
    CampaignNotEnded,
    #[msg("Unauthorized access")]
    Unauthorized,
    #[msg("Campaign goal not reached")]
    GoalNotReached,
    #[msg("Campaign goal was reached, funds cannot be reclaimed")]
    GoalReached,
    #[msg("No donation found for this donor")]
    NoDonationFound,
    #[msg("Insufficient funds in campaign account")]
    InsufficientCampaignFunds,
    #[msg("Campaign name does not match")]
    CampaignNameMismatch,
    #[msg("Numerical overflow occurred")]
    NumericalOverflow,
}