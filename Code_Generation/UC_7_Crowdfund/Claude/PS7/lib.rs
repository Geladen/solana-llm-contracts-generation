use anchor_lang::prelude::*;

declare_id!("AYzgbm9rHtWhoWe7TfZWx953cSFAQNmE2i4FBU6S8gng");

#[program]
pub mod crowdfund {
    use super::*;

    /// Initialize a new crowdfunding campaign
    /// Only callable by the campaign owner
    pub fn initialize(
        ctx: Context<InitializeCtx>, 
        campaign_name: String, 
        end_donate_slot: u64, 
        goal_in_lamports: u64
    ) -> Result<()> {
        let current_slot = Clock::get()?.slot;
        
        // Validate that end_donate_slot is in the future
        require!(
            end_donate_slot > current_slot, 
            CrowdfundingError::InvalidEndSlot
        );

        // Validate goal amount is positive
        require!(
            goal_in_lamports > 0, 
            CrowdfundingError::InvalidGoalAmount
        );

        // Initialize campaign PDA
        let campaign_pda = &mut ctx.accounts.campaign_pda;
        campaign_pda.campaign_name = campaign_name;
        campaign_pda.campaign_owner = ctx.accounts.campaign_owner.key();
        campaign_pda.end_donate_slot = end_donate_slot;
        campaign_pda.goal_in_lamports = goal_in_lamports;

        Ok(())
    }

    /// Donate to a campaign
    /// Callable by any donor
    pub fn donate(
        ctx: Context<DonateCtx>, 
        _campaign_name: String, 
        donated_lamports: u64
    ) -> Result<()> {
        let current_slot = Clock::get()?.slot;
        
        // Check if donation period is still active
        require!(
            current_slot <= ctx.accounts.campaign_pda.end_donate_slot,
            CrowdfundingError::DonationPeriodEnded
        );

        // Validate donation amount is positive
        require!(
            donated_lamports > 0,
            CrowdfundingError::InvalidDonationAmount
        );

        // Transfer lamports from donor to campaign PDA
        let transfer_instruction = anchor_lang::system_program::Transfer {
            from: ctx.accounts.donor.to_account_info(),
            to: ctx.accounts.campaign_pda.to_account_info(),
        };
        let transfer_ctx = CpiContext::new(
            ctx.accounts.system_program.to_account_info(),
            transfer_instruction,
        );
        anchor_lang::system_program::transfer(transfer_ctx, donated_lamports)?;

        // Update deposit PDA to track donor's contribution
        let deposit_pda = &mut ctx.accounts.deposit_pda;
        deposit_pda.total_donated = deposit_pda.total_donated.checked_add(donated_lamports)
            .ok_or(CrowdfundingError::MathOverflow)?;

        Ok(())
    }

    /// Withdraw funds from campaign (owner only, goal must be reached)
    pub fn withdraw(
        ctx: Context<WithdrawCtx>, 
        _campaign_name: String
    ) -> Result<()> {
        let campaign_pda = &ctx.accounts.campaign_pda;
        
        // Calculate rent exemption minimum for campaign PDA
        let rent = Rent::get()?;
        let campaign_pda_info = ctx.accounts.campaign_pda.to_account_info();
        let rent_exempt_minimum = rent.minimum_balance(campaign_pda_info.data_len());
        
        // Get current campaign balance
        let campaign_balance = ctx.accounts.campaign_pda.to_account_info().lamports();
        
        // Check if goal was reached (excluding rent exemption from goal calculation)
        let available_balance = campaign_balance.saturating_sub(rent_exempt_minimum);
        require!(
            available_balance >= campaign_pda.goal_in_lamports,
            CrowdfundingError::GoalNotReached
        );

        // Calculate withdrawable amount (total balance minus rent exemption)
        let withdrawable_amount = available_balance;
        
        require!(
            withdrawable_amount > 0,
            CrowdfundingError::NoFundsToWithdraw
        );

        // Transfer withdrawable funds from campaign PDA to owner
        **ctx.accounts.campaign_pda.to_account_info().try_borrow_mut_lamports()? -= withdrawable_amount;
        **ctx.accounts.campaign_owner.to_account_info().try_borrow_mut_lamports()? += withdrawable_amount;

        Ok(())
    }

