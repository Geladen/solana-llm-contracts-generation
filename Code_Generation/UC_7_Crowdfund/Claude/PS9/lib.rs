use anchor_lang::prelude::*;

declare_id!("7iKLeerXAdCPmTWD1Yz5EEVUivxBwhpAXxhvKNPZzr3z");

#[program]
pub mod crowdfund {
    use super::*;

    pub fn initialize(
        ctx: Context<InitializeCtx>,
        campaign_name: String,
        end_donate_slot: u64,
        goal_in_lamports: u64,
    ) -> Result<()> {
        let current_slot = Clock::get()?.slot;
        
        // Validate end slot is in the future
        require!(
            end_donate_slot > current_slot,
            CrowdfundingError::InvalidEndSlot
        );

        // Validate goal is reasonable (greater than 0)
        require!(
            goal_in_lamports > 0,
            CrowdfundingError::InvalidGoal
        );

        let campaign_pda = &mut ctx.accounts.campaign_pda;
        campaign_pda.campaign_name = campaign_name;
        campaign_pda.campaign_owner = ctx.accounts.campaign_owner.key();
        campaign_pda.end_donate_slot = end_donate_slot;
        campaign_pda.goal_in_lamports = goal_in_lamports;

        Ok(())
    }

    pub fn donate(
        ctx: Context<DonateCtx>,
        _campaign_name: String,
        donated_lamports: u64,
    ) -> Result<()> {
        let current_slot = Clock::get()?.slot;
        let campaign_pda = &ctx.accounts.campaign_pda;

        // Check if donation period has ended
        require!(
            current_slot <= campaign_pda.end_donate_slot,
            CrowdfundingError::DonationPeriodEnded
        );

        // Validate donation amount
        require!(
            donated_lamports > 0,
            CrowdfundingError::InvalidDonationAmount
        );

        // Transfer lamports from donor to campaign PDA
        let transfer_ix = anchor_lang::system_program::Transfer {
            from: ctx.accounts.donor.to_account_info(),
            to: ctx.accounts.campaign_pda.to_account_info(),
        };
        
        let cpi_ctx = CpiContext::new(
            ctx.accounts.system_program.to_account_info(),
            transfer_ix,
        );
        
        anchor_lang::system_program::transfer(cpi_ctx, donated_lamports)?;

        // Update or initialize deposit PDA
        let deposit_pda = &mut ctx.accounts.deposit_pda;
        deposit_pda.total_donated = deposit_pda
            .total_donated
            .checked_add(donated_lamports)
            .ok_or(CrowdfundingError::MathOverflow)?;

        Ok(())
    }

    pub fn withdraw(ctx: Context<WithdrawCtx>, _campaign_name: String) -> Result<()> {
        let current_slot = Clock::get()?.slot;
        let campaign_pda = &mut ctx.accounts.campaign_pda;

        // Check if donation period has ended
        require!(
            current_slot > campaign_pda.end_donate_slot,
            CrowdfundingError::DonationPeriodNotEnded
        );

        // Check if goal was reached
        let campaign_balance = campaign_pda.to_account_info().lamports();
        require!(
            campaign_balance >= campaign_pda.goal_in_lamports,
            CrowdfundingError::GoalNotReached
        );

        // Transfer all funds from campaign PDA to campaign owner
        **campaign_pda.to_account_info().try_borrow_mut_lamports()? = 0;
        **ctx.accounts.campaign_owner.try_borrow_mut_lamports()? = ctx
            .accounts
            .campaign_owner
            .lamports()
            .checked_add(campaign_balance)
            .ok_or(CrowdfundingError::MathOverflow)?;

        Ok(())
    }

    pub fn reclaim(ctx: Context<ReclaimCtx>, _campaign_name: String) -> Result<()> {
        let current_slot = Clock::get()?.slot;
        let campaign_pda = &mut ctx.accounts.campaign_pda;
        let deposit_pda = &ctx.accounts.deposit_pda;

        // Check if donation period has ended
        require!(
            current_slot > campaign_pda.end_donate_slot,
            CrowdfundingError::DonationPeriodNotEnded
        );

        // Check if goal was NOT reached
        let campaign_balance = campaign_pda.to_account_info().lamports();
        require!(
            campaign_balance < campaign_pda.goal_in_lamports,
            CrowdfundingError::GoalWasReached
        );

        let donated_amount = deposit_pda.total_donated;
        
        // Ensure there are funds to reclaim
        require!(
            donated_amount > 0,
            CrowdfundingError::NoFundsToReclaim
        );

        // Return donated amount from campaign PDA to donor
        **campaign_pda.to_account_info().try_borrow_mut_lamports()? = campaign_pda
            .to_account_info()
            .lamports()
            .checked_sub(donated_amount)
            .ok_or(CrowdfundingError::InsufficientFunds)?;

        **ctx.accounts.donor.try_borrow_mut_lamports()? = ctx
            .accounts
            .donor
            .lamports()
            .checked_add(donated_amount)
            .ok_or(CrowdfundingError::MathOverflow)?;

        // Close deposit PDA and return rent to donor
        let deposit_pda_lamports = ctx.accounts.deposit_pda.to_account_info().lamports();
        **ctx.accounts.deposit_pda.to_account_info().try_borrow_mut_lamports()? = 0;
        **ctx.accounts.donor.try_borrow_mut_lamports()? = ctx
            .accounts
            .donor
            .lamports()
            .checked_add(deposit_pda_lamports)
            .ok_or(CrowdfundingError::MathOverflow)?;

        Ok(())
    }
}

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
        constraint = campaign_pda.campaign_owner == campaign_owner.key() @ CrowdfundingError::UnauthorizedOwner
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
        seeds = ["deposit".as_ref(), campaign_name.as_ref(), donor.key().as_ref()],
        bump,
        close = donor
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
}

#[account]
#[derive(InitSpace)]
pub struct DepositPDA {
    pub total_donated: u64,
}

#[error_code]
pub enum CrowdfundingError {
    #[msg("End slot must be in the future")]
    InvalidEndSlot,
    
    #[msg("Goal must be greater than 0")]
    InvalidGoal,
    
    #[msg("Donation period has ended")]
    DonationPeriodEnded,
    
    #[msg("Donation amount must be greater than 0")]
    InvalidDonationAmount,
    
    #[msg("Donation period has not ended yet")]
    DonationPeriodNotEnded,
    
    #[msg("Goal has not been reached")]
    GoalNotReached,
    
    #[msg("Goal was already reached, cannot reclaim")]
    GoalWasReached,
    
    #[msg("No funds to reclaim")]
    NoFundsToReclaim,
    
    #[msg("Insufficient funds in campaign")]
    InsufficientFunds,
    
    #[msg("Math overflow occurred")]
    MathOverflow,
    
    #[msg("Unauthorized: not the campaign owner")]
    UnauthorizedOwner,
}