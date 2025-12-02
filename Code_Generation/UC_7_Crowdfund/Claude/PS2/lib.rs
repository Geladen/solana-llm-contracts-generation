use anchor_lang::prelude::*;
use anchor_lang::system_program;

declare_id!("6xdQBYz6mBAFECFMGj8KzxDQUNnqCb8fRnarccA5e4ib");

#[program]
pub mod crowdfund {
    use super::*;

    pub fn initialize(
        ctx: Context<InitializeCtx>,
        campaign_name: String,
        end_donate_slot: u64,
        goal_in_lamports: u64,
    ) -> Result<()> {
        // Validate campaign name length
        require!(
            campaign_name.len() > 0 && campaign_name.len() <= 32,
            CrowdfundingError::InvalidCampaignName
        );

        // Validate end donate slot is in the future
        let current_slot = Clock::get()?.slot;
        require!(
            end_donate_slot > current_slot,
            CrowdfundingError::InvalidEndSlot
        );

        // Validate goal amount is greater than 0
        require!(
            goal_in_lamports > 0,
            CrowdfundingError::InvalidGoalAmount
        );

        let campaign_pda = &mut ctx.accounts.campaign_pda;
        campaign_pda.campaign_name = campaign_name;
        campaign_pda.campaign_owner = ctx.accounts.campaign_owner.key();
        campaign_pda.end_donate_slot = end_donate_slot;
        campaign_pda.goal_in_lamports = goal_in_lamports;

        msg!("Campaign initialized: {}", campaign_pda.campaign_name);
        Ok(())
    }

    pub fn donate(
        ctx: Context<DonateCtx>,
        _campaign_name: String,
        donated_lamports: u64,
    ) -> Result<()> {
        // Validate donation amount
        require!(
            donated_lamports > 0,
            CrowdfundingError::InvalidDonationAmount
        );

        let campaign_pda = &ctx.accounts.campaign_pda;
        
        // Check if donation period is still active
        let current_slot = Clock::get()?.slot;
        require!(
            current_slot <= campaign_pda.end_donate_slot,
            CrowdfundingError::DonationPeriodEnded
        );

        // Transfer lamports from donor to campaign PDA
        let transfer_instruction = system_program::Transfer {
            from: ctx.accounts.donor.to_account_info(),
            to: ctx.accounts.campaign_pda.to_account_info(),
        };

        let cpi_ctx = CpiContext::new(
            ctx.accounts.system_program.to_account_info(),
            transfer_instruction,
        );

        system_program::transfer(cpi_ctx, donated_lamports)?;

        // Update or initialize deposit PDA
        let deposit_pda = &mut ctx.accounts.deposit_pda;
        deposit_pda.total_donated = deposit_pda.total_donated.checked_add(donated_lamports)
            .ok_or(CrowdfundingError::MathOverflow)?;

        msg!("Donation of {} lamports received from {}", donated_lamports, ctx.accounts.donor.key());
        Ok(())
    }

    pub fn withdraw(
        ctx: Context<WithdrawCtx>,
        _campaign_name: String,
    ) -> Result<()> {
        let campaign_pda = &mut ctx.accounts.campaign_pda;
        let campaign_owner = &ctx.accounts.campaign_owner;

        // Check if the goal has been reached
        let campaign_balance = campaign_pda.to_account_info().lamports();
        require!(
            campaign_balance >= campaign_pda.goal_in_lamports,
            CrowdfundingError::GoalNotReached
        );

        // Calculate rent-exempt minimum for the campaign PDA
        let rent = Rent::get()?;
        let campaign_account_info = campaign_pda.to_account_info();
        let rent_exempt_minimum = rent.minimum_balance(campaign_account_info.data_len());

        // Calculate withdrawable amount (total balance minus rent-exempt minimum)
        let withdrawable_amount = campaign_balance.saturating_sub(rent_exempt_minimum);
        
        require!(
            withdrawable_amount > 0,
            CrowdfundingError::InsufficientFunds
        );

        // Transfer withdrawable lamports from campaign PDA to campaign owner
        **campaign_pda.to_account_info().try_borrow_mut_lamports()? -= withdrawable_amount;
        **campaign_owner.to_account_info().try_borrow_mut_lamports()? += withdrawable_amount;

        msg!("Campaign owner withdrew {} lamports", withdrawable_amount);
        Ok(())
    }