    /// Reclaim donated funds (donor only, goal must not be reached)
    pub fn reclaim(
        ctx: Context<ReclaimCtx>, 
        _campaign_name: String
    ) -> Result<()> {
        let campaign_pda = &ctx.accounts.campaign_pda;
        let deposit_pda = &ctx.accounts.deposit_pda;
        
        // Check if goal was NOT reached
        let campaign_balance = ctx.accounts.campaign_pda.to_account_info().lamports();
        require!(
            campaign_balance < campaign_pda.goal_in_lamports,
            CrowdfundingError::GoalAlreadyReached
        );

        // Get the amount to reclaim
        let reclaim_amount = deposit_pda.total_donated;
        
        // Ensure there's something to reclaim
        require!(
            reclaim_amount > 0,
            CrowdfundingError::NothingToReclaim
        );

        // Calculate rent exemption minimum for campaign PDA
        let rent = Rent::get()?;
        let campaign_pda_info = ctx.accounts.campaign_pda.to_account_info();
        let rent_exempt_minimum = rent.minimum_balance(campaign_pda_info.data_len());

        // Ensure we don't drain the campaign PDA below rent exemption
        require!(
            campaign_balance.saturating_sub(reclaim_amount) >= rent_exempt_minimum,
            CrowdfundingError::InsufficientFundsForReclaim
        );

        // Transfer donated amount back to donor from campaign PDA
        **ctx.accounts.campaign_pda.to_account_info().try_borrow_mut_lamports()? -= reclaim_amount;
        **ctx.accounts.donor.to_account_info().try_borrow_mut_lamports()? += reclaim_amount;

        // Close deposit PDA and return rent to donor
        let deposit_lamports = ctx.accounts.deposit_pda.to_account_info().lamports();
        **ctx.accounts.deposit_pda.to_account_info().try_borrow_mut_lamports()? = 0;
        **ctx.accounts.donor.to_account_info().try_borrow_mut_lamports()? += deposit_lamports;

        Ok(())
    }
}

// Context structs for each instruction
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
        seeds = ["deposit".as_ref(), campaign_name.as_ref(), donor.key().as_ref()],
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
        constraint = campaign_owner.key() == campaign_pda.campaign_owner @ CrowdfundingError::UnauthorizedAccess
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
        close = donor,
        seeds = ["deposit".as_ref(), campaign_name.as_ref(), donor.key().as_ref()],
        bump
    )]
    pub deposit_pda: Account<'info, DepositPDA>,
}

// Account state structures
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

// Custom error types
#[error_code]
pub enum CrowdfundingError {
    #[msg("End donation slot must be in the future")]
    InvalidEndSlot,
    
    #[msg("Goal amount must be greater than 0")]
    InvalidGoalAmount,
    
    #[msg("Donation period has ended")]
    DonationPeriodEnded,
    
    #[msg("Donation amount must be greater than 0")]
    InvalidDonationAmount,
    
    #[msg("Goal has not been reached, cannot withdraw")]
    GoalNotReached,
    
    #[msg("Goal has already been reached, cannot reclaim")]
    GoalAlreadyReached,
    
    #[msg("Nothing to reclaim")]
    NothingToReclaim,
    
    #[msg("Insufficient funds in campaign for reclaim")]
    InsufficientFundsForReclaim,
    
    #[msg("No funds available to withdraw")]
    NoFundsToWithdraw,
    
    #[msg("Unauthorized access")]
    UnauthorizedAccess,
    
    #[msg("Mathematical overflow")]
    MathOverflow,
}