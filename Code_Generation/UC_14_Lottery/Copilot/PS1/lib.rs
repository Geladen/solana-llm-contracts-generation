use anchor_lang::prelude::*;
use anchor_lang::solana_program::{keccak, sysvar::clock::Clock};
use anchor_lang::system_program::{Transfer, transfer};

declare_id!("2pngBWtERx4ShLzqhcGLMB7sPwYq7co8iKG9URmBWYka");

pub const REVEAL_EXTENSION: u64 = 60; // seconds

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
        require!(hashlock1 != hashlock2, LotteryError::IdenticalHashlocks);

        let clock = Clock::get()?;
        let now = clock.unix_timestamp as u64;
        let end_reveal = now
            .checked_add(delay)
            .ok_or(LotteryError::TimestampOverflow)?;
        require!(end_reveal > now, LotteryError::EndRevealInPast);

        let lottery = &mut ctx.accounts.lottery_info;
        lottery.state = LotteryState::Init;
        lottery.player1 = ctx.accounts.player1.key();
        lottery.player2 = ctx.accounts.player2.key();
        lottery.hashlock1 = hashlock1;
        lottery.hashlock2 = hashlock2;
        lottery.secret1 = String::new();
        lottery.secret2 = String::new();
        lottery.end_reveal = end_reveal;

        // Transfer 'amount' lamports from player1 to PDA
        {
            let cpi_accounts = Transfer {
                from: ctx.accounts.player1.to_account_info(),
                to: ctx.accounts.lottery_info.to_account_info(),
            };
            let cpi_ctx = CpiContext::new(ctx.accounts.system_program.to_account_info(), cpi_accounts);
            transfer(cpi_ctx, amount)?;
        }

        // Transfer 'amount' lamports from player2 to PDA
        {
            let cpi_accounts = Transfer {
                from: ctx.accounts.player2.to_account_info(),
                to: ctx.accounts.lottery_info.to_account_info(),
            };
            let cpi_ctx = CpiContext::new(ctx.accounts.system_program.to_account_info(), cpi_accounts);
            transfer(cpi_ctx, amount)?;
        }

        Ok(())
    }

    pub fn reveal_p1(ctx: Context<RevealP1Ctx>, secret: String) -> Result<()> {
        let clock = Clock::get()?;
        let now = clock.unix_timestamp as u64;
        let lottery = &mut ctx.accounts.lottery_info;

        require!(now <= lottery.end_reveal, LotteryError::RevealDeadlinePassed);
        require!(ctx.accounts.player1.key() == lottery.player1, LotteryError::InvalidPlayer);

        let hashed = keccak::hash(secret.as_bytes()).0;
        require!(hashed == lottery.hashlock1, LotteryError::HashMismatch);

        lottery.secret1 = secret;
        lottery.state = LotteryState::RevealP1;

        Ok(())
    }

    pub fn reveal_p2(ctx: Context<RevealP2Ctx>, secret: String) -> Result<()> {
        let clock = Clock::get()?;
        let now = clock.unix_timestamp as u64;
        let lottery = &mut ctx.accounts.lottery_info;

        let cutoff = lottery
            .end_reveal
            .checked_add(REVEAL_EXTENSION)
            .ok_or(LotteryError::TimestampOverflow)?;
        require!(now <= cutoff, LotteryError::RevealDeadlinePassed);
        require!(ctx.accounts.player2.key() == lottery.player2, LotteryError::InvalidPlayer);

        let hashed = keccak::hash(secret.as_bytes()).0;
        require!(hashed == lottery.hashlock2, LotteryError::HashMismatch);

        // Ensure player1 already revealed
        require!(!lottery.secret1.is_empty(), LotteryError::P1NotRevealed);

        // Store secret2 and update state
        lottery.secret2 = secret.clone();
        lottery.state = LotteryState::RevealP2;

        // Determine winner
        let s1_len = lottery.secret1.len();
        let s2_len = secret.len();
        let parity = (s1_len + s2_len) % 2;

        // Transfer whole pot to winner using safe scoped borrows
        let lottery_ai = ctx.accounts.lottery_info.to_account_info();
        let player1_ai = ctx.accounts.player1.to_account_info();
        let player2_ai = ctx.accounts.player2.to_account_info();

        // take pot and zero PDA in one scoped borrow
        let pot: u64;
        {
            let mut lotto_lamports_ref = lottery_ai.try_borrow_mut_lamports()?;
            pot = **lotto_lamports_ref;
            require!(pot > 0, LotteryError::EmptyPot);
            **lotto_lamports_ref = 0u64;
        } // lotto_lamports_ref dropped here

        if parity == 0 {
            let mut dest_ref = player1_ai.try_borrow_mut_lamports()?;
            let curr = **dest_ref;
            let new = curr.checked_add(pot).ok_or(LotteryError::LamportOverflow)?;
            **dest_ref = new;
        } else {
            let mut dest_ref = player2_ai.try_borrow_mut_lamports()?;
            let curr = **dest_ref;
            let new = curr.checked_add(pot).ok_or(LotteryError::LamportOverflow)?;
            **dest_ref = new;
        }

        Ok(())
    }

    pub fn redeem_if_p1_no_reveal(ctx: Context<RedeemIfP1NoRevealCtx>) -> Result<()> {
        let clock = Clock::get()?;
        let now = clock.unix_timestamp as u64;
        let lottery = &mut ctx.accounts.lottery_info;

        require!(now > lottery.end_reveal, LotteryError::RevealDeadlineNotPassed);
        require!(lottery.secret1.is_empty(), LotteryError::P1AlreadyRevealed);
        require!(ctx.accounts.player2.key() == lottery.player2, LotteryError::InvalidPlayer);

        let lottery_ai = ctx.accounts.lottery_info.to_account_info();
        let player2_ai = ctx.accounts.player2.to_account_info();

        let pot: u64;
        {
            let mut lotto_lamports_ref = lottery_ai.try_borrow_mut_lamports()?;
            pot = **lotto_lamports_ref;
            require!(pot > 0, LotteryError::EmptyPot);
            **lotto_lamports_ref = 0u64;
        }

        let mut dest_ref = player2_ai.try_borrow_mut_lamports()?;
        let curr = **dest_ref;
        let new = curr.checked_add(pot).ok_or(LotteryError::LamportOverflow)?;
        **dest_ref = new;

        Ok(())
    }

    pub fn redeem_if_p2_no_reveal(ctx: Context<RedeemIfP2NoRevealCtx>) -> Result<()> {
        let clock = Clock::get()?;
        let now = clock.unix_timestamp as u64;
        let lottery = &mut ctx.accounts.lottery_info;

        let cutoff = lottery
            .end_reveal
            .checked_add(REVEAL_EXTENSION)
            .ok_or(LotteryError::TimestampOverflow)?;
        require!(now > cutoff, LotteryError::RevealDeadlineNotPassed);
        require!(lottery.secret2.is_empty(), LotteryError::P2AlreadyRevealed);
        require!(ctx.accounts.player1.key() == lottery.player1, LotteryError::InvalidPlayer);

        let lottery_ai = ctx.accounts.lottery_info.to_account_info();
        let player1_ai = ctx.accounts.player1.to_account_info();

        let pot: u64;
        {
            let mut lotto_lamports_ref = lottery_ai.try_borrow_mut_lamports()?;
            pot = **lotto_lamports_ref;
            require!(pot > 0, LotteryError::EmptyPot);
            **lotto_lamports_ref = 0u64;
        }

        let mut dest_ref = player1_ai.try_borrow_mut_lamports()?;
        let curr = **dest_ref;
        let new = curr.checked_add(pot).ok_or(LotteryError::LamportOverflow)?;
        **dest_ref = new;

        Ok(())
    }
}

