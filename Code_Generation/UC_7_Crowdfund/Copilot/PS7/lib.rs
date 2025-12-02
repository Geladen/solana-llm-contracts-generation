use anchor_lang::prelude::*;
use anchor_lang::system_program;

declare_id!("ECXUC6J4pT28cj5cUw8uxYADdFTLEoU5SM4yCCKDCxzv");

const MAX_CAMPAIGN_NAME_LEN: usize = 32;
const CAMPAIGN_PDA_ACCOUNT_SIZE: usize = 8   // discriminator
    + 4 + MAX_CAMPAIGN_NAME_LEN            // String (4-byte len + content)
    + 32                                   // campaign_owner Pubkey
    + 8                                    // end_donate_slot
    + 8;                                   // goal_in_lamports
const DEPOSIT_PDA_ACCOUNT_SIZE: usize = 8   // discriminator
    + 8;                                   // total_donated

#[error_code]
pub enum CrowdfundingError {
    EndSlotInPast,
    DonationPeriodEnded,
    InsufficientDonation,
    InsufficientFunds,
    CampaignGoalNotReached,
    CampaignGoalAlreadyReached,
}

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

#[derive(Accounts)]
#[instruction(campaign_name: String)]
pub struct InitializeCtx<'info> {
    #[account(mut)]
    pub campaign_owner: Signer<'info>,

    #[account(
        init,
        seeds = [campaign_name.as_bytes()],
        bump,
        payer = campaign_owner,
        space = CAMPAIGN_PDA_ACCOUNT_SIZE
    )]
    pub campaign_pda: Account<'info, CampaignPDA>,

    pub system_program: Program<'info, System>,
    pub clock: Sysvar<'info, Clock>,
}

#[derive(Accounts)]
#[instruction(campaign_name: String)]
pub struct DonateCtx<'info> {
    #[account(mut)]
    pub donor: Signer<'info>,

    #[account(mut, seeds = [campaign_name.as_bytes()], bump)]
    pub campaign_pda: Account<'info, CampaignPDA>,

    #[account(
        init_if_needed,
        seeds = [
            b"deposit".as_ref(),
            campaign_name.as_bytes(),
            donor.key().as_ref()
        ],
        bump,
        payer = donor,
        space = DEPOSIT_PDA_ACCOUNT_SIZE
    )]
    pub deposit_pda: Account<'info, DepositPDA>,

    pub system_program: Program<'info, System>,
    pub clock: Sysvar<'info, Clock>,
}

#[derive(Accounts)]
#[instruction(campaign_name: String)]
pub struct WithdrawCtx<'info> {
    /// Program‐owned account; holds all lamports so far
    #[account(mut, has_one = campaign_owner, seeds = [campaign_name.as_bytes()], bump)]
    pub campaign_pda: Account<'info, CampaignPDA>,

    /// We must mark the owner `mut` to allow lamport credit
    #[account(mut)]
    pub campaign_owner: Signer<'info>,

    // system_program not needed here
}

#[derive(Accounts)]
#[instruction(campaign_name: String)]
pub struct ReclaimCtx<'info> {
    /// donor must be mutable so we can refund lamports
    #[account(mut)]
    pub donor: Signer<'info>,

    #[account(mut, seeds = [campaign_name.as_bytes()], bump)]
    pub campaign_pda: Account<'info, CampaignPDA>,

    /// closed at end, refunding rent to donor
    #[account(
        mut,
        seeds = [
            b"deposit".as_ref(),
            campaign_name.as_bytes(),
            donor.key().as_ref()
        ],
        bump,
        close = donor
    )]
    pub deposit_pda: Account<'info, DepositPDA>,
}

#[program]
pub mod crowdfund {
    use super::*;

    pub fn initialize(
        ctx: Context<InitializeCtx>,
        campaign_name: String,
        end_donate_slot: u64,
        goal_in_lamports: u64,
    ) -> Result<()> {
        let clock = &ctx.accounts.clock;
        require!(
            end_donate_slot > clock.slot,
            CrowdfundingError::EndSlotInPast
        );

        let campaign = &mut ctx.accounts.campaign_pda;
        campaign.campaign_name = campaign_name;
        campaign.campaign_owner = ctx.accounts.campaign_owner.key();
        campaign.end_donate_slot = end_donate_slot;
        campaign.goal_in_lamports = goal_in_lamports;
        Ok(())
    }

    pub fn donate(
        ctx: Context<DonateCtx>,
        _campaign_name: String,
        donated_lamports: u64,
    ) -> Result<()> {
        let clock = &ctx.accounts.clock;
        let campaign = &ctx.accounts.campaign_pda;

        require!(
            clock.slot <= campaign.end_donate_slot,
            CrowdfundingError::DonationPeriodEnded
        );
        require!(donated_lamports > 0, CrowdfundingError::InsufficientDonation);

        // CPI is fine here: donor → campaign_pda (system-owned accounts)
        let cpi = CpiContext::new(
            ctx.accounts.system_program.to_account_info(),
            anchor_lang::system_program::Transfer {
                from: ctx.accounts.donor.to_account_info(),
                to: campaign.to_account_info(),
            },
        );
        anchor_lang::system_program::transfer(cpi, donated_lamports)?;

        let deposit = &mut ctx.accounts.deposit_pda;
        deposit.total_donated = deposit
            .total_donated
            .checked_add(donated_lamports)
            .ok_or(CrowdfundingError::InsufficientDonation)?;
        Ok(())
    }

    pub fn withdraw(
        ctx: Context<WithdrawCtx>,
        _campaign_name: String,
    ) -> Result<()> {
        let campaign_info = ctx.accounts.campaign_pda.to_account_info();
        let owner_info = ctx.accounts.campaign_owner.to_account_info();

        // compute how much is beyond rent-exempt
        let rent = Rent::get()?;
        let exempt = rent.minimum_balance(campaign_info.data_len());
        let available = campaign_info
            .lamports()
            .checked_sub(exempt)
            .ok_or(CrowdfundingError::InsufficientFunds)?;

        require!(
            available >= ctx.accounts.campaign_pda.goal_in_lamports,
            CrowdfundingError::CampaignGoalNotReached
        );

        // direct lamport move
        **campaign_info.lamports.borrow_mut() -= available;
        **owner_info.lamports.borrow_mut() += available;
        Ok(())
    }

    pub fn reclaim(
        ctx: Context<ReclaimCtx>,
        _campaign_name: String,
    ) -> Result<()> {
        let campaign_info = ctx.accounts.campaign_pda.to_account_info();
        let donor_info = ctx.accounts.donor.to_account_info();

        let rent = Rent::get()?;
        let exempt = rent.minimum_balance(campaign_info.data_len());
        let available = campaign_info
            .lamports()
            .checked_sub(exempt)
            .ok_or(CrowdfundingError::InsufficientFunds)?;

        require!(
            available < ctx.accounts.campaign_pda.goal_in_lamports,
            CrowdfundingError::CampaignGoalAlreadyReached
        );

        let refund = ctx.accounts.deposit_pda.total_donated;
        require!(refund > 0, CrowdfundingError::InsufficientDonation);

        // refund donor
        **campaign_info.lamports.borrow_mut() -= refund;
        **donor_info.lamports.borrow_mut() += refund;

        // deposit_pda will be closed automatically, returning its rent to donor
        Ok(())
    }
}