use anchor_lang::prelude::*;

declare_id!("EG37kUpfWREwZUncG16biC76XUsb8vNkCA3gFsMSYp3n");

const MAX_NAME_LENGTH: usize = 64;
const CAMPAIGN_SPACE: usize = 8    // discriminator
    + 4 + MAX_NAME_LENGTH           // campaign_name: String
    + 32                            // campaign_owner: Pubkey
    + 8                             // end_donate_slot: u64
    + 8;                            // goal_in_lamports: u64

const DEPOSIT_SPACE: usize = 8    // discriminator
    + 8;                            // total_donated: u64

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
            ErrorCode::EndSlotInPast
        );

        let campaign = &mut ctx.accounts.campaign_pda;
        campaign.campaign_name = campaign_name;
        // Signer has a `key()` method which returns &Pubkey
        campaign.campaign_owner = *ctx.accounts.campaign_owner.key;
        campaign.end_donate_slot = end_donate_slot;
        campaign.goal_in_lamports = goal_in_lamports;
        Ok(())
    }

    pub fn donate(
        ctx: Context<DonateCtx>,
        campaign_name: String,
        donated_lamports: u64,
    ) -> Result<()> {
        let campaign = &ctx.accounts.campaign_pda;
        let clock = Clock::get()?;
        require!(
            clock.slot <= campaign.end_donate_slot,
            ErrorCode::DonateAfterDeadline
        );

        // transfer lamports from donor to campaign PDA
        let cpi_ctx = CpiContext::new(
            ctx.accounts.system_program.to_account_info(),
            anchor_lang::system_program::Transfer {
                from: ctx.accounts.donor.to_account_info(),
                to: ctx.accounts.campaign_pda.to_account_info(),
            },
        );
        anchor_lang::system_program::transfer(cpi_ctx, donated_lamports)?;

        // update donor deposit
        let deposit = &mut ctx.accounts.deposit_pda;
        deposit.total_donated = deposit
            .total_donated
            .checked_add(donated_lamports)
            .ok_or(ErrorCode::NumericalOverflow)?;
        Ok(())
    }

    pub fn withdraw(
        ctx: Context<WithdrawCtx>,
        _campaign_name: String,
    ) -> Result<()> {
        let campaign = &ctx.accounts.campaign_pda;

        // compute how many lamports exceed rent-exempt
        let rent_exempt = Rent::get()?.minimum_balance(CAMPAIGN_SPACE);
        let balance = ctx.accounts.campaign_pda.to_account_info().lamports();
        let available = balance
            .checked_sub(rent_exempt)
            .ok_or(ErrorCode::NumericalOverflow)?;

        require!(
            available >= campaign.goal_in_lamports,
            ErrorCode::GoalNotReached
        );

        // move available funds to owner
        **ctx
            .accounts
            .campaign_pda
            .to_account_info()
            .try_borrow_mut_lamports()? -= available;
        **ctx
            .accounts
            .campaign_owner
            .to_account_info()
            .try_borrow_mut_lamports()? += available;
        Ok(())
    }

    pub fn reclaim(
        ctx: Context<ReclaimCtx>,
        _campaign_name: String,
    ) -> Result<()> {
        let campaign = &ctx.accounts.campaign_pda;
        let clock = Clock::get()?;
        require!(
            clock.slot > campaign.end_donate_slot,
            ErrorCode::ReclaimTooEarly
        );

        let rent_exempt = Rent::get()?.minimum_balance(CAMPAIGN_SPACE);
        let balance = ctx.accounts.campaign_pda.to_account_info().lamports();
        let collected = balance
            .checked_sub(rent_exempt)
            .ok_or(ErrorCode::NumericalOverflow)?;

        require!(
            collected < campaign.goal_in_lamports,
            ErrorCode::GoalAlreadyMet
        );

        let donated = ctx.accounts.deposit_pda.total_donated;

        // refund donor
        **ctx
            .accounts
            .campaign_pda
            .to_account_info()
            .try_borrow_mut_lamports()? -= donated;
        **ctx
            .accounts
            .donor
            .to_account_info()
            .try_borrow_mut_lamports()? += donated;
        // deposit_pda is closed by `close = donor`
        Ok(())
    }
}

#[derive(Accounts)]
#[instruction(campaign_name: String, end_donate_slot: u64, goal_in_lamports: u64)]
pub struct InitializeCtx<'info> {
    #[account(mut)]
    pub campaign_owner: Signer<'info>,

    #[account(
        init,
        seeds = [campaign_name.as_bytes()],
        bump,
        payer = campaign_owner,
        space = CAMPAIGN_SPACE,
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
        bump,
    )]
    pub campaign_pda: Account<'info, CampaignPDA>,

    #[account(
        init_if_needed,
        seeds = [
            b"deposit",
            campaign_name.as_bytes(),
            donor.key().as_ref()
        ],
        bump,
        payer = donor,
        space = DEPOSIT_SPACE,
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
        has_one = campaign_owner,
    )]
    pub campaign_pda: Account<'info, CampaignPDA>,
}

#[derive(Accounts)]
#[instruction(campaign_name: String)]
pub struct ReclaimCtx<'info> {
    #[account(mut)]
    pub donor: Signer<'info>,

    #[account(mut, seeds = [campaign_name.as_bytes()], bump)]
    pub campaign_pda: Account<'info, CampaignPDA>,

    #[account(
        mut,
        seeds = [
            b"deposit",
            campaign_name.as_bytes(),
            donor.key().as_ref()
        ],
        bump,
        close = donor
    )]
    pub deposit_pda: Account<'info, DepositPDA>,
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

#[error_code]
pub enum ErrorCode {
    #[msg("End slot must be in the future")]
    EndSlotInPast,
    #[msg("Cannot donate after the deadline")]
    DonateAfterDeadline,
    #[msg("Goal has not been reached")]
    GoalNotReached,
    #[msg("Campaign goal was met; cannot reclaim")]
    GoalAlreadyMet,
    #[msg("Too early to reclaim funds")]
    ReclaimTooEarly,
    #[msg("Numerical operation overflow")]
    NumericalOverflow,
}
