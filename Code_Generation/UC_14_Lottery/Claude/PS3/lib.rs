use anchor_lang::prelude::*;
use anchor_lang::system_program;
use anchor_lang::solana_program::keccak;

declare_id!("GBrUZ9UXGT85D5vmfLsnEXa11HekBJXkRvCfuhg6je6x");

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

        // Transfer from player1 to lottery PDA
        system_program::transfer(
            CpiContext::new(
                ctx.accounts.system_program.to_account_info(),
                system_program::Transfer {
                    from: ctx.accounts.player1.to_account_info(),
                    to: ctx.accounts.lottery_info.to_account_info(),
                },
            ),
            amount,
        )?;

        // Transfer from player2 to lottery PDA
        system_program::transfer(
            CpiContext::new(
                ctx.accounts.system_program.to_account_info(),
                system_program::Transfer {
                    from: ctx.accounts.player2.to_account_info(),
                    to: ctx.accounts.lottery_info.to_account_info(),
                },
            ),
            amount,
        )?;

        // Initialize lottery state
        let lottery_info = &mut ctx.accounts.lottery_info;
        lottery_info.state = LotteryState::Init;
        lottery_info.player1 = ctx.accounts.player1.key();
        lottery_info.player2 = ctx.accounts.player2.key();
        lottery_info.hashlock1 = hashlock1;
        lottery_info.secret1 = String::new();
        lottery_info.hashlock2 = hashlock2;
        lottery_info.secret2 = String::new();
        lottery_info.end_reveal = end_reveal;

        Ok(())
    }

    pub fn reveal_p1(ctx: Context<RevealP1Ctx>, secret: String) -> Result<()> {
        let lottery_info = &mut ctx.accounts.lottery_info;

        // Check state
        require!(
            lottery_info.state == LotteryState::Init,
            LotteryError::InvalidState
        );

        // Check deadline
        let clock = Clock::get()?;
        require!(
            (clock.unix_timestamp as u64) < lottery_info.end_reveal,
            LotteryError::DeadlinePassed
        );

        // Validate secret matches hashlock1 using Keccak-256
        let hash = keccak::hash(secret.as_bytes());
        require!(
            hash.to_bytes() == lottery_info.hashlock1,
            LotteryError::InvalidSecret
        );

        // Update state
        lottery_info.state = LotteryState::RevealP1;
        lottery_info.secret1 = secret;

        Ok(())
    }

    pub fn reveal_p2(ctx: Context<RevealP2Ctx>, secret: String) -> Result<()> {
        // Check state
        require!(
            ctx.accounts.lottery_info.state == LotteryState::RevealP1,
            LotteryError::InvalidState
        );

        // Check deadline (with extension - player2 gets extra time)
        let clock = Clock::get()?;
        let extended_deadline = ctx.accounts.lottery_info.end_reveal + ctx.accounts.lottery_info.end_reveal;
        require!(
            (clock.unix_timestamp as u64) < extended_deadline,
            LotteryError::DeadlinePassed
        );

        // Validate secret matches hashlock2 using Keccak-256
        let hash = keccak::hash(secret.as_bytes());
        require!(
            hash.to_bytes() == ctx.accounts.lottery_info.hashlock2,
            LotteryError::InvalidSecret
        );

        // Determine winner using fair function: (secret1.len() + secret2.len()) % 2
        let total_len = ctx.accounts.lottery_info.secret1.len() + secret.len();
        let winner_is_p1 = (total_len % 2) == 0;

        // Get lottery PDA balance
        let lottery_balance = ctx.accounts.lottery_info.to_account_info().lamports();

        // Transfer entire pot to winner via direct lamports manipulation
        if winner_is_p1 {
            **ctx.accounts.lottery_info.to_account_info().try_borrow_mut_lamports()? -= lottery_balance;
            **ctx.accounts.player1.to_account_info().try_borrow_mut_lamports()? += lottery_balance;
        } else {
            **ctx.accounts.lottery_info.to_account_info().try_borrow_mut_lamports()? -= lottery_balance;
            **ctx.accounts.player2.to_account_info().try_borrow_mut_lamports()? += lottery_balance;
        }

        // Store revealed secret and update state
        let lottery_info = &mut ctx.accounts.lottery_info;
        lottery_info.secret2 = secret;
        lottery_info.state = LotteryState::RevealP2;

        Ok(())
    }

    pub fn redeem_if_p1_no_reveal(ctx: Context<RedeemIfP1NoRevealCtx>) -> Result<()> {
        // Check state - Player1 should not have revealed
        require!(
            ctx.accounts.lottery_info.state == LotteryState::Init,
            LotteryError::InvalidState
        );

        // Check deadline has passed
        let clock = Clock::get()?;
        require!(
            (clock.unix_timestamp as u64) >= ctx.accounts.lottery_info.end_reveal,
            LotteryError::DeadlineNotPassed
        );

        // Get lottery PDA balance
        let lottery_balance = ctx.accounts.lottery_info.to_account_info().lamports();

        // Transfer entire pot to Player2 as penalty
        **ctx.accounts.lottery_info.to_account_info().try_borrow_mut_lamports()? -= lottery_balance;
        **ctx.accounts.player2.to_account_info().try_borrow_mut_lamports()? += lottery_balance;

        Ok(())
    }

    pub fn redeem_if_p2_no_reveal(ctx: Context<RedeemIfP2NoRevealCtx>) -> Result<()> {
        // Check state - Player1 revealed but Player2 didn't
        require!(
            ctx.accounts.lottery_info.state == LotteryState::RevealP1,
            LotteryError::InvalidState
        );

        // Check deadline + extension has passed
        let clock = Clock::get()?;
        let extended_deadline = ctx.accounts.lottery_info.end_reveal + ctx.accounts.lottery_info.end_reveal;
        require!(
            (clock.unix_timestamp as u64) >= extended_deadline,
            LotteryError::DeadlineNotPassed
        );

        // Get lottery PDA balance
        let lottery_balance = ctx.accounts.lottery_info.to_account_info().lamports();

        // Transfer entire pot to Player1 as penalty
        **ctx.accounts.lottery_info.to_account_info().try_borrow_mut_lamports()? -= lottery_balance;
        **ctx.accounts.player1.to_account_info().try_borrow_mut_lamports()? += lottery_balance;

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
    
    /// CHECK: Player2 reference for validation
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
    /// CHECK: Player1 reference for validation
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
    /// CHECK: Player1 reference for validation
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
    
    /// CHECK: Player2 reference for validation
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
    #[max_len(100)]
    pub secret1: String,
    pub hashlock2: [u8; 32],
    #[max_len(100)]
    pub secret2: String,
    pub end_reveal: u64,
}

#[derive(AnchorSerialize, AnchorDeserialize, Clone, Copy, PartialEq, Eq, InitSpace)]
pub enum LotteryState {
    Init = 0,
    RevealP1 = 1,
    RevealP2 = 2,
}

// Error Codes

#[error_code]
pub enum LotteryError {
    #[msg("Hash commitments cannot be identical")]
    IdenticalHashlocks,
    
    #[msg("Deadline cannot be in the past")]
    DeadlineInPast,
    
    #[msg("Invalid lottery state for this operation")]
    InvalidState,
    
    #[msg("Deadline has passed")]
    DeadlinePassed,
    
    #[msg("Invalid secret - does not match commitment")]
    InvalidSecret,
    
    #[msg("Deadline has not passed yet")]
    DeadlineNotPassed,
    
    #[msg("Invalid player for this lottery")]
    InvalidPlayer,
}