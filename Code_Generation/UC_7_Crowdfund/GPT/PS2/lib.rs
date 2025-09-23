use anchor_lang::prelude::*;
use anchor_lang::solana_program::rent::Rent;

declare_id!("5KgDeERz9bjTbTarz1LKqDBB14iCtABsCxUsktj64Dmg");

#[program]
pub mod crowdfund {
    use super::*;

    pub fn initialize(
        ctx: Context<Initialize>,
        campaign_name: String,
        end_donate_slot: u64,
        goal_in_lamports: u64,
    ) -> Result<()> {
        let current_slot = Clock::get()?.slot;

        require!(
            !campaign_name.is_empty() && campaign_name.as_bytes().len() <= CampaignPDA::MAX_NAME_LEN,
            ErrorCode::InvalidCampaignName
        );

        require!(end_donate_slot > current_slot, ErrorCode::EndSlotInPast);
        require!(goal_in_lamports > 0, ErrorCode::InvalidGoal);

        let campaign = &mut ctx.accounts.campaign_pda;
        campaign.campaign_name = campaign_name;
        campaign.campaign_owner = ctx.accounts.campaign_owner.key();
        campaign.end_donate_slot = end_donate_slot;
        campaign.goal_in_lamports = goal_in_lamports;

        Ok(())
    }

    pub fn donate(
        ctx: Context<Donate>,
        _campaign_name: String,
        donated_lamports: u64,
    ) -> Result<()> {
        let clock = Clock::get()?;
        require!(
            clock.slot <= ctx.accounts.campaign_pda.end_donate_slot,
            ErrorCode::DonationPeriodEnded
        );
        require!(donated_lamports > 0, ErrorCode::InvalidDonationAmount);

        let cpi_accounts = anchor_lang::system_program::Transfer {
            from: ctx.accounts.donor.to_account_info(),
            to: ctx.accounts.campaign_pda.to_account_info(),
        };
        let cpi_ctx = CpiContext::new(ctx.accounts.system_program.to_account_info(), cpi_accounts);
        anchor_lang::system_program::transfer(cpi_ctx, donated_lamports)?;

        let deposit = &mut ctx.accounts.deposit_pda;
        deposit.total_donated = deposit
            .total_donated
            .checked_add(donated_lamports)
            .ok_or(ErrorCode::Overflow)?;

        Ok(())
    }



pub fn withdraw(ctx: Context<Withdraw>, _campaign_name: String) -> Result<()> {
    let campaign_info = ctx.accounts.campaign_pda.to_account_info();
    let owner_info = ctx.accounts.campaign_owner.to_account_info();
    let campaign = &ctx.accounts.campaign_pda;

    let balance = **campaign_info.lamports.borrow();
    let rent_exempt_min = Rent::get()?.minimum_balance(CampaignPDA::LEN + 8);

    require!(balance >= campaign.goal_in_lamports, ErrorCode::GoalNotReached);
    require!(balance > rent_exempt_min, ErrorCode::InsufficientFundsInCampaign);

    // Only withdraw excess above rent exemption
    let withdrawable = balance
        .checked_sub(rent_exempt_min)
        .ok_or(ErrorCode::Overflow)?;

    let owner_balance = **owner_info.lamports.borrow();
    **owner_info.lamports.borrow_mut() = owner_balance
        .checked_add(withdrawable)
        .ok_or(ErrorCode::Overflow)?;
    **campaign_info.lamports.borrow_mut() = rent_exempt_min;

    Ok(())
}


pub fn reclaim(ctx: Context<Reclaim>, _campaign_name: String) -> Result<()> {
    let campaign_info = ctx.accounts.campaign_pda.to_account_info();
    let donor_info = ctx.accounts.donor.to_account_info();
    let deposit = &ctx.accounts.deposit_pda;
    let campaign = &ctx.accounts.campaign_pda;

    let campaign_balance = **campaign_info.lamports.borrow();
    let rent_exempt_min = Rent::get()?.minimum_balance(CampaignPDA::LEN + 8);

    require!(
        campaign_balance < campaign.goal_in_lamports,
        ErrorCode::GoalAlreadyReached
    );

    let amount = deposit.total_donated;
    require!(amount > 0, ErrorCode::NothingToReclaim);
    require!(
        campaign_balance.saturating_sub(rent_exempt_min) >= amount,
        ErrorCode::InsufficientFundsInCampaign
    );

    let donor_balance = **donor_info.lamports.borrow();
    **donor_info.lamports.borrow_mut() = donor_balance
        .checked_add(amount)
        .ok_or(ErrorCode::Overflow)?;
    **campaign_info.lamports.borrow_mut() = campaign_balance - amount;

    Ok(())
}

}

/* ========================= Accounts & State ========================= */

#[derive(Accounts)]
#[instruction(campaign_name: String)]
pub struct Initialize<'info> {
    #[account(mut)]
    pub campaign_owner: Signer<'info>,

    #[account(
        init,
        payer = campaign_owner,
        space = 8 + CampaignPDA::LEN,
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

    #[account(mut, seeds = [campaign_name.as_bytes()], bump)]
    pub campaign_pda: Account<'info, CampaignPDA>,

    #[account(
        init_if_needed,
        payer = donor,
        space = 8 + DepositPDA::LEN,
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
        bump,
        has_one = campaign_owner
    )]
    pub campaign_pda: Account<'info, CampaignPDA>,
}

#[derive(Accounts)]
#[instruction(campaign_name: String)]
pub struct Reclaim<'info> {
    #[account(mut)]
    pub donor: Signer<'info>,

    #[account(mut, seeds = [campaign_name.as_bytes()], bump)]
    pub campaign_pda: Account<'info, CampaignPDA>,

    #[account(
        mut,
        seeds = [b"deposit", campaign_name.as_bytes(), donor.key().as_ref()],
        bump,
        close = donor
    )]
    pub deposit_pda: Account<'info, DepositPDA>,
}

/* ========================= State structs ========================= */

#[account]
pub struct CampaignPDA {
    pub campaign_name: String,
    pub campaign_owner: Pubkey,
    pub end_donate_slot: u64,
    pub goal_in_lamports: u64,
}
impl CampaignPDA {
    pub const MAX_NAME_LEN: usize = 32;
    pub const LEN: usize = 4 + Self::MAX_NAME_LEN + 32 + 8 + 8;
}

#[account]
pub struct DepositPDA {
    pub total_donated: u64,
}
impl DepositPDA {
    pub const LEN: usize = 8;
}

/* ========================= Errors ========================= */

#[error_code]
pub enum ErrorCode {
    #[msg("End slot must be greater than current slot.")]
    EndSlotInPast,
    #[msg("Campaign goal must be > 0.")]
    InvalidGoal,
    #[msg("Invalid campaign name.")]
    InvalidCampaignName,
    #[msg("Donation period has already ended.")]
    DonationPeriodEnded,
    #[msg("Donation amount must be greater than zero.")]
    InvalidDonationAmount,
    #[msg("Numeric overflow.")]
    Overflow,
    #[msg("Campaign goal not reached yet.")]
    GoalNotReached,
    #[msg("Campaign goal already reached — cannot reclaim.")]
    GoalAlreadyReached,
    #[msg("Nothing recorded to reclaim for this deposit.")]
    NothingToReclaim,
    #[msg("Campaign does not have sufficient funds to refund.")]
    InsufficientFundsInCampaign,
}
