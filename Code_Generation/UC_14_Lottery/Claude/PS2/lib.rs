use anchor_lang::prelude::*;
use anchor_lang::solana_program::{keccak, system_instruction};
use anchor_lang::system_program;

declare_id!("FicibChW4Zhn2sf7ayutqMeajhSoGaMrRwrLYNVXeU9q");

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
        let lottery = &mut ctx.accounts.lottery_info;
        let clock = Clock::get()?;

        // Reject identical hash commitments
        require!(hashlock1 != hashlock2, LotteryError::IdenticalHashlocks);

        // Calculate end_reveal timestamp
        let end_reveal = clock.unix_timestamp as u64 + delay;
        
        // Reject end_reveal in the past
        require!(
            end_reveal > clock.unix_timestamp as u64,
            LotteryError::EndRevealInPast
        );

        // Transfer from player1 to lottery PDA
        let transfer_ix_p1 = system_instruction::transfer(
            &ctx.accounts.player1.key(),
            &lottery.key(),
            amount,
        );
        anchor_lang::solana_program::program::invoke(
            &transfer_ix_p1,
            &[
                ctx.accounts.player1.to_account_info(),
                lottery.to_account_info(),
                ctx.accounts.system_program.to_account_info(),
            ],
        )?;

        // Transfer from player2 to lottery PDA
        let transfer_ix_p2 = system_instruction::transfer(
            &ctx.accounts.player2.key(),
            &lottery.key(),
            amount,
        );
        anchor_lang::solana_program::program::invoke(
            &transfer_ix_p2,
            &[
                ctx.accounts.player2.to_account_info(),
                lottery.to_account_info(),
                ctx.accounts.system_program.to_account_info(),
            ],
        )?;

        // Initialize lottery state
        lottery.state = LotteryState::Init;
        lottery.player1 = ctx.accounts.player1.key();
        lottery.player2 = ctx.accounts.player2.key();
        lottery.hashlock1 = hashlock1;
        lottery.secret1 = String::new();
        lottery.hashlock2 = hashlock2;
        lottery.secret2 = String::new();
        lottery.end_reveal = end_reveal;

        Ok(())
    }

    pub fn reveal_p1(ctx: Context<RevealP1Ctx>, secret: String) -> Result<()> {
        let lottery = &mut ctx.accounts.lottery_info;
        let clock = Clock::get()?;

        // Check state is Init
        require!(
            lottery.state == LotteryState::Init,
            LotteryError::InvalidState
        );

        // Check deadline hasn't passed
        require!(
            (clock.unix_timestamp as u64) <= lottery.end_reveal,
            LotteryError::RevealDeadlinePassed
        );

        // Validate secret matches hashlock1 using Keccak-256
        let hash = keccak::hash(secret.as_bytes());
        require!(
            hash.to_bytes() == lottery.hashlock1,
            LotteryError::InvalidSecret
        );

        // Update state and store secret
        lottery.state = LotteryState::RevealP1;
        lottery.secret1 = secret;

        Ok(())
    }

    pub fn reveal_p2(ctx: Context<RevealP2Ctx>, secret: String) -> Result<()> {
        let lottery = &mut ctx.accounts.lottery_info;
        let clock = Clock::get()?;

        // Check state is RevealP1
        require!(
            lottery.state == LotteryState::RevealP1,
            LotteryError::InvalidState
        );

        // Check deadline hasn't passed (Player2 has same deadline as Player1)
        require!(
            clock.unix_timestamp as u64 <= lottery.end_reveal,
            LotteryError::RevealDeadlinePassed
        );

        // Validate secret matches hashlock2 using Keccak-256
        let hash = keccak::hash(secret.as_bytes());
        require!(
            hash.to_bytes() == lottery.hashlock2,
            LotteryError::InvalidSecret
        );

        // Update state and store secret
        lottery.state = LotteryState::RevealP2;
        lottery.secret2 = secret;

        // Determine winner using fair function
        let sum = lottery.secret1.len() + lottery.secret2.len();
        let winner_is_p1 = sum % 2 == 0;

        // Get total pot
        let lottery_lamports = lottery.to_account_info().lamports();
        let rent_exempt = Rent::get()?.minimum_balance(lottery.to_account_info().data_len());
        let pot = lottery_lamports.checked_sub(rent_exempt).unwrap_or(0);

        // Transfer entire pot to winner via direct lamports manipulation
        if winner_is_p1 {
            **lottery.to_account_info().try_borrow_mut_lamports()? -= pot;
            **ctx.accounts.player1.to_account_info().try_borrow_mut_lamports()? += pot;
        } else {
            **lottery.to_account_info().try_borrow_mut_lamports()? -= pot;
            **ctx.accounts.player2.to_account_info().try_borrow_mut_lamports()? += pot;
        }

        Ok(())
    }

    pub fn redeem_if_p1_no_reveal(ctx: Context<RedeemIfP1NoRevealCtx>) -> Result<()> {
        let lottery = &mut ctx.accounts.lottery_info;
        let clock = Clock::get()?;

        // Check state is still Init (Player1 didn't reveal)
        require!(
            lottery.state == LotteryState::Init,
            LotteryError::InvalidState
        );

        // Check deadline has passed or reached
        require!(
            clock.unix_timestamp as u64 >= lottery.end_reveal,
            LotteryError::DeadlineNotPassed
        );

        // Get total pot
        let lottery_lamports = lottery.to_account_info().lamports();
        let rent_exempt = Rent::get()?.minimum_balance(lottery.to_account_info().data_len());
        let pot = lottery_lamports.checked_sub(rent_exempt).unwrap_or(0);

        // Transfer entire pot to Player2 as penalty
        **lottery.to_account_info().try_borrow_mut_lamports()? -= pot;
        **ctx.accounts.player2.to_account_info().try_borrow_mut_lamports()? += pot;

        Ok(())
    }

    pub fn redeem_if_p2_no_reveal(ctx: Context<RedeemIfP2NoRevealCtx>) -> Result<()> {
        let lottery = &mut ctx.accounts.lottery_info;
        let clock = Clock::get()?;

        // Check state is RevealP1 (Player2 didn't reveal)
        require!(
            lottery.state == LotteryState::RevealP1,
            LotteryError::InvalidState
        );

        // Check deadline has passed or reached (Player2 gets same deadline as Player1)
        require!(
            clock.unix_timestamp as u64 >= lottery.end_reveal,
            LotteryError::DeadlineNotPassed
        );

        // Get total pot
        let lottery_lamports = lottery.to_account_info().lamports();
        let rent_exempt = Rent::get()?.minimum_balance(lottery.to_account_info().data_len());
        let pot = lottery_lamports.checked_sub(rent_exempt).unwrap_or(0);

        // Transfer entire pot to Player1 as penalty
        **lottery.to_account_info().try_borrow_mut_lamports()? -= pot;
        **ctx.accounts.player1.to_account_info().try_borrow_mut_lamports()? += pot;

        Ok(())
    }
}

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
    pub player2: UncheckedAccount<'info>,
    
    #[account(
        mut,
        seeds = [player1.key().as_ref(), player2.key().as_ref()],
        bump,
        constraint = lottery_info.player1 == player1.key() @ LotteryError::UnauthorizedPlayer,
        constraint = lottery_info.player2 == player2.key() @ LotteryError::UnauthorizedPlayer
    )]
    pub lottery_info: Account<'info, LotteryInfo>,
}

