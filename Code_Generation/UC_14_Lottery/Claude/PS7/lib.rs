use anchor_lang::prelude::*;
use anchor_lang::system_program;
use anchor_lang::solana_program::keccak;
use borsh::{BorshDeserialize, BorshSerialize};

declare_id!("9mPmseXe1paaxfxFfrxwMJuER4kw6w1rfSgo5fA6G4R6");

#[program]
pub mod lottery {
    use super::*;

    pub fn join(
        ctx: Context<JoinCtx>,
        hashlock1: [u8; 32],
        hashlock2: [u8; 32],
        delay: u64,
        amount: u64,
    ) -> Result<()> {
        // Reject identical hash commitments
        require!(hashlock1 != hashlock2, LotteryError::IdenticalHashlocks);

        let clock = Clock::get()?;
        let end_reveal = clock.unix_timestamp as u64 + delay;

        // Reject end_reveal in the past
        require!(
            end_reveal > clock.unix_timestamp as u64,
            LotteryError::DeadlineInPast
        );

        let lottery_info = &mut ctx.accounts.lottery_info;

        // Initialize lottery state
        lottery_info.state = LotteryState::Init;
        lottery_info.player1 = ctx.accounts.player1.key();
        lottery_info.player2 = ctx.accounts.player2.key();
        lottery_info.hashlock1 = hashlock1;
        lottery_info.hashlock2 = hashlock2;
        lottery_info.secret1 = String::new();
        lottery_info.secret2 = String::new();
        lottery_info.end_reveal = end_reveal;

        // Transfer amount from player1 to lottery PDA
        let cpi_context_p1 = CpiContext::new(
            ctx.accounts.system_program.to_account_info(),
            system_program::Transfer {
                from: ctx.accounts.player1.to_account_info(),
                to: ctx.accounts.lottery_info.to_account_info(),
            },
        );
        system_program::transfer(cpi_context_p1, amount)?;

        // Transfer amount from player2 to lottery PDA
        let cpi_context_p2 = CpiContext::new(
            ctx.accounts.system_program.to_account_info(),
            system_program::Transfer {
                from: ctx.accounts.player2.to_account_info(),
                to: ctx.accounts.lottery_info.to_account_info(),
            },
        );
        system_program::transfer(cpi_context_p2, amount)?;

        Ok(())
    }

    pub fn reveal_p1(ctx: Context<RevealP1Ctx>, secret: String) -> Result<()> {
        let lottery_info = &mut ctx.accounts.lottery_info;
        let clock = Clock::get()?;

        // Must be in Init state
        require!(
            lottery_info.state == LotteryState::Init,
            LotteryError::InvalidState
        );

        // Check deadline
        require!(
            (clock.unix_timestamp as u64) <= lottery_info.end_reveal,
            LotteryError::DeadlinePassed
        );

        // Validate secret matches hashlock1 using Keccak-256
        let hash = keccak::hash(secret.as_bytes());
        require!(
            hash.to_bytes() == lottery_info.hashlock1,
            LotteryError::InvalidSecret
        );

        // Update state and store secret
        lottery_info.state = LotteryState::RevealP1;
        lottery_info.secret1 = secret;

        Ok(())
    }

    pub fn reveal_p2(ctx: Context<RevealP2Ctx>, secret: String) -> Result<()> {
        let lottery_info = &mut ctx.accounts.lottery_info;
        let clock = Clock::get()?;

        // Must be in RevealP1 state
        require!(
            lottery_info.state == LotteryState::RevealP1,
            LotteryError::InvalidState
        );

        // Check deadline with extension (2x the original delay)
        let extension = lottery_info.end_reveal;
        require!(
            (clock.unix_timestamp as u64) <= extension * 2,
            LotteryError::DeadlinePassed
        );

        // Validate secret matches hashlock2 using Keccak-256
        let hash = keccak::hash(secret.as_bytes());
        require!(
            hash.to_bytes() == lottery_info.hashlock2,
            LotteryError::InvalidSecret
        );

        // Update state and store secret
        lottery_info.state = LotteryState::RevealP2;
        lottery_info.secret2 = secret;

        // Determine winner using fair function
        let sum = lottery_info.secret1.len() + lottery_info.secret2.len();
        let winner_is_p1 = sum % 2 == 0;

        // Transfer entire pot to winner via direct lamports manipulation
        let lottery_lamports = lottery_info.to_account_info().lamports();
        let rent_exempt = Rent::get()?.minimum_balance(lottery_info.to_account_info().data_len());
        let prize = lottery_lamports.saturating_sub(rent_exempt);

        if winner_is_p1 {
            **lottery_info.to_account_info().try_borrow_mut_lamports()? -= prize;
            **ctx.accounts.player1.to_account_info().try_borrow_mut_lamports()? += prize;
        } else {
            **lottery_info.to_account_info().try_borrow_mut_lamports()? -= prize;
            **ctx.accounts.player2.to_account_info().try_borrow_mut_lamports()? += prize;
        }

        Ok(())
    }

    pub fn redeem_if_p1_no_reveal(ctx: Context<RedeemIfP1NoRevealCtx>) -> Result<()> {
        let lottery_info = &mut ctx.accounts.lottery_info;
        let clock = Clock::get()?;

        // Must be in Init state (player1 never revealed)
        require!(
            lottery_info.state == LotteryState::Init,
            LotteryError::InvalidState
        );

        // Check deadline has passed
        require!(
            (clock.unix_timestamp as u64) > lottery_info.end_reveal,
            LotteryError::DeadlineNotPassed
        );

        // Transfer entire pot to player2 as penalty
        let lottery_lamports = lottery_info.to_account_info().lamports();
        let rent_exempt = Rent::get()?.minimum_balance(lottery_info.to_account_info().data_len());
        let prize = lottery_lamports.saturating_sub(rent_exempt);

        **lottery_info.to_account_info().try_borrow_mut_lamports()? -= prize;
        **ctx.accounts.player2.to_account_info().try_borrow_mut_lamports()? += prize;

        Ok(())
    }

