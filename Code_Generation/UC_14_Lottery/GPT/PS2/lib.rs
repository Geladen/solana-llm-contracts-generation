use anchor_lang::prelude::*;
use anchor_lang::solana_program::{keccak, system_instruction, program::invoke};

declare_id!("G259uKPsZNmaYpL78Z7ifPE4NQ3Dn8fR9QMfaSfbnkz7");

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
        let end_reveal = clock.unix_timestamp as u64 + delay;

        require!(hashlock1 != hashlock2, LotteryError::IdenticalHashlocks);
        require!(end_reveal > clock.unix_timestamp as u64, LotteryError::InvalidEndReveal);

        // Transfer lamports from Player1
        invoke(
            &system_instruction::transfer(&ctx.accounts.player1.key(), &ctx.accounts.lottery_info.key(), amount),
            &[
                ctx.accounts.player1.to_account_info(),
                ctx.accounts.lottery_info.to_account_info(),
                ctx.accounts.system_program.to_account_info(),
            ],
        )?;

        // Transfer lamports from Player2
        invoke(
            &system_instruction::transfer(&ctx.accounts.player2.key(), &ctx.accounts.lottery_info.key(), amount),
            &[
                ctx.accounts.player2.to_account_info(),
                ctx.accounts.lottery_info.to_account_info(),
                ctx.accounts.system_program.to_account_info(),
            ],
        )?;

        let lottery = &mut ctx.accounts.lottery_info;
        lottery.state = LotteryState::Init;
        lottery.player1 = ctx.accounts.player1.key();
        lottery.player2 = ctx.accounts.player2.key();
        lottery.hashlock1 = hashlock1;
        lottery.hashlock2 = hashlock2;
        lottery.secret1 = "".to_string();
        lottery.secret2 = "".to_string();
        lottery.end_reveal = end_reveal;

        Ok(())
    }

    pub fn reveal_p1(ctx: Context<RevealP1Ctx>, secret: String) -> Result<()> {
        let clock = Clock::get()?;
        let lottery = &mut ctx.accounts.lottery_info;

        require!(lottery.state == LotteryState::Init, LotteryError::InvalidState);
        require!(clock.unix_timestamp as u64 <= lottery.end_reveal, LotteryError::RevealDeadlinePassed);

        let hash = keccak::hash(secret.as_bytes());
        require!(hash.0 == lottery.hashlock1, LotteryError::HashMismatch);

        lottery.secret1 = secret;
        lottery.state = LotteryState::RevealP1;

        Ok(())
    }

    pub fn reveal_p2(ctx: Context<RevealP2Ctx>, secret: String) -> Result<()> {
        let clock = Clock::get()?;
        let lottery = &mut ctx.accounts.lottery_info;

        require!(lottery.state == LotteryState::RevealP1, LotteryError::InvalidState);
        require!(clock.unix_timestamp as u64 <= lottery.end_reveal + 60, LotteryError::RevealDeadlinePassed);

        let hash = keccak::hash(secret.as_bytes());
        require!(hash.0 == lottery.hashlock2, LotteryError::HashMismatch);

        lottery.secret2 = secret.clone();
        lottery.state = LotteryState::RevealP2;

        // Determine winner
        let winner = if (lottery.secret1.len() + secret.len()) % 2 == 0 {
            &ctx.accounts.player1
        } else {
            &ctx.accounts.player2
        };

        // Transfer entire pot to winner
        **winner.to_account_info().lamports.borrow_mut() += ctx.accounts.lottery_info.to_account_info().lamports();
        **ctx.accounts.lottery_info.to_account_info().lamports.borrow_mut() = 0;

        Ok(())
    }

    pub fn redeem_if_p1_no_reveal(ctx: Context<RedeemIfP1NoRevealCtx>) -> Result<()> {
        let clock = Clock::get()?;
        let lottery = &mut ctx.accounts.lottery_info;

        require!(lottery.state == LotteryState::Init, LotteryError::InvalidState);
        require!(clock.unix_timestamp as u64 > lottery.end_reveal, LotteryError::RevealNotExpired);

        **ctx.accounts.player2.to_account_info().lamports.borrow_mut() += ctx.accounts.lottery_info.to_account_info().lamports();
        **ctx.accounts.lottery_info.to_account_info().lamports.borrow_mut() = 0;

        Ok(())
    }

    pub fn redeem_if_p2_no_reveal(ctx: Context<RedeemIfP2NoRevealCtx>) -> Result<()> {
        let clock = Clock::get()?;
        let lottery = &mut ctx.accounts.lottery_info;

        require!(lottery.state == LotteryState::RevealP1, LotteryError::InvalidState);
        require!(clock.unix_timestamp as u64 > lottery.end_reveal + 60, LotteryError::RevealNotExpired);

        **ctx.accounts.player1.to_account_info().lamports.borrow_mut() += ctx.accounts.lottery_info.to_account_info().lamports();
        **ctx.accounts.lottery_info.to_account_info().lamports.borrow_mut() = 0;

        Ok(())
    }
}

