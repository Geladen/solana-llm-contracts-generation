use anchor_lang::prelude::*;
use anchor_lang::system_program;

declare_id!("FwgJ4JkR56RGj7Q5TC2WDvua3PwY1io7AjDT98kyKZ4y");

#[program]
pub mod crowdfund {
    use super::*;

    pub fn initialize(
        ctx: Context<Initialize>,
        campaign_name: String,
        end_donate_slot: u64,
        goal_in_lamports: u64,
    ) -> Result<()> {
        // Validate end slot is in future
        require!(
            end_donate_slot > Clock::get()?.slot,
            CrowdfundingError::InvalidEndSlot
        );

        // Validate goal is reasonable
        require!(goal_in_lamports > 0, CrowdfundingError::InvalidGoal);

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
        _campaign_name: String,
        donated_lamports: u64,
    ) -> Result<()> {
        // Validate campaign is still active
        require!(
            Clock::get()?.slot <= ctx.accounts.campaign_pda.end_donate_slot,
            CrowdfundingError::CampaignEnded
        );

        // Validate donation amount
        require!(donated_lamports > 0, CrowdfundingError::InvalidDonation);

        // Transfer lamports from donor to campaign PDA
        let cpi_context = CpiContext::new(
            ctx.accounts.system_program.to_account_info(),
            system_program::Transfer {
                from: ctx.accounts.donor.to_account_info(),
                to: ctx.accounts.campaign_pda.to_account_info(),
            },
        );
        system_program::transfer(cpi_context, donated_lamports)?;

        // Update campaign total
        ctx.accounts.campaign_pda.total_donated = ctx
            .accounts
            .campaign_pda
            .total_donated
            .checked_add(donated_lamports)
            .unwrap();

        // Update donor's deposit tracking
        ctx.accounts.deposit_pda.total_donated = ctx
            .accounts
            .deposit_pda
            .total_donated
            .checked_add(donated_lamports)
            .unwrap();
        ctx.accounts.deposit_pda.bump = ctx.bumps.deposit_pda;

        Ok(())
    }

    pub fn withdraw(ctx: Context<Withdraw>, _campaign_name: String) -> Result<()> {
        // Validate campaign has ended
        require!(
            Clock::get()?.slot > ctx.accounts.campaign_pda.end_donate_slot,
            CrowdfundingError::CampaignActive
        );

        // Validate goal was reached
        require!(
            ctx.accounts.campaign_pda.total_donated >= ctx.accounts.campaign_pda.goal_in_lamports,
            CrowdfundingError::GoalNotReached
        );

        // Validate caller is campaign owner
        require!(
            ctx.accounts.campaign_owner.key() == ctx.accounts.campaign_pda.campaign_owner,
            CrowdfundingError::Unauthorized
        );

        // Transfer all funds from campaign PDA to owner
        let campaign_balance = ctx.accounts.campaign_pda.to_account_info().lamports();
        **ctx.accounts.campaign_pda.to_account_info().try_borrow_mut_lamports()? = ctx
            .accounts
            .campaign_pda
            .to_account_info()
            .lamports()
            .checked_sub(campaign_balance)
            .unwrap();
        **ctx.accounts.campaign_owner.try_borrow_mut_lamports()? = ctx
            .accounts
            .campaign_owner
            .lamports()
            .checked_add(campaign_balance)
            .unwrap();

        Ok(())
    }

    pub fn reclaim(ctx: Context<Reclaim>, _campaign_name: String) -> Result<()> {
        // Validate campaign has ended
        require!(
            Clock::get()?.slot > ctx.accounts.campaign_pda.end_donate_slot,
            CrowdfundingError::CampaignActive
        );

        // Validate goal was NOT reached
        require!(
            ctx.accounts.campaign_pda.total_donated < ctx.accounts.campaign_pda.goal_in_lamports,
            CrowdfundingError::GoalReached
        );

        // Validate donor has actually donated
        require!(
            ctx.accounts.deposit_pda.total_donated > 0,
            CrowdfundingError::NoDonationFound
        );

        let refund_amount = ctx.accounts.deposit_pda.total_donated;

        // Close deposit PDA and return rent
        let deposit_pda_lamports = ctx.accounts.deposit_pda.to_account_info().lamports();
        **ctx.accounts.deposit_pda.to_account_info().try_borrow_mut_lamports()? = 0;
        **ctx.accounts.donor.try_borrow_mut_lamports()? = ctx
            .accounts
            .donor
            .lamports()
            .checked_add(deposit_pda_lamports)
            .unwrap();

        // Refund donation from campaign PDA
        **ctx.accounts.campaign_pda.to_account_info().try_borrow_mut_lamports()? = ctx
            .accounts
            .campaign_pda
            .to_account_info()
            .lamports()
            .checked_sub(refund_amount)
            .unwrap();
        **ctx.accounts.donor.try_borrow_mut_lamports()? = ctx
            .accounts
            .donor
            .lamports()
            .checked_add(refund_amount)
            .unwrap();

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
        space = CampaignPDA::SIZE,
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
        space = DepositPDA::SIZE,
        seeds = [b"deposit".as_ref(), campaign_name.as_bytes(), donor.key().as_ref()],
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
        seeds = [b"deposit".as_ref(), campaign_name.as_bytes(), donor.key().as_ref()],
        bump = deposit_pda.bump
    )]
    pub deposit_pda: Account<'info, DepositPDA>,
}

#[account]
pub struct CampaignPDA {
    pub campaign_name: String,
    pub campaign_owner: Pubkey,
    pub end_donate_slot: u64,
    pub goal_in_lamports: u64,
    pub total_donated: u64,
    pub bump: u8,
}

impl CampaignPDA {
    const SIZE: usize = 8 + // discriminator
        4 + 32 + // campaign_name (string with length prefix + max 32 chars)
        32 + // campaign_owner
        8 +  // end_donate_slot
        8 +  // goal_in_lamports
        8 +  // total_donated
        1;   // bump
}

#[account]
pub struct DepositPDA {
    pub total_donated: u64,
    pub bump: u8,
}

impl DepositPDA {
    const SIZE: usize = 8 + // discriminator
        8 + // total_donated
        1;  // bump
}

#[error_code]
pub enum CrowdfundingError {
    #[msg("End slot must be in the future")]
    InvalidEndSlot,
    #[msg("Goal must be greater than zero")]
    InvalidGoal,
    #[msg("Campaign has ended")]
    CampaignEnded,
    #[msg("Campaign is still active")]
    CampaignActive,
    #[msg("Donation amount must be greater than zero")]
    InvalidDonation,
    #[msg("Campaign goal not reached")]
    GoalNotReached,
    #[msg("Campaign goal was reached, funds cannot be reclaimed")]
    GoalReached,
    #[msg("Unauthorized access")]
    Unauthorized,
    #[msg("No donation found for this donor")]
    NoDonationFound,
}