#[derive(AnchorSerialize, AnchorDeserialize, Clone, PartialEq, Eq)]
pub enum LotteryState {
    Init = 0,
    RevealP1 = 1,
    RevealP2 = 2,
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

#[derive(Accounts)]
#[instruction(hashlock1: [u8;32], hashlock2: [u8;32], delay: u64, amount: u64)]
pub struct JoinCtx<'info> {
    /// CHECK: player1 is the payer and signer for init; validated as signer by Anchor
    #[account(mut, signer)]
    pub player1: AccountInfo<'info>,

    /// CHECK: player2 must sign the join; validated as signer by Anchor
    #[account(mut, signer)]
    pub player2: AccountInfo<'info>,

    /// PDA initialized and owned by this program via seeds [player1, player2]
    #[account(
        init,
        payer = player1,
        space = LotteryInfo::space(),
        seeds = [player1.key().as_ref(), player2.key().as_ref()],
        bump
    )]
    pub lottery_info: Account<'info, LotteryInfo>,

    pub system_program: Program<'info, System>,
}

#[derive(Accounts)]
pub struct RevealP1Ctx<'info> {
    /// CHECK: player1 signer; Anchor enforces signer
    #[account(signer)]
    pub player1: AccountInfo<'info>,

    /// CHECK: player2 is readonly reference used for seed derivation only
    pub player2: AccountInfo<'info>,

    /// PDA derived from [player1, player2]; owned and validated by Anchor via seeds
    #[account(mut, seeds = [player1.key().as_ref(), player2.key().as_ref()], bump)]
    pub lottery_info: Account<'info, LotteryInfo>,
}