    pub fn reclaim(
        ctx: Context<ReclaimCtx>,
        _campaign_name: String,
    ) -> Result<()> {
        let campaign_pda = &ctx.accounts.campaign_pda;
        let deposit_pda = &ctx.accounts.deposit_pda;
        let donor = &ctx.accounts.donor;

        // Check if the goal was NOT reached
        let campaign_balance = campaign_pda.to_account_info().lamports();
        
        // Calculate rent-exempt minimum for the campaign PDA
        let rent = Rent::get()?;
        let campaign_account_info = campaign_pda.to_account_info();
        let rent_exempt_minimum = rent.minimum_balance(campaign_account_info.data_len());
        
        // Available balance for reclaiming (excluding rent-exempt minimum)
        let available_balance = campaign_balance.saturating_sub(rent_exempt_minimum);
        
        require!(
            available_balance < campaign_pda.goal_in_lamports,
            CrowdfundingError::GoalWasReached
        );

        // Check if donation period has ended
        let current_slot = Clock::get()?.slot;
        require!(
            current_slot > campaign_pda.end_donate_slot,
            CrowdfundingError::DonationPeriodActive
        );

        let donated_amount = deposit_pda.total_donated;
        require!(
            donated_amount > 0,
            CrowdfundingError::NoDonationToReclaim
        );

        // Ensure we don't withdraw below rent-exempt minimum
        require!(
            available_balance >= donated_amount,
            CrowdfundingError::InsufficientFundsForReclaim
        );

        // Transfer donated amount back to donor from campaign PDA
        **campaign_pda.to_account_info().try_borrow_mut_lamports()? -= donated_amount;
        **donor.to_account_info().try_borrow_mut_lamports()? += donated_amount;

        // Close deposit PDA and transfer rent back to donor
        let deposit_lamports = deposit_pda.to_account_info().lamports();
        **deposit_pda.to_account_info().try_borrow_mut_lamports()? = 0;
        **donor.to_account_info().try_borrow_mut_lamports()? += deposit_lamports;

        msg!("Donor reclaimed {} lamports", donated_amount);
        Ok(())
    }
}

// Context Structs
#[derive(Accounts)]
#[instruction(campaign_name: String)]
pub struct InitializeCtx<'info> {
    #[account(mut)]
    pub campaign_owner: Signer<'info>,

    #[account(
        init,
        payer = campaign_owner,
        space = 8 + CampaignPDA::INIT_SPACE,
        seeds = [campaign_name.as_ref()],
        bump
    )]
    pub campaign_pda: Account<'info, CampaignPDA>,

    pub system_program: Program<'info, System>,
}

#[derive(Accounts)]
#[instruction(campaign_name: String)]
pub struct DonateCtx<'info> {
    #[account(mut)]
    pub donor: Signer<'info>,

    #[account(
        mut,
        seeds = [campaign_name.as_ref()],
        bump
    )]
    pub campaign_pda: Account<'info, CampaignPDA>,

    #[account(
        init_if_needed,
        payer = donor,
        space = 8 + DepositPDA::INIT_SPACE,
        seeds = [b"deposit".as_ref(), campaign_name.as_ref(), donor.key().as_ref()],
        bump
    )]
    pub deposit_pda: Account<'info, DepositPDA>,

    pub system_program: Program<'info, System>,
}

#[derive(Accounts)]
#[instruction(campaign_name: String)]
pub struct WithdrawCtx<'info> {
    #[account(
        mut,
        constraint = campaign_owner.key() == campaign_pda.campaign_owner @ CrowdfundingError::UnauthorizedWithdraw
    )]
    pub campaign_owner: Signer<'info>,

    #[account(
        mut,
        seeds = [campaign_name.as_ref()],
        bump
    )]
    pub campaign_pda: Account<'info, CampaignPDA>,
}

#[derive(Accounts)]
#[instruction(campaign_name: String)]
pub struct ReclaimCtx<'info> {
    #[account(mut)]
    pub donor: Signer<'info>,

    #[account(
        mut,
        seeds = [campaign_name.as_ref()],
        bump
    )]
    pub campaign_pda: Account<'info, CampaignPDA>,

    #[account(
        mut,
        seeds = [b"deposit".as_ref(), campaign_name.as_ref(), donor.key().as_ref()],
        bump,
        close = donor
    )]
    pub deposit_pda: Account<'info, DepositPDA>,
}

// Account Structs
#[account]
#[derive(InitSpace)]
pub struct CampaignPDA {
    #[max_len(32)]
    pub campaign_name: String,
    pub campaign_owner: Pubkey,
    pub end_donate_slot: u64,
    pub goal_in_lamports: u64,
}

#[account]
#[derive(InitSpace)]
pub struct DepositPDA {
    pub total_donated: u64,
}

// Error Codes
#[error_code]
pub enum CrowdfundingError {
    #[msg("Invalid campaign name. Must be 1-32 characters.")]
    InvalidCampaignName,

    #[msg("End donate slot must be in the future.")]
    InvalidEndSlot,

    #[msg("Goal amount must be greater than 0.")]
    InvalidGoalAmount,

    #[msg("Donation amount must be greater than 0.")]
    InvalidDonationAmount,

    #[msg("Donation period has ended.")]
    DonationPeriodEnded,

    #[msg("Goal has not been reached.")]
    GoalNotReached,

    #[msg("Goal was already reached.")]
    GoalWasReached,

    #[msg("Donation period is still active.")]
    DonationPeriodActive,

    #[msg("No donation to reclaim.")]
    NoDonationToReclaim,

    #[msg("Unauthorized withdrawal attempt.")]
    UnauthorizedWithdraw,

    #[msg("Math operation overflow.")]
    MathOverflow,

    #[msg("Insufficient funds for withdrawal.")]
    InsufficientFunds,

    #[msg("Insufficient funds available for reclaim.")]
    InsufficientFundsForReclaim,
}