use anchor_lang::prelude::*;
use borsh::{BorshDeserialize, BorshSerialize};
use anchor_lang::solana_program::keccak::hash as keccak_hash;

declare_id!("21XS8WfVLHccBDZje8V4jE9y4J3n2T42ja7a5Map2ZK8");

/// Configuration constants
const SECRET_MAX_LEN: usize = 64;
const LOTTERY_INFO_DISCRIMINATOR: usize = 8;
const PUBKEY_BYTES: usize = 32;
const HASHLOCK_BYTES: usize = 32;
const U64_BYTES: usize = 8;
const STRING_PREFIX: usize = 4;
const EXTENSION_SECONDS: u64 = 300;

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
        require!(hashlock1 != hashlock2, LotteryError::IdenticalHashlocksNotAllowed);

        let clock = Clock::get()?;
        let current_ts = clock.unix_timestamp as u64;
        let end_reveal = current_ts.checked_add(delay).ok_or(LotteryError::DeadlineOverflow)?;
        require!(end_reveal > current_ts, LotteryError::EndRevealInPast);

        // Transfer 'amount' lamports from both player1 and player2 into the lottery PDA using CPI
        let ix1 = anchor_lang::solana_program::system_instruction::transfer(
            ctx.accounts.player1.to_account_info().key,
            ctx.accounts.lottery_info.to_account_info().key,
            amount,
        );
        anchor_lang::solana_program::program::invoke(
            &ix1,
            &[
                ctx.accounts.player1.to_account_info(),
                ctx.accounts.lottery_info.to_account_info(),
                ctx.accounts.system_program.to_account_info(),
            ],
        )?;

        let ix2 = anchor_lang::solana_program::system_instruction::transfer(
            ctx.accounts.player2.to_account_info().key,
            ctx.accounts.lottery_info.to_account_info().key,
            amount,
        );
        anchor_lang::solana_program::program::invoke(
            &ix2,
            &[
                ctx.accounts.player2.to_account_info(),
                ctx.accounts.lottery_info.to_account_info(),
                ctx.accounts.system_program.to_account_info(),
            ],
        )?;

        // Initialize LotteryInfo fields
        let lottery = &mut ctx.accounts.lottery_info;
        lottery.state = LotteryState::Init;
        lottery.player1 = *ctx.accounts.player1.to_account_info().key;
        lottery.player2 = *ctx.accounts.player2.to_account_info().key;
        lottery.hashlock1 = hashlock1;
        lottery.hashlock2 = hashlock2;
        lottery.secret1 = String::new();
        lottery.secret2 = String::new();
        lottery.end_reveal = end_reveal;

        Ok(())
    }

    pub fn reveal_p1(ctx: Context<RevealP1Ctx>, secret: String) -> Result<()> {
        let clock = Clock::get()?;
        let now = clock.unix_timestamp as u64;
        require!(now <= ctx.accounts.lottery_info.end_reveal, LotteryError::RevealPeriodOver);

        require!(secret.as_bytes().len() <= SECRET_MAX_LEN, LotteryError::SecretTooLong);

        let hash = keccak_hash(secret.as_bytes()).0;
        require!(hash == ctx.accounts.lottery_info.hashlock1, LotteryError::InvalidSecret);

        let lottery = &mut ctx.accounts.lottery_info;
        lottery.secret1 = secret;
        lottery.state = LotteryState::RevealP1;

        Ok(())
    }

    pub fn reveal_p2(ctx: Context<RevealP2Ctx>, secret: String) -> Result<()> {
        let clock = Clock::get()?;
        let now = clock.unix_timestamp as u64;

        require!(
            ctx.accounts.lottery_info.state == LotteryState::RevealP1,
            LotteryError::InvalidStateForRevealP2
        );

        require!(
            now <= ctx.accounts.lottery_info.end_reveal.checked_add(EXTENSION_SECONDS).ok_or(LotteryError::DeadlineOverflow)?,
            LotteryError::RevealPeriodOver
        );

        require!(secret.as_bytes().len() <= SECRET_MAX_LEN, LotteryError::SecretTooLong);

        let hash = keccak_hash(secret.as_bytes()).0;
        require!(hash == ctx.accounts.lottery_info.hashlock2, LotteryError::InvalidSecret);

        // Store secret2 and update state
        let lottery = &mut ctx.accounts.lottery_info;
        lottery.secret2 = secret;
        lottery.state = LotteryState::RevealP2;

        // Determine winner: (secret1.len() + secret2.len()) % 2
        let s1_len = lottery.secret1.as_bytes().len() as u64;
        let s2_len = lottery.secret2.as_bytes().len() as u64;
        let choice = (s1_len + s2_len) % 2;

        let winner_pubkey = if choice == 0 {
            lottery.player1
        } else {
            lottery.player2
        };

        // Transfer entire pot to winner via direct lamports manipulation
        let lottery_ai = ctx.accounts.lottery_info.to_account_info();
        let winner_ai = if winner_pubkey == *ctx.accounts.player1.to_account_info().key {
            ctx.accounts.player1.to_account_info()
        } else {
            ctx.accounts.player2.to_account_info()
        };

        // Safely move lamports
        let mut lottery_lamports = lottery_ai.try_borrow_mut_lamports()?;
        let amount = **lottery_lamports;
        require!(amount > 0, LotteryError::NoFundsInPot);
        **lottery_lamports = 0u64;

        let mut winner_lamports = winner_ai.try_borrow_mut_lamports()?;
        let winner_current = **winner_lamports;
        **winner_lamports = winner_current
            .checked_add(amount)
            .ok_or(LotteryError::LamportArithmeticOverflow)?;

        Ok(())
    }

    pub fn redeem_if_p1_no_reveal(ctx: Context<RedeemIfP1NoRevealCtx>) -> Result<()> {
        let clock = Clock::get()?;
        let now = clock.unix_timestamp as u64;
        require!(now >= ctx.accounts.lottery_info.end_reveal, LotteryError::RevealPeriodNotOver);

        // Ensure player1 didn't reveal
        require!(ctx.accounts.lottery_info.secret1.is_empty(), LotteryError::Player1AlreadyRevealed);

        let lottery_ai = ctx.accounts.lottery_info.to_account_info();
        let player2_ai = ctx.accounts.player2.to_account_info();

        let mut lottery_lamports = lottery_ai.try_borrow_mut_lamports()?;
        let amount = **lottery_lamports;
        require!(amount > 0, LotteryError::NoFundsInPot);
        **lottery_lamports = 0u64;

        let mut player2_lamports = player2_ai.try_borrow_mut_lamports()?;
        let player2_current = **player2_lamports;
        **player2_lamports = player2_current
            .checked_add(amount)
            .ok_or(LotteryError::LamportArithmeticOverflow)?;

        Ok(())
    }

    pub fn redeem_if_p2_no_reveal(ctx: Context<RedeemIfP2NoRevealCtx>) -> Result<()> {
        let clock = Clock::get()?;
        let now = clock.unix_timestamp as u64;

        require!(
            now >= ctx.accounts.lottery_info.end_reveal.checked_add(EXTENSION_SECONDS).ok_or(LotteryError::DeadlineOverflow)?,
            LotteryError::RevealPeriodNotOver
        );

        // Ensure player2 didn't reveal
        require!(ctx.accounts.lottery_info.secret2.is_empty(), LotteryError::Player2AlreadyRevealed);

        let lottery_ai = ctx.accounts.lottery_info.to_account_info();
        let player1_ai = ctx.accounts.player1.to_account_info();

        let mut lottery_lamports = lottery_ai.try_borrow_mut_lamports()?;
        let amount = **lottery_lamports;
        require!(amount > 0, LotteryError::NoFundsInPot);
        **lottery_lamports = 0u64;

        let mut player1_lamports = player1_ai.try_borrow_mut_lamports()?;
        let player1_current = **player1_lamports;
        **player1_lamports = player1_current
            .checked_add(amount)
            .ok_or(LotteryError::LamportArithmeticOverflow)?;

        Ok(())
    }
}