#[derive(Accounts)]
pub struct RevealP2Ctx<'info> {
    /// CHECK: player1 is a reference and may receive lamports; must be mutable
    #[account(mut)]
    pub player1: AccountInfo<'info>,

    /// CHECK: player2 signer and may receive lamports; must be mutable
    #[account(mut, signer)]
    pub player2: AccountInfo<'info>,

    /// PDA derived from [player1, player2]; owned and validated by Anchor via seeds
    #[account(mut, seeds = [player1.key().as_ref(), player2.key().as_ref()], bump)]
    pub lottery_info: Account<'info, LotteryInfo>,
}

#[derive(Accounts)]
pub struct RedeemIfP1NoRevealCtx<'info> {
    /// CHECK: player1 is readonly; used for seed derivation and validation
    pub player1: AccountInfo<'info>,

    /// CHECK: player2 signer and recipient for redeem; Anchor enforces signer
    #[account(mut, signer)]
    pub player2: AccountInfo<'info>,

    /// PDA derived from [player1, player2]; owned and validated by Anchor via seeds
    #[account(mut, seeds = [player1.key().as_ref(), player2.key().as_ref()], bump)]
    pub lottery_info: Account<'info, LotteryInfo>,
}

#[derive(Accounts)]
pub struct RedeemIfP2NoRevealCtx<'info> {
    /// CHECK: player1 signer and recipient for redeem; Anchor enforces signer
    #[account(mut, signer)]
    pub player1: AccountInfo<'info>,

    /// CHECK: player2 is readonly; used for seed derivation and validation
    pub player2: AccountInfo<'info>,

    /// PDA derived from [player1, player2]; owned and validated by Anchor via seeds
    #[account(mut, seeds = [player1.key().as_ref(), player2.key().as_ref()], bump)]
    pub lottery_info: Account<'info, LotteryInfo>,
}


impl LotteryInfo {
    pub fn space() -> usize {
        const MAX_SECRET_LEN: usize = 128;
        8 + 1 + 32 + 32 + 32 + 4 + MAX_SECRET_LEN + 32 + 4 + MAX_SECRET_LEN + 8
    }
}

#[error_code]
pub enum LotteryError {
    #[msg("Hashlocks must be different")]
    IdenticalHashlocks,
    #[msg("End reveal in the past or overflow")]
    EndRevealInPast,
    #[msg("Timestamp arithmetic overflow")]
    TimestampOverflow,
    #[msg("Reveal deadline has passed")]
    RevealDeadlinePassed,
    #[msg("Invalid player for this action")]
    InvalidPlayer,
    #[msg("Hash mismatch for provided secret")]
    HashMismatch,
    #[msg("Player1 did not reveal")]
    P1NotRevealed,
    #[msg("Player1 already revealed")]
    P1AlreadyRevealed,
    #[msg("Player2 did not reveal")]
    P2NotRevealed,
    #[msg("Player2 already revealed")]
    P2AlreadyRevealed,
    #[msg("Pot is empty")]
    EmptyPot,
    #[msg("Lamport arithmetic overflow")]
    LamportOverflow,
    #[msg("Reveal deadline not yet passed")]
    RevealDeadlineNotPassed,
}
