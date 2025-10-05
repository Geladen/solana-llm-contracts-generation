use anchor_lang::prelude::*;
use anchor_lang::system_program;

declare_id!("3WHmDD6PWgyTYUgrXt9RHQNmeEEYJjXMa4YVFJkCJHd2");

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
        let clock = Clock::get()?;

        // Validate inputs
        require!(hashlock1 != hashlock2, LotteryError::DuplicateHashes);
        require!(delay > 0, LotteryError::InvalidDelay);
        require!(amount > 0, LotteryError::InvalidAmount);

        let end_reveal = clock.slot + delay;
        require!(end_reveal > clock.slot, LotteryError::InvalidEndReveal);

        // Transfer funds from both players
        let cpi_program = ctx.accounts.system_program.to_account_info();
        
        // Transfer from player1
        let cpi_accounts = system_program::Transfer {
            from: ctx.accounts.player1.to_account_info(),
            to: ctx.accounts.lottery_info.to_account_info(),
        };
        let cpi_ctx = CpiContext::new(cpi_program.clone(), cpi_accounts);
        system_program::transfer(cpi_ctx, amount)?;

        // Transfer from player2
        let cpi_accounts = system_program::Transfer {
            from: ctx.accounts.player2.to_account_info(),
            to: ctx.accounts.lottery_info.to_account_info(),
        };
        let cpi_ctx = CpiContext::new(cpi_program, cpi_accounts);
        system_program::transfer(cpi_ctx, amount)?;

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

        emit!(LotteryJoined {
            player1: lottery_info.player1,
            player2: lottery_info.player2,
            amount,
            end_reveal
        });

        Ok(())
    }

    pub fn reveal_p1(ctx: Context<RevealP1Ctx>, secret: String) -> Result<()> {
        let lottery_info = &mut ctx.accounts.lottery_info;
        let clock = Clock::get()?;

        // Validate state and timing
        require!(
            lottery_info.state == LotteryState::Init,
            LotteryError::InvalidState
        );
        require!(
            clock.slot <= lottery_info.end_reveal,
            LotteryError::RevealPeriodEnded
        );

        // Validate secret matches commitment using Keccak-256
        let computed_hash = keccak_hash(&secret);
        require!(
            computed_hash == lottery_info.hashlock1,
            LotteryError::InvalidSecret
        );

        // Update state
        lottery_info.state = LotteryState::RevealP1;
        lottery_info.secret1 = secret;

        emit!(PlayerRevealed {
            player: ctx.accounts.player1.key(),
            state: lottery_info.state
        });

        Ok(())
    }

    pub fn reveal_p2(ctx: Context<RevealP2Ctx>, secret: String) -> Result<()> {
        let lottery_info = &mut ctx.accounts.lottery_info;
        let clock = Clock::get()?;

        // Validate state and timing
        require!(
            lottery_info.state == LotteryState::RevealP1,
            LotteryError::InvalidState
        );
        require!(
            clock.slot <= lottery_info.end_reveal,
            LotteryError::RevealPeriodEnded
        );

        // Validate secret matches commitment
        let computed_hash = keccak_hash(&secret);
        require!(
            computed_hash == lottery_info.hashlock2,
            LotteryError::InvalidSecret
        );

        // Update state and store secret
        lottery_info.state = LotteryState::RevealP2;
        lottery_info.secret2 = secret.clone();

        // Extract data needed for winner determination before transfer
        let secret1_len = lottery_info.secret1.len();
        let secret2_len = lottery_info.secret2.len();
        let player1_pubkey = lottery_info.player1;
        let player2_pubkey = lottery_info.player2;

        // Determine winner fairly
        let winner = determine_winner(secret1_len, secret2_len);
        
        // Transfer funds to winner
        let lottery_balance = ctx.accounts.lottery_info.to_account_info().lamports();
        
        if winner == 0 {
            // Transfer to player1
            **ctx.accounts.lottery_info.to_account_info().try_borrow_mut_lamports()? = 0;
            **ctx.accounts.player1.try_borrow_mut_lamports()? += lottery_balance;
        } else {
            // Transfer to player2  
            **ctx.accounts.lottery_info.to_account_info().try_borrow_mut_lamports()? = 0;
            **ctx.accounts.player2.try_borrow_mut_lamports()? += lottery_balance;
        }

        emit!(LotteryCompleted {
            winner: if winner == 0 { player1_pubkey } else { player2_pubkey },
            player1_secret_len: secret1_len as u32,
            player2_secret_len: secret2_len as u32,
            winning_computation: winner
        });

        Ok(())
    }

    pub fn redeem_if_p1_no_reveal(ctx: Context<RedeemIfP1NoRevealCtx>) -> Result<()> {
        let clock = Clock::get()?;
        
        // Validate conditions for redemption without mutable borrow first
        {
            let lottery_info = &ctx.accounts.lottery_info;
            require!(
                lottery_info.state == LotteryState::Init,
                LotteryError::InvalidState
            );
            require!(
                clock.slot > lottery_info.end_reveal,
                LotteryError::RevealPeriodNotEnded
            );
        }

        // Transfer entire pot to player2 as penalty
        let lottery_balance = ctx.accounts.lottery_info.to_account_info().lamports();
        
        **ctx.accounts.lottery_info.to_account_info().try_borrow_mut_lamports()? = 0;
        **ctx.accounts.player2.try_borrow_mut_lamports()? += lottery_balance;

        emit!(PenaltyRedeemed {
            redeemer: ctx.accounts.player2.key(),
            amount: lottery_balance,
            reason: "Player1 failed to reveal".to_string()
        });

        Ok(())
    }

    pub fn redeem_if_p2_no_reveal(ctx: Context<RedeemIfP2NoRevealCtx>) -> Result<()> {
        let clock = Clock::get()?;
        
        // Validate conditions for redemption without mutable borrow first
        {
            let lottery_info = &ctx.accounts.lottery_info;
            require!(
                lottery_info.state == LotteryState::RevealP1,
                LotteryError::InvalidState
            );
            require!(
                clock.slot > lottery_info.end_reveal,
                LotteryError::RevealPeriodNotEnded
            );
        }

        // Transfer entire pot to player1 as penalty
        let lottery_balance = ctx.accounts.lottery_info.to_account_info().lamports();
        
        **ctx.accounts.lottery_info.to_account_info().try_borrow_mut_lamports()? = 0;
        **ctx.accounts.player1.try_borrow_mut_lamports()? += lottery_balance;

        emit!(PenaltyRedeemed {
            redeemer: ctx.accounts.player1.key(),
            amount: lottery_balance,
            reason: "Player2 failed to reveal".to_string()
        });

        Ok(())
    }
}