#[derive(Accounts)]
#[instruction(hashlock1: [u8; 32], hashlock2: [u8; 32])]
pub struct JoinCtx<'info> {
    #[account(mut)]
    pub player1: Signer<'info>,
    #[account(mut)]
    pub player2: Signer<'info>,
    #[account(
        init,
        payer = player1,
        seeds = [player1.key().as_ref(), player2.key().as_ref()],
        bump,
        space = 8 + 1 + 32*2 + 32*2 + 64*2 + 8
    )]
    pub lottery_info: Account<'info, LotteryInfo>,
    pub system_program: Program<'info, System>,
}

#[derive(Accounts)]
pub struct RevealP1Ctx<'info> {
    pub player1: Signer<'info>,
    /// CHECK: Player2 is only referenced for PDA derivation
    pub player2: AccountInfo<'info>,
    #[account(mut, seeds = [player1.key().as_ref(), player2.key().as_ref()], bump)]
    pub lottery_info: Account<'info, LotteryInfo>,
}

#[derive(Accounts)]
pub struct RevealP2Ctx<'info> {
    /// CHECK: Player1 is only referenced for PDA derivation
    #[account(mut)] // ✅ must be mutable to receive lamports
    pub player1: AccountInfo<'info>,

    #[account(mut)] // ✅ must be mutable to receive lamports
    pub player2: Signer<'info>,

    #[account(mut, seeds = [player1.key().as_ref(), player2.key().as_ref()], bump)]
    pub lottery_info: Account<'info, LotteryInfo>,
}


#[derive(Accounts)]
pub struct RedeemIfP1NoRevealCtx<'info> {
    /// CHECK: Player1 is only referenced
    pub player1: AccountInfo<'info>,

    #[account(mut)] // ✅ must be mutable
    pub player2: Signer<'info>,

    #[account(mut, seeds = [player1.key().as_ref(), player2.key().as_ref()], bump)]
    pub lottery_info: Account<'info, LotteryInfo>,
}


#[derive(Accounts)]
pub struct RedeemIfP2NoRevealCtx<'info> {
    #[account(mut)] // ✅ must be mutable
    pub player1: Signer<'info>,

    /// CHECK: Player2 is only referenced
    pub player2: AccountInfo<'info>,

    #[account(mut, seeds = [player1.key().as_ref(), player2.key().as_ref()], bump)]
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

#[derive(AnchorSerialize, AnchorDeserialize, Clone, PartialEq, Eq)]
pub enum LotteryState {
    Init,
    RevealP1,
    RevealP2,
}

#[error_code]
pub enum LotteryError {
    #[msg("Hashlocks cannot be identical.")]
    IdenticalHashlocks,
    #[msg("End reveal must be in the future.")]
    InvalidEndReveal,
    #[msg("Invalid lottery state for this action.")]
    InvalidState,
    #[msg("Secret does not match hashlock.")]
    HashMismatch,
    #[msg("Reveal deadline has passed.")]
    RevealDeadlinePassed,
    #[msg("Reveal period has not expired.")]
    RevealNotExpired,
}
