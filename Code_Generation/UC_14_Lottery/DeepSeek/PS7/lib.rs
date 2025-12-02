use anchor_lang::prelude::*;
use anchor_lang::system_program;
use borsh::{BorshDeserialize, BorshSerialize};

declare_id!("9ms3t3C9WsUDt5dZ6V37jRrqqKrnzz5cNr9GBU6LoNUj");

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
        // Validate inputs
        require!(hashlock1 != hashlock2, LotteryError::DuplicateCommitments);
        require!(delay > 0, LotteryError::InvalidDelay);
        require!(amount > 0, LotteryError::InvalidAmount);

        let clock = Clock::get()?;
        let end_reveal = clock.unix_timestamp as u64 + delay;
        require!(end_reveal > clock.unix_timestamp as u64, LotteryError::InvalidRevealTime);

        // Transfer funds from both players
        let player1_ix = system_program::Transfer {
            from: ctx.accounts.player1.to_account_info(),
            to: ctx.accounts.lottery_info.to_account_info(),
        };
        let player1_cpi = CpiContext::new(
            ctx.accounts.system_program.to_account_info(),
            player1_ix,
        );
        system_program::transfer(player1_cpi, amount)?;

        let player2_ix = system_program::Transfer {
            from: ctx.accounts.player2.to_account_info(),
            to: ctx.accounts.lottery_info.to_account_info(),
        };
        let player2_cpi = CpiContext::new(
            ctx.accounts.system_program.to_account_info(),
            player2_ix,
        );
        system_program::transfer(player2_cpi, amount)?;

        // Initialize lottery state
        let lottery_info = &mut ctx.accounts.lottery_info;
        lottery_info.state = LotteryState::Init;
        lottery_info.player1 = ctx.accounts.player1.key();
        lottery_info.player2 = ctx.accounts.player2.key();
        lottery_info.hashlock1 = hashlock1;
        lottery_info.hashlock2 = hashlock2;
        lottery_info.secret1 = String::new();
        lottery_info.secret2 = String::new();
        lottery_info.end_reveal = end_reveal;

        Ok(())
    }

    pub fn reveal_p1(ctx: Context<RevealP1Ctx>, secret: String) -> Result<()> {
        let lottery_info = &mut ctx.accounts.lottery_info;
        
        // Validate timing
        let clock = Clock::get()?;
        require!(
            clock.unix_timestamp as u64 <= lottery_info.end_reveal,
            LotteryError::RevealPeriodEnded
        );

        // Validate secret matches commitment
        let hash = keccak256_hash(secret.as_bytes());
        require!(
            hash == lottery_info.hashlock1,
            LotteryError::InvalidSecret
        );

        // Update state
        lottery_info.state = LotteryState::RevealP1;
        lottery_info.secret1 = secret;

        Ok(())
    }

    pub fn reveal_p2(ctx: Context<RevealP2Ctx>, secret: String) -> Result<()> {
        let lottery_info = &mut ctx.accounts.lottery_info;
        
        // Validate state
        require!(
            lottery_info.state == LotteryState::RevealP1,
            LotteryError::InvalidStateTransition
        );

        // Validate timing (extended period)
        let clock = Clock::get()?;
        let extended_deadline = lottery_info.end_reveal + 300; // 5-minute extension
        require!(
            clock.unix_timestamp as u64 <= extended_deadline,
            LotteryError::RevealPeriodEnded
        );

        // Validate secret matches commitment
        let hash = keccak256_hash(secret.as_bytes());
        require!(
            hash == lottery_info.hashlock2,
            LotteryError::InvalidSecret
        );

        // Update state and store secret
        lottery_info.state = LotteryState::RevealP2;
        lottery_info.secret2 = secret;

        // Determine winner and distribute funds
        let winner = determine_winner(&lottery_info.secret1, &lottery_info.secret2);
        distribute_prize(
            &ctx.accounts.lottery_info.to_account_info(),
            &ctx.accounts.player1.to_account_info(),
            &ctx.accounts.player2.to_account_info(),
            winner,
        )
    }

    pub fn redeem_if_p1_no_reveal(ctx: Context<RedeemIfP1NoRevealCtx>) -> Result<()> {
        let lottery_info = &ctx.accounts.lottery_info;
        let clock = Clock::get()?;

        // Validate Player1 didn't reveal and deadline passed
        require!(
            lottery_info.state == LotteryState::Init,
            LotteryError::PlayerAlreadyRevealed
        );
        require!(
            clock.unix_timestamp as u64 > lottery_info.end_reveal,
            LotteryError::RevealPeriodNotEnded
        );

        // Transfer to Player2 as penalty
        distribute_penalty(
            &ctx.accounts.lottery_info.to_account_info(),
            &ctx.accounts.player2.to_account_info(),
        )
    }

    pub fn redeem_if_p2_no_reveal(ctx: Context<RedeemIfP2NoRevealCtx>) -> Result<()> {
        let lottery_info = &ctx.accounts.lottery_info;
        let clock = Clock::get()?;

        // Validate Player2 didn't reveal and extended deadline passed
        require!(
            lottery_info.state == LotteryState::RevealP1,
            LotteryError::PlayerAlreadyRevealed
        );
        let extended_deadline = lottery_info.end_reveal + 300;
        require!(
            clock.unix_timestamp as u64 > extended_deadline,
            LotteryError::RevealPeriodNotEnded
        );

        // Transfer to Player1 as penalty
        distribute_penalty(
            &ctx.accounts.lottery_info.to_account_info(),
            &ctx.accounts.player1.to_account_info(),
        )
    }
}