#[derive(Accounts)]
#[instruction(hashlock1: [u8;32], hashlock2: [u8;32], delay: u64, amount: u64)]
pub struct JoinCtx<'info> {
    /// Player1 must sign and pays for account creation
    #[account(mut)]
    pub player1: Signer<'info>,

    /// Player2 must sign
    #[account(mut)]
    pub player2: Signer<'info>,

    #[account(
        init,
        seeds = [player1.key().as_ref(), player2.key().as_ref()],
        bump,
        payer = player1,
        space = LOTTERY_INFO_DISCRIMINATOR + LotteryInfo::MAX_SIZE
    )]
    pub lottery_info: Account<'info, LotteryInfo>,

    pub system_program: Program<'info, System>,
}

#[derive(Accounts)]
pub struct RevealP1Ctx<'info> {
    #[account(mut)]
    pub player1: Signer<'info>,

    /// CHECK: player2 is used only for PDA seed verification; mark mut if you plan to transfer to it
    pub player2: UncheckedAccount<'info>,

    #[account(
        mut,
        seeds = [player1.key().as_ref(), player2.key().as_ref()],
        bump,
        has_one = player1,
        has_one = player2,
    )]
    pub lottery_info: Account<'info, LotteryInfo>,
}

#[derive(Accounts)]
pub struct RevealP2Ctx<'info> {
    /// CHECK: player1 used only for PDA seed verification, but lamports may be credited so mark mut
    #[account(mut)]
    pub player1: UncheckedAccount<'info>,

    #[account(mut)]
    pub player2: Signer<'info>,

    #[account(
        mut,
        seeds = [player1.key().as_ref(), player2.key().as_ref()],
        bump,
        has_one = player1,
        has_one = player2,
    )]
    pub lottery_info: Account<'info, LotteryInfo>,
}

