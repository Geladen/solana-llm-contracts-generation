use anchor_lang::prelude::*;
use anchor_lang::system_program::{self, Transfer};
use anchor_lang::prelude::Rent;    // <-- import Rent

declare_id!("87y3ceti5rgj333AiH8nujh25cZ9Sx3S8GbJ4XhhY8c6");

pub const MAX_CAMPAIGN_NAME_LEN: usize = 32;

#[error_code]
pub enum CrowdfundError {
    #[msg("end_donate_slot must be in the future")]
    EndSlotInPast,
    #[msg("donation window has closed")]
    DonationDeadlinePassed,
    #[msg("campaign goal not reached")]
    GoalNotReached,
    #[msg("campaign goal already reached")]
    GoalAlreadyReached,
    #[msg("withdrawal too early; campaign still running")]
    TooEarlyWithdraw,
    #[msg("reclaim too early; campaign still running")]
    TooEarlyReclaim,
    #[msg("nothing to reclaim")]
    NothingToReclaim,
}

#[account]
pub struct CampaignPDA {
    pub campaign_name: String,
    pub campaign_owner: Pubkey,
    pub end_donate_slot: u64,
    pub goal_in_lamports: u64,
}
impl CampaignPDA {
    pub const LEN: usize = 8
        + 4 + MAX_CAMPAIGN_NAME_LEN
        + 32
        + 8
        + 8;
}

#[account]
pub struct DepositPDA {
    pub total_donated: u64,
}
impl DepositPDA {
    pub const LEN: usize = 8 + 8;
}

#[derive(Accounts)]
#[instruction(campaign_name: String, end_donate_slot: u64, goal_in_lamports: u64)]
pub struct InitializeCtx<'info> {
    #[account(mut)]
    pub campaign_owner: Signer<'info>,

    #[account(
        init,
        payer = campaign_owner,
        space = CampaignPDA::LEN,
        seeds = [campaign_name.as_bytes()],
        bump
    )]
    pub campaign_pda: Account<'info, CampaignPDA>,

    pub system_program: Program<'info, System>,
}

#[derive(Accounts)]
#[instruction(campaign_name: String, donated_lamports: u64)]
pub struct DonateCtx<'info> {
    #[account(mut)]
    pub donor: Signer<'info>,

    #[account(
        mut,
        seeds = [campaign_name.as_bytes()],
        bump
    )]
    pub campaign_pda: Account<'info, CampaignPDA>,

    #[account(
        init_if_needed,
        payer = donor,
        space = DepositPDA::LEN,
        seeds = [b"deposit", campaign_name.as_bytes(), donor.key().as_ref()],
        bump
    )]
    pub deposit_pda: Account<'info, DepositPDA>,

    pub system_program: Program<'info, System>,
}

#[derive(Accounts)]
#[instruction(campaign_name: String)]
pub struct WithdrawCtx<'info> {
    #[account(mut)]
    pub campaign_owner: Signer<'info>,

    #[account(
        mut,
        seeds = [campaign_name.as_bytes()],
        bump,
        has_one = campaign_owner
    )]
    pub campaign_pda: Account<'info, CampaignPDA>,

    pub system_program: Program<'info, System>,
}

#[derive(Accounts)]
#[instruction(campaign_name: String)]
pub struct ReclaimCtx<'info> {
    #[account(mut)]
    pub donor: Signer<'info>,

    #[account(
        mut,
        seeds = [campaign_name.as_bytes()],
        bump
    )]
    pub campaign_pda: Account<'info, CampaignPDA>,

    #[account(
        mut,
        close = donor,
        seeds = [b"deposit", campaign_name.as_bytes(), donor.key().as_ref()],
        bump
    )]
    pub deposit_pda: Account<'info, DepositPDA>,

    pub system_program: Program<'info, System>,
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
        let clock = Clock::get()?;
        require!(
            end_donate_slot > clock.slot,
            CrowdfundError::EndSlotInPast
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
        let clock = Clock::get()?;
        let campaign = &ctx.accounts.campaign_pda;
        require!(
            clock.slot <= campaign.end_donate_slot,
            CrowdfundError::DonationDeadlinePassed
        );

        // Transfer lamports
        let cpi_ctx = CpiContext::new(
            ctx.accounts.system_program.to_account_info(),
            Transfer {
                from: ctx.accounts.donor.to_account_info(),
                to: campaign.to_account_info(),
            },
        );
        system_program::transfer(cpi_ctx, donated_lamports)?;

        // Update donor record
        let deposit = &mut ctx.accounts.deposit_pda;
        deposit.total_donated = deposit
            .total_donated
            .checked_add(donated_lamports)
            .unwrap();
        Ok(())
    }

    pub fn withdraw(
        ctx: Context<WithdrawCtx>,
        _campaign_name: String,
    ) -> Result<()> {
        let clock = Clock::get()?;
        let campaign_acc = &mut ctx.accounts.campaign_pda.to_account_info();
        let owner_acc    = &mut ctx.accounts.campaign_owner.to_account_info();

        // must be after deadline & goal reached
        require!(clock.slot > ctx.accounts.campaign_pda.end_donate_slot, CrowdfundError::TooEarlyWithdraw);

        let current_balance = **campaign_acc.lamports.borrow();
        require!(current_balance >= ctx.accounts.campaign_pda.goal_in_lamports, CrowdfundError::GoalNotReached);

        // leave rent‐exempt minimum behind
        let rent = Rent::get()?.minimum_balance(campaign_acc.data_len());
        let to_transfer = current_balance.checked_sub(rent).unwrap();

        **campaign_acc.try_borrow_mut_lamports()? -= to_transfer;
        **owner_acc.try_borrow_mut_lamports()?    += to_transfer;

        Ok(())
    }

    pub fn reclaim(
        ctx: Context<ReclaimCtx>,
        _campaign_name: String,
    ) -> Result<()> {
        let clock = Clock::get()?;
        let campaign_acc = &mut ctx.accounts.campaign_pda.to_account_info();
        let donor_acc    = &mut ctx.accounts.donor.to_account_info();
        let deposit      = &ctx.accounts.deposit_pda;

        // must be after deadline & goal not met
        require!(clock.slot > ctx.accounts.campaign_pda.end_donate_slot, CrowdfundError::TooEarlyReclaim);
        let current_balance = **campaign_acc.lamports.borrow();
        require!(current_balance < ctx.accounts.campaign_pda.goal_in_lamports, CrowdfundError::GoalAlreadyReached);

        require!(deposit.total_donated > 0, CrowdfundError::NothingToReclaim);

        // refund exactly what this donor gave
        let to_refund = deposit.total_donated;
        **campaign_acc.try_borrow_mut_lamports()? -= to_refund;
        **donor_acc.try_borrow_mut_lamports()?     += to_refund;
        // deposit_pda is closed by Anchor, returning its rent to donor

        Ok(())
    }
}