// Helper function for keccak hash - using Solana's keccak implementation
fn keccak_hash(input: &str) -> [u8; 32] {
    use anchor_lang::solana_program::keccak::hashv;
    let hash = hashv(&[input.as_bytes()]);
    hash.to_bytes()
}

// Helper function for fair winner determination
fn determine_winner(secret1_len: usize, secret2_len: usize) -> u8 {
    ((secret1_len + secret2_len) % 2) as u8
}

// Context Definitions
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
    /// CHECK: This is not mutable or signed, just for reference to validate PDA seeds
    pub player2: AccountInfo<'info>,
    #[account(
        mut,
        seeds = [player1.key().as_ref(), player2.key().as_ref()],
        bump
    )]
    pub lottery_info: Account<'info, LotteryInfo>,
}

#[derive(Accounts)]
pub struct RevealP2Ctx<'info> {
    /// CHECK: This account is mutable because it might receive funds if player1 wins
    #[account(mut)]
    pub player1: AccountInfo<'info>,
    #[account(mut)]
    pub player2: Signer<'info>,
    #[account(
        mut,
        seeds = [player1.key().as_ref(), player2.key().as_ref()],
        bump
    )]
    pub lottery_info: Account<'info, LotteryInfo>,
}

#[derive(Accounts)]
pub struct RedeemIfP1NoRevealCtx<'info> {
    /// CHECK: This is not mutable or signed, just for reference to validate PDA seeds
    pub player1: AccountInfo<'info>,
    #[account(mut)]
    pub player2: Signer<'info>,
    #[account(
        mut,
        seeds = [player1.key().as_ref(), player2.key().as_ref()],
        bump
    )]
    pub lottery_info: Account<'info, LotteryInfo>,
}

#[derive(Accounts)]
pub struct RedeemIfP2NoRevealCtx<'info> {
    #[account(mut)]
    pub player1: Signer<'info>,
    /// CHECK: This is not mutable or signed, just for reference to validate PDA seeds
    pub player2: AccountInfo<'info>,
    #[account(
        mut,
        seeds = [player1.key().as_ref(), player2.key().as_ref()],
        bump
    )]
    pub lottery_info: Account<'info, LotteryInfo>,
}

// Data Structures
#[account]
pub struct LotteryInfo {
    pub state: u8,           // LotteryState as u8
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
        (4 + 100) + // secret1 (max 100 chars)
        32 + // hashlock2
        (4 + 100) + // secret2 (max 100 chars)
        8; // end_reveal
}

// LotteryState constants
pub struct LotteryState;

impl LotteryState {
    pub const Init: u8 = 0;
    pub const RevealP1: u8 = 1;
    pub const RevealP2: u8 = 2;
}

// Error Codes
#[error_code]
pub enum LotteryError {
    #[msg("Duplicate hash commitments are not allowed")]
    DuplicateHashes,
    #[msg("Invalid delay value")]
    InvalidDelay,
    #[msg("Invalid amount")]
    InvalidAmount,
    #[msg("Invalid end reveal time")]
    InvalidEndReveal,
    #[msg("Invalid lottery state for this operation")]
    InvalidState,
    #[msg("Reveal period has ended")]
    RevealPeriodEnded,
    #[msg("Reveal period has not ended yet")]
    RevealPeriodNotEnded,
    #[msg("Provided secret does not match the commitment")]
    InvalidSecret,
    #[msg("Math overflow occurred")]
    MathOverflow,
}

// Events
#[event]
pub struct LotteryJoined {
    pub player1: Pubkey,
    pub player2: Pubkey,
    pub amount: u64,
    pub end_reveal: u64,
}

#[event]
pub struct PlayerRevealed {
    pub player: Pubkey,
    pub state: u8,
}

#[event]
pub struct LotteryCompleted {
    pub winner: Pubkey,
    pub player1_secret_len: u32,
    pub player2_secret_len: u32,
    pub winning_computation: u8,
}

#[event]
pub struct PenaltyRedeemed {
    pub redeemer: Pubkey,
    pub amount: u64,
    pub reason: String,
}