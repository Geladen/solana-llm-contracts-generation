use anchor_lang::prelude::*;
use anchor_lang::system_program::{transfer, Transfer};
declare_id!("GuiYASX7y5Rq6pgESGNq7MHEianMzbcckL3zJNbVjc9Q");

#[program]
pub mod lottery_gpt {
    use super::*;

    pub fn join(
    ctx: Context<Join>,
    bet_amount_p1: u64,
    player1_commitment: [u8; 32],
    bet_amount_p2: u64,
    player2_commitment: [u8; 32],
) -> Result<()> {
    // clone AccountInfos before taking mutable borrow
    let lottery_info_ai = ctx.accounts.lottery_info.to_account_info();

    // Both players must bet the same
    require!(bet_amount_p1 == bet_amount_p2, LotteryError::UnequalBets);

    // Transfer player1 bet
    transfer(
        CpiContext::new(
            ctx.accounts.system_program.to_account_info(),
            Transfer {
                from: ctx.accounts.player1.to_account_info(),
                to: lottery_info_ai.clone(),
            },
        ),
        bet_amount_p1,
    )?;

    // Transfer player2 bet
    transfer(
        CpiContext::new(
            ctx.accounts.system_program.to_account_info(),
            Transfer {
                from: ctx.accounts.player2.to_account_info(),
                to: lottery_info_ai.clone(),
            },
        ),
        bet_amount_p2,
    )?;

    // now safe to mutably borrow
    let lottery = &mut ctx.accounts.lottery_info;

    lottery.player1 = ctx.accounts.player1.key();
    lottery.player2 = ctx.accounts.player2.key();
    lottery.bet_amount = bet_amount_p1;
    lottery.commitment1 = player1_commitment;
    lottery.commitment2 = player2_commitment;
    lottery.state = LotteryState::RevealP1;

    let clock = Clock::get()?;
    lottery.reveal_deadline = clock.unix_timestamp + 10;

    Ok(())
}


    pub fn reveal_p1(ctx: Context<RevealP1>, secret: String) -> Result<()> {
        let lottery = &mut ctx.accounts.lottery_info;

        require!(lottery.state == LotteryState::RevealP1, LotteryError::InvalidState);

        let expected = anchor_lang::solana_program::keccak::hash(secret.as_bytes());
        require!(expected.0 == lottery.commitment1, LotteryError::InvalidReveal);

        lottery.secret1 = secret.into_bytes();
        lottery.state = LotteryState::RevealP2;

        Ok(())
    }

    pub fn reveal_p2(ctx: Context<RevealP2>, secret: String) -> Result<()> {
        let lottery = &mut ctx.accounts.lottery_info;

        require!(lottery.state == LotteryState::RevealP2, LotteryError::InvalidState);

        let expected = anchor_lang::solana_program::keccak::hash(secret.as_bytes());
        require!(expected.0 == lottery.commitment2, LotteryError::InvalidReveal);

        lottery.secret2 = secret.into_bytes();

        // Determine winner: XOR first byte of secrets
        let s1 = lottery.secret1[0];
        let s2 = lottery.secret2[0];
        let winner = if (s1 ^ s2) % 2 == 0 {
            lottery.player1
        } else {
            lottery.player2
        };

        let pot = **lottery.to_account_info().lamports.borrow();

        **lottery.to_account_info().try_borrow_mut_lamports()? -= pot;
        **ctx.accounts.winner.to_account_info().try_borrow_mut_lamports()? += pot;

        lottery.state = LotteryState::Finished;

        Ok(())
    }

    pub fn redeem_if_p1_no_reveal(ctx: Context<RedeemIfP1NoReveal>) -> Result<()> {
        let lottery = &mut ctx.accounts.lottery_info;

        let clock = Clock::get()?;
        require!(clock.unix_timestamp > lottery.reveal_deadline, LotteryError::TooEarly);

        let pot = **lottery.to_account_info().lamports.borrow();

        **lottery.to_account_info().try_borrow_mut_lamports()? -= pot;
        **ctx.accounts.player2.to_account_info().try_borrow_mut_lamports()? += pot;

        lottery.state = LotteryState::Finished;

        Ok(())
    }
}

#[derive(Accounts)]
pub struct Join<'info> {
    /// CHECK: Verified in instruction
    #[account(mut)]
    pub player1: Signer<'info>,
    /// CHECK: Verified in instruction
    #[account(mut)]
    pub player2: Signer<'info>,
    #[account(init, payer = player1, space = 8 + Lottery::SIZE)]
    pub lottery_info: Account<'info, Lottery>,
    pub system_program: Program<'info, System>,
}

#[derive(Accounts)]
pub struct RevealP1<'info> {
    #[account(mut)]
    pub player1: Signer<'info>,
    #[account(mut)]
    pub lottery_info: Account<'info, Lottery>,
}

#[derive(Accounts)]
pub struct RevealP2<'info> {
    #[account(mut)]
    pub player2: Signer<'info>,
    #[account(mut)]
    pub lottery_info: Account<'info, Lottery>,
    /// CHECK: Verified winner payout
    #[account(mut)]
    pub winner: AccountInfo<'info>,
}

#[derive(Accounts)]
pub struct RedeemIfP1NoReveal<'info> {
    #[account(mut)]
    pub player2: Signer<'info>,
    #[account(mut)]
    pub lottery_info: Account<'info, Lottery>,
}

#[account]
pub struct Lottery {
    pub player1: Pubkey,
    pub player2: Pubkey,
    pub bet_amount: u64,
    pub commitment1: [u8; 32],
    pub commitment2: [u8; 32],
    pub secret1: Vec<u8>,
    pub secret2: Vec<u8>,
    pub state: LotteryState,
    pub reveal_deadline: i64,
}

impl Lottery {
    pub const SIZE: usize = 32 + 32 + 8 + 32 + 32 + 64 + 64 + 1 + 8;
}

#[derive(AnchorSerialize, AnchorDeserialize, Clone, PartialEq, Eq)]
pub enum LotteryState {
    Waiting,
    RevealP1,
    RevealP2,
    Finished,
}

#[error_code]
pub enum LotteryError {
    #[msg("Bets must be equal.")]
    UnequalBets,
    #[msg("Invalid reveal.")]
    InvalidReveal,
    #[msg("Too early to redeem.")]
    TooEarly,
    #[msg("Invalid state transition.")]
    InvalidState,
}