// Helper functions
fn keccak256_hash(input: &[u8]) -> [u8; 32] {
    use anchor_lang::solana_program::keccak::hash;
    hash(input).0
}

fn determine_winner(secret1: &str, secret2: &str) -> Winner {
    let result = (secret1.len() + secret2.len()) % 2;
    if result == 0 {
        Winner::Player1
    } else {
        Winner::Player2
    }
}

fn distribute_prize(
    lottery_account: &AccountInfo,
    player1: &AccountInfo,
    player2: &AccountInfo,
    winner: Winner,
) -> Result<()> {
    let lamports = lottery_account.lamports();
    
    // Zero out lottery account lamports
    **lottery_account.try_borrow_mut_lamports()? = 0;
    
    // Transfer to winner - ensure accounts are mutable
    match winner {
        Winner::Player1 => {
            **player1.try_borrow_mut_lamports()? += lamports;
        }
        Winner::Player2 => {
            **player2.try_borrow_mut_lamports()? += lamports;
        }
    }
    
    Ok(())
}

fn distribute_penalty(lottery_account: &AccountInfo, recipient: &AccountInfo) -> Result<()> {
    let lamports = lottery_account.lamports();
    **lottery_account.try_borrow_mut_lamports()? = 0;
    **recipient.try_borrow_mut_lamports()? += lamports;
    Ok(())
}

// Context Definitions - FIXED: Made player accounts mutable where needed
#[derive(Accounts)]
pub struct JoinCtx<'info> {
    #[account(mut)]
    pub player1: Signer<'info>,
    #[account(mut)]
    pub player2: Signer<'info>,
    #[account(
        init,
        payer = player1,
        space = LotteryInfo::LEN,
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
    /// CHECK: This is not mutable or signed, just for verification
    pub player2: AccountInfo<'info>,
    #[account(
        mut,
        seeds = [player1.key().as_ref(), player2.key().as_ref()],
        bump,
        has_one = player1,
        has_one = player2
    )]
    pub lottery_info: Account<'info, LotteryInfo>,
}

#[derive(Accounts)]
pub struct RevealP2Ctx<'info> {
    /// CHECK: This account needs to be mutable to receive funds
    #[account(mut)]
    pub player1: AccountInfo<'info>,
    #[account(mut)]
    pub player2: Signer<'info>,
    #[account(
        mut,
        seeds = [player1.key().as_ref(), player2.key().as_ref()],
        bump,
        has_one = player1,
        has_one = player2
    )]
    pub lottery_info: Account<'info, LotteryInfo>,
}

#[derive(Accounts)]
pub struct RedeemIfP1NoRevealCtx<'info> {
    /// CHECK: This is not mutable or signed, just for verification
    pub player1: AccountInfo<'info>,
    #[account(mut)]
    pub player2: Signer<'info>,
    #[account(
        mut,
        seeds = [player1.key().as_ref(), player2.key().as_ref()],
        bump,
        has_one = player1,
        has_one = player2
    )]
    pub lottery_info: Account<'info, LotteryInfo>,
}

#[derive(Accounts)]
pub struct RedeemIfP2NoRevealCtx<'info> {
    #[account(mut)]
    pub player1: Signer<'info>,
    /// CHECK: This is not mutable or signed, just for verification
    pub player2: AccountInfo<'info>,
    #[account(
        mut,
        seeds = [player1.key().as_ref(), player2.key().as_ref()],
        bump,
        has_one = player1,
        has_one = player2
    )]
    pub lottery_info: Account<'info, LotteryInfo>,
}

// Data Structures
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
    pub const LEN: usize = 8 + // discriminator
        1 + // state
        32 + // player1
        32 + // player2
        32 + // hashlock1
        (4 + 256) + // secret1 (String max length)
        32 + // hashlock2
        (4 + 256) + // secret2 (String max length)
        8; // end_reveal
}

#[derive(AnchorSerialize, AnchorDeserialize, Clone, PartialEq, Eq)]
pub enum LotteryState {
    Init = 0,
    RevealP1 = 1,
    RevealP2 = 2,
}

enum Winner {
    Player1,
    Player2,
}

// Error Handling
#[error_code]
pub enum LotteryError {
    #[msg("Duplicate hash commitments are not allowed")]
    DuplicateCommitments,
    #[msg("Invalid delay period")]
    InvalidDelay,
    #[msg("Invalid amount")]
    InvalidAmount,
    #[msg("Reveal period has ended")]
    RevealPeriodEnded,
    #[msg("Reveal period has not ended yet")]
    RevealPeriodNotEnded,
    #[msg("Invalid secret does not match commitment")]
    InvalidSecret,
    #[msg("Invalid state transition")]
    InvalidStateTransition,
    #[msg("Player has already revealed their secret")]
    PlayerAlreadyRevealed,
    #[msg("Invalid reveal time")]
    InvalidRevealTime,
}