    pub fn redeem_if_p2_no_reveal(ctx: Context<RedeemIfP2NoRevealCtx>) -> Result<()> {
        let lottery_info = &mut ctx.accounts.lottery_info;
        let clock = Clock::get()?;

        // Must be in RevealP1 state (player2 never revealed)
        require!(
            lottery_info.state == LotteryState::RevealP1,
            LotteryError::InvalidState
        );

        // Check deadline + extension has passed
        let extension = lottery_info.end_reveal * 2;
        require!(
            (clock.unix_timestamp as u64) > extension,
            LotteryError::DeadlineNotPassed
        );

        // Transfer entire pot to player1 as penalty
        let lottery_lamports = lottery_info.to_account_info().lamports();
        let rent_exempt = Rent::get()?.minimum_balance(lottery_info.to_account_info().data_len());
        let prize = lottery_lamports.saturating_sub(rent_exempt);

        **lottery_info.to_account_info().try_borrow_mut_lamports()? -= prize;
        **ctx.accounts.player1.to_account_info().try_borrow_mut_lamports()? += prize;

        Ok(())
    }
}

// Context Structs

#[derive(Accounts)]
pub struct JoinCtx<'info> {
    #[account(mut)]
    pub player1: Signer<'info>,
    #[account(mut)]
    pub player2: Signer<'info>,
    #[account(
        init,
        payer = player1,
        space = 8 + LotteryInfo::INIT_SPACE,
        seeds = [player1.key().as_ref(), player2.key().as_ref()],
        bump
    )]
    pub lottery_info: Account<'info, LotteryInfo>,
    pub system_program: Program<'info, System>,
}

#[derive(Accounts)]
pub struct RevealP1Ctx<'info> {
    #[account(mut)]
    pub player1: Signer<'info>,
    /// CHECK: Player2 is validated in the lottery_info account
    pub player2: AccountInfo<'info>,
    #[account(
        mut,
        seeds = [player1.key().as_ref(), player2.key().as_ref()],
        bump,
        constraint = lottery_info.player1 == player1.key() @ LotteryError::InvalidPlayer,
        constraint = lottery_info.player2 == player2.key() @ LotteryError::InvalidPlayer
    )]
    pub lottery_info: Account<'info, LotteryInfo>,
}

#[derive(Accounts)]
pub struct RevealP2Ctx<'info> {
    /// CHECK: Player1 is validated in the lottery_info account
    #[account(mut)]
    pub player1: AccountInfo<'info>,
    #[account(mut)]
    pub player2: Signer<'info>,
    #[account(
        mut,
        seeds = [player1.key().as_ref(), player2.key().as_ref()],
        bump,
        constraint = lottery_info.player1 == player1.key() @ LotteryError::InvalidPlayer,
        constraint = lottery_info.player2 == player2.key() @ LotteryError::InvalidPlayer
    )]
    pub lottery_info: Account<'info, LotteryInfo>,
}

#[derive(Accounts)]
pub struct RedeemIfP1NoRevealCtx<'info> {
    /// CHECK: Player1 is validated in the lottery_info account
    #[account(mut)]
    pub player1: AccountInfo<'info>,
    #[account(mut)]
    pub player2: Signer<'info>,
    #[account(
        mut,
        seeds = [player1.key().as_ref(), player2.key().as_ref()],
        bump,
        constraint = lottery_info.player1 == player1.key() @ LotteryError::InvalidPlayer,
        constraint = lottery_info.player2 == player2.key() @ LotteryError::InvalidPlayer
    )]
    pub lottery_info: Account<'info, LotteryInfo>,
}

#[derive(Accounts)]
pub struct RedeemIfP2NoRevealCtx<'info> {
    #[account(mut)]
    pub player1: Signer<'info>,
    /// CHECK: Player2 is validated in the lottery_info account
    #[account(mut)]
    pub player2: AccountInfo<'info>,
    #[account(
        mut,
        seeds = [player1.key().as_ref(), player2.key().as_ref()],
        bump,
        constraint = lottery_info.player1 == player1.key() @ LotteryError::InvalidPlayer,
        constraint = lottery_info.player2 == player2.key() @ LotteryError::InvalidPlayer
    )]
    pub lottery_info: Account<'info, LotteryInfo>,
}

// Account Structures

#[account]
#[derive(InitSpace)]
pub struct LotteryInfo {
    pub state: LotteryState,
    pub player1: Pubkey,
    pub player2: Pubkey,
    pub hashlock1: [u8; 32],
    #[max_len(64)]
    pub secret1: String,
    pub hashlock2: [u8; 32],
    #[max_len(64)]
    pub secret2: String,
    pub end_reveal: u64,
}

// Enums

#[derive(AnchorSerialize, AnchorDeserialize, Clone, Copy, PartialEq, Eq, InitSpace)]
pub enum LotteryState {
    Init = 0,
    RevealP1 = 1,
    RevealP2 = 2,
}

// Error Codes

#[error_code]
pub enum LotteryError {
    #[msg("Identical hashlocks are not allowed")]
    IdenticalHashlocks,
    #[msg("Deadline is in the past")]
    DeadlineInPast,
    #[msg("Invalid lottery state")]
    InvalidState,
    #[msg("Deadline has passed")]
    DeadlinePassed,
    #[msg("Invalid secret provided")]
    InvalidSecret,
    #[msg("Deadline has not passed yet")]
    DeadlineNotPassed,
    #[msg("Invalid player")]
    InvalidPlayer,
}