#[derive(Accounts)]
pub struct RevealP2Ctx<'info> {
    /// CHECK: Player1 reference for validation
    #[account(mut)]
    pub player1: UncheckedAccount<'info>,
    
    #[account(mut)]
    pub player2: Signer<'info>,
    
    #[account(
        mut,
        seeds = [player1.key().as_ref(), player2.key().as_ref()],
        bump,
        constraint = lottery_info.player1 == player1.key() @ LotteryError::UnauthorizedPlayer,
        constraint = lottery_info.player2 == player2.key() @ LotteryError::UnauthorizedPlayer
    )]
    pub lottery_info: Account<'info, LotteryInfo>,
}

#[derive(Accounts)]
pub struct RedeemIfP1NoRevealCtx<'info> {
    /// CHECK: Player1 reference for validation
    #[account(mut)]
    pub player1: UncheckedAccount<'info>,
    
    #[account(mut)]
    pub player2: Signer<'info>,
    
    #[account(
        mut,
        seeds = [player1.key().as_ref(), player2.key().as_ref()],
        bump,
        constraint = lottery_info.player1 == player1.key() @ LotteryError::UnauthorizedPlayer,
        constraint = lottery_info.player2 == player2.key() @ LotteryError::UnauthorizedPlayer
    )]
    pub lottery_info: Account<'info, LotteryInfo>,
}

#[derive(Accounts)]
pub struct RedeemIfP2NoRevealCtx<'info> {
    #[account(mut)]
    pub player1: Signer<'info>,
    
    /// CHECK: Player2 reference for validation
    #[account(mut)]
    pub player2: UncheckedAccount<'info>,
    
    #[account(
        mut,
        seeds = [player1.key().as_ref(), player2.key().as_ref()],
        bump,
        constraint = lottery_info.player1 == player1.key() @ LotteryError::UnauthorizedPlayer,
        constraint = lottery_info.player2 == player2.key() @ LotteryError::UnauthorizedPlayer
    )]
    pub lottery_info: Account<'info, LotteryInfo>,
}

#[account]
#[derive(InitSpace)]
pub struct LotteryInfo {
    pub state: LotteryState,
    pub player1: Pubkey,
    pub player2: Pubkey,
    pub hashlock1: [u8; 32],
    #[max_len(128)]
    pub secret1: String,
    pub hashlock2: [u8; 32],
    #[max_len(128)]
    pub secret2: String,
    pub end_reveal: u64,
}

#[derive(AnchorSerialize, AnchorDeserialize, Clone, Copy, PartialEq, Eq, InitSpace)]
pub enum LotteryState {
    Init = 0,
    RevealP1 = 1,
    RevealP2 = 2,
}

#[error_code]
pub enum LotteryError {
    #[msg("Hashlocks cannot be identical")]
    IdenticalHashlocks,
    #[msg("End reveal timestamp is in the past")]
    EndRevealInPast,
    #[msg("Invalid lottery state for this operation")]
    InvalidState,
    #[msg("Reveal deadline has passed")]
    RevealDeadlinePassed,
    #[msg("Secret does not match hashlock")]
    InvalidSecret,
    #[msg("Unauthorized player")]
    UnauthorizedPlayer,
    #[msg("Deadline has not passed yet")]
    DeadlineNotPassed,
}