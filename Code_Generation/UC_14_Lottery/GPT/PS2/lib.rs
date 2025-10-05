use anchor_lang::prelude::*;

declare_id!("6GAsJQHX4agdF8qn1DgTmJxDeJ1EPRSHcnVxoTPGCr7F");

#[program]
pub mod lottery_gpt {
    use super::*;

    pub fn join(
        ctx: Context<JoinLottery>,
        hashlock1: Vec<u8>,
        hashlock2: Vec<u8>,
        delay: u64,
        wager: u64,
    ) -> Result<()> {
        let lottery = &mut ctx.accounts.lottery_info;

        // Initialize lottery state
        lottery.player1 = ctx.accounts.player1.key();
        lottery.player2 = ctx.accounts.player2.key();
        lottery.hashlock1 = hashlock1;
        lottery.hashlock2 = hashlock2;
        lottery.state = LotteryState::WaitingForReveal;
        lottery.end_reveal = Clock::get()?.unix_timestamp as u64 + delay;
        lottery.bump = *ctx.bumps.get("lottery_info").unwrap();

        // Bind account infos to variables to extend lifetimes
        let player1_ai = ctx.accounts.player1.to_account_info();
        let player2_ai = ctx.accounts.player2.to_account_info();
        let lottery_ai = ctx.accounts.lottery_info.to_account_info();

        // Borrow lamports mutably
        let mut from_p1 = player1_ai.try_borrow_mut_lamports()?;
        let mut from_p2 = player2_ai.try_borrow_mut_lamports()?;
        let mut to_pda = lottery_ai.try_borrow_mut_lamports()?;

        // Transfer wagers
        **from_p1 = from_p1.checked_sub(wager).ok_or(LotteryError::InsufficientFunds)?;
        **from_p2 = from_p2.checked_sub(wager).ok_or(LotteryError::InsufficientFunds)?;
        **to_pda = to_pda.checked_add(wager * 2).unwrap();

        Ok(())
    }

    pub fn reveal_p1(ctx: Context<Reveal>, secret: Vec<u8>) -> Result<()> {
        let lottery = &mut ctx.accounts.lottery_info;

        // Verify secret
        require!(hash(secret.clone()) == lottery.hashlock1, LotteryError::InvalidReveal);

        lottery.revealed1 = true;

        Ok(())
    }

    pub fn reveal_p2(ctx: Context<Reveal>, secret: Vec<u8>) -> Result<()> {
        let lottery = &mut ctx.accounts.lottery_info;

        require!(lottery.revealed1, LotteryError::RevealSequence);
        require!(hash(secret.clone()) == lottery.hashlock2, LotteryError::InvalidReveal);

        lottery.revealed2 = true;

        Ok(())
    }

    pub fn redeem(ctx: Context<Redeem>) -> Result<()> {
        let lottery = &mut ctx.accounts.lottery_info;
        let player_ai = ctx.accounts.winner.to_account_info();
        let lottery_ai = ctx.accounts.lottery_info.to_account_info();

        let mut from_pda = lottery_ai.try_borrow_mut_lamports()?;
        let mut to_player = player_ai.try_borrow_mut_lamports()?;

        // Transfer all funds to winner
        **to_player = to_player.checked_add(**from_pda).unwrap();
        **from_pda = 0;

        lottery.state = LotteryState::Finished;

        Ok(())
    }
}

// Helper function to hash secrets
fn hash(secret: Vec<u8>) -> Vec<u8> {
    use sha2::{Digest, Sha256};
    Sha256::digest(&secret).to_vec()
}

// -------------------- Account Contexts --------------------
#[derive(Accounts)]
pub struct JoinLottery<'info> {
    #[account(mut)]
    pub player1: Signer<'info>,
    #[account(mut)]
    pub player2: Signer<'info>,
    #[account(
        init,
        payer = player1,
        space = 8 + std::mem::size_of::<LotteryInfo>(),
        seeds = [b"lottery", player1.key().as_ref(), player2.key().as_ref()],
        bump
    )]
    pub lottery_info: Account<'info, LotteryInfo>,
    pub system_program: Program<'info, System>,
}

#[derive(Accounts)]
pub struct Reveal<'info> {
    #[account(mut)]
    pub player: Signer<'info>,
    #[account(mut, seeds = [b"lottery", lottery_info.player1.as_ref(), lottery_info.player2.as_ref()], bump = lottery_info.bump)]
    pub lottery_info: Account<'info, LotteryInfo>,
}

#[derive(Accounts)]
pub struct Redeem<'info> {
    #[account(mut)]
    pub winner: Signer<'info>,
    #[account(mut, seeds = [b"lottery", lottery_info.player1.as_ref(), lottery_info.player2.as_ref()], bump = lottery_info.bump)]
    pub lottery_info: Account<'info, LotteryInfo>,
}

// -------------------- Data Structures --------------------
#[account]
pub struct LotteryInfo {
    pub player1: Pubkey,
    pub player2: Pubkey,
    pub hashlock1: Vec<u8>,
    pub hashlock2: Vec<u8>,
    pub revealed1: bool,
    pub revealed2: bool,
    pub state: LotteryState,
    pub end_reveal: u64,
    pub bump: u8,
}

#[derive(AnchorSerialize, AnchorDeserialize, Clone, PartialEq, Eq)]
pub enum LotteryState {
    WaitingForReveal,
    Finished,
}

// -------------------- Errors --------------------
#[error_code]
pub enum LotteryError {
    #[msg("Invalid reveal secret")]
    InvalidReveal,
    #[msg("Reveal must happen in proper sequence")]
    RevealSequence,
    #[msg("Insufficient funds for wager")]
    InsufficientFunds,
}
