use anchor_lang::prelude::*;
use anchor_lang::system_program;

declare_id!("GbJdVnArDodix9r5TUnuUWy5hn14s14jzr26RyVbRY3j");

#[program]
pub mod crowdfund {
    use super::*;

    pub fn initialize(
        ctx: Context<Initialize>,
        campaign_name: String,
        end_donate_slot: u64,
        goal_in_lamports: u64,
    ) -> Result<()> {
        require!(!campaign_name.is_empty(), CrowdfundingError::InvalidCampaignName);
        require!(campaign_name.len() <= 50, CrowdfundingError::InvalidCampaignName);

        let clock = Clock::get()?;
        require!(end_donate_slot > clock.slot, CrowdfundingError::InvalidEndSlot);
        require!(goal_in_lamports > 0, CrowdfundingError::InvalidGoalAmount);

        let campaign = &mut ctx.accounts.campaign;
        campaign.campaign_owner = ctx.accounts.campaign_owner.key();
        campaign.campaign_name = campaign_name;
        campaign.end_donate_slot = end_donate_slot;
        campaign.goal_in_lamports = goal_in_lamports;
        campaign.total_donated = 0;

        msg!("Campaign initialized");
        Ok(())
    }

    pub fn donate(
        ctx: Context<Donate>,
        campaign_name: String,
        donated_lamports: u64,
    ) -> Result<()> {
        // Validate campaign is still active
        let clock = Clock::get()?;
        require!(
            clock.slot <= ctx.accounts.campaign.end_donate_slot,
            CrowdfundingError::CampaignEnded
        );
        require!(donated_lamports > 0, CrowdfundingError::InvalidDonationAmount);

        // Transfer lamports from donor to campaign PDA using CPI
        let cpi_context = CpiContext::new(
            ctx.accounts.system_program.to_account_info(),
            system_program::Transfer {
                from: ctx.accounts.donor.to_account_info(),
                to: ctx.accounts.campaign.to_account_info(),
            },
        );
        system_program::transfer(cpi_context, donated_lamports)?;

        // Update deposit and campaign totals
        let deposit = &mut ctx.accounts.deposit;
        deposit.total_donated = deposit.total_donated.checked_add(donated_lamports).unwrap();

        let campaign = &mut ctx.accounts.campaign;
        campaign.total_donated = campaign.total_donated.checked_add(donated_lamports).unwrap();

        msg!("Donated {} lamports", donated_lamports);
        Ok(())
    }

    pub fn withdraw(ctx: Context<Withdraw>, campaign_name: String) -> Result<()> {
        let campaign = &ctx.accounts.campaign;
        let clock = Clock::get()?;
        
        require!(clock.slot > campaign.end_donate_slot, CrowdfundingError::CampaignActive);
        require!(
            campaign.total_donated >= campaign.goal_in_lamports,
            CrowdfundingError::GoalNotReached
        );
        require!(
            ctx.accounts.campaign_owner.key() == campaign.campaign_owner,
            CrowdfundingError::Unauthorized
        );

        // Transfer all funds from campaign to owner
        let campaign_balance = campaign.to_account_info().lamports();
        
        **campaign.to_account_info().try_borrow_mut_lamports()? = 0;
        **ctx.accounts.campaign_owner.to_account_info().try_borrow_mut_lamports()? = 
            ctx.accounts.campaign_owner.to_account_info().lamports()
                .checked_add(campaign_balance).unwrap();

        msg!("Withdrawn {} lamports", campaign_balance);
        Ok(())
    }

    pub fn reclaim(ctx: Context<Reclaim>, campaign_name: String) -> Result<()> {
        let campaign = &ctx.accounts.campaign;
        let clock = Clock::get()?;
        
        require!(clock.slot > campaign.end_donate_slot, CrowdfundingError::CampaignActive);
        require!(
            campaign.total_donated < campaign.goal_in_lamports,
            CrowdfundingError::GoalReached
        );

        let deposit = &ctx.accounts.deposit;
        require!(deposit.total_donated > 0, CrowdfundingError::NoDonationFound);

        let refund_amount = deposit.total_donated;
        
        // Refund donor from campaign funds
        **campaign.to_account_info().try_borrow_mut_lamports()? = 
            campaign.to_account_info().lamports().checked_sub(refund_amount).unwrap();
        **ctx.accounts.donor.to_account_info().try_borrow_mut_lamports()? = 
            ctx.accounts.donor.to_account_info().lamports().checked_add(refund_amount).unwrap();

        msg!("Reclaimed {} lamports", refund_amount);
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
        space = 8 + Campaign::LEN,
        seeds = [campaign_name.as_bytes()],
        bump
    )]
    pub campaign: Account<'info, Campaign>,
    
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
        bump
    )]
    pub campaign: Account<'info, Campaign>,
    
    #[account(
        init_if_needed,
        payer = donor,
        space = 8 + Deposit::LEN,
        seeds = [b"deposit".as_ref(), campaign_name.as_bytes(), donor.key().as_ref()],
        bump
    )]
    pub deposit: Account<'info, Deposit>,
    
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
        bump,
        has_one = campaign_owner
    )]
    pub campaign: Account<'info, Campaign>,
}

#[derive(Accounts)]
#[instruction(campaign_name: String)]
pub struct Reclaim<'info> {
    #[account(mut)]
    pub donor: Signer<'info>,
    
    #[account(
        mut,
        seeds = [campaign_name.as_bytes()],
        bump
    )]
    pub campaign: Account<'info, Campaign>,
    
    #[account(
        mut,
        seeds = [b"deposit".as_ref(), campaign_name.as_bytes(), donor.key().as_ref()],
        bump,
        close = donor
    )]
    pub deposit: Account<'info, Deposit>,
}

#[account]
pub struct Campaign {
    pub campaign_owner: Pubkey,
    pub campaign_name: String,
    pub end_donate_slot: u64,
    pub goal_in_lamports: u64,
    pub total_donated: u64,
}

#[account]
pub struct Deposit {
    pub total_donated: u64,
}

impl Campaign {
    pub const LEN: usize = 32 + 50 + 8 + 8 + 8;
}

impl Deposit {
    pub const LEN: usize = 8;
}

#[error_code]
pub enum CrowdfundingError {
    #[msg("Invalid campaign name")]
    InvalidCampaignName,
    #[msg("Invalid end slot")]
    InvalidEndSlot,
    #[msg("Campaign ended")]
    CampaignEnded,
    #[msg("Campaign active")]
    CampaignActive,
    #[msg("Invalid goal amount")]
    InvalidGoalAmount,
    #[msg("Invalid donation amount")]
    InvalidDonationAmount,
    #[msg("Goal not reached")]
    GoalNotReached,
    #[msg("Goal reached")]
    GoalReached,
    #[msg("Unauthorized")]
    Unauthorized,
    #[msg("No donation found")]
    NoDonationFound,
    #[msg("Insufficient campaign funds")]
    InsufficientCampaignFunds,
}