#[derive(Accounts)]
pub struct RedeemIfP1NoRevealCtx<'info> {
    /// CHECK: player1 used only for PDA seed verification, lamports not changed here but keep as reference
    pub player1: UncheckedAccount<'info>,

    #[account(mut)]
    pub player2: Signer<'info>,

    #[account(
        mut,
        seeds = [player1.key().as_ref(), player2.key().as_ref()],
        bump,
        has_one = player1,
        has_one = player2,
    )]
    pub lottery_info: Account<'info, LotteryInfo>,
}
#[derive(Accounts)]
pub struct RedeemIfP2NoRevealCtx<'info> {
    #[account(mut)]
    pub player1: Signer<'info>,

    /// CHECK: player2 used only for PDA seed verification, may be read-only if not credited
    pub player2: UncheckedAccount<'info>,

    #[account(
        mut,
        seeds = [player1.key().as_ref(), player2.key().as_ref()],
        bump,
        has_one = player1,
        has_one = player2,
    )]
    pub lottery_info: Account<'info, LotteryInfo>,
}
#[account]
pub struct LotteryInfo {
    pub state: LotteryState,
    pub player1: Pubkey,
    pub player2: Pubkey,
    pub hashlock1: [u8; 32],
    pub secret1: String,
    pub hashlock2: [u8; 32],
    pub secret2: String,
    pub end_reveal: u64,
}

impl LotteryInfo {
    pub const MAX_SIZE: usize = 1 // state as u8
        + PUBKEY_BYTES
        + PUBKEY_BYTES
        + HASHLOCK_BYTES
        + (STRING_PREFIX + SECRET_MAX_LEN)
        + HASHLOCK_BYTES
        + (STRING_PREFIX + SECRET_MAX_LEN)
        + U64_BYTES;
}

#[derive(AnchorSerialize, AnchorDeserialize, Clone, PartialEq, Debug)]
pub enum LotteryState {
    Init = 0,
    RevealP1 = 1,
    RevealP2 = 2,
}

#[error_code]
pub enum LotteryError {
    #[msg("Players provided identical hash commitments which is not allowed")]
    IdenticalHashlocksNotAllowed,
    #[msg("End reveal would be in the past")]
    EndRevealInPast,
    #[msg("Deadline arithmetic overflow")]
    DeadlineOverflow,
    #[msg("Reveal period is over")]
    RevealPeriodOver,
    #[msg("Secret provided does not match the hashlock")]
    InvalidSecret,
    #[msg("Secret exceeds maximum allowed length")]
    SecretTooLong,
    #[msg("Invalid state for RevealP2; player1 must reveal first")]
    InvalidStateForRevealP2,
    #[msg("No funds in the pot to transfer")]
    NoFundsInPot,
    #[msg("Lamport arithmetic underflow")]
    LamportArithmeticUnderflow,
    #[msg("Lamport arithmetic overflow")]
    LamportArithmeticOverflow,
    #[msg("Reveal period not over yet")]
    RevealPeriodNotOver,
    #[msg("Player1 already revealed")]
    Player1AlreadyRevealed,
    #[msg("Player2 already revealed")]
    Player2AlreadyRevealed,
}
