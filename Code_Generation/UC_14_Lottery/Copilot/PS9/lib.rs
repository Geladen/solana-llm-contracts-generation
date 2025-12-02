use anchor_lang::prelude::*;
use anchor_lang::system_program;
use borsh::{BorshDeserialize, BorshSerialize};
use anchor_lang::solana_program::keccak::hash as keccak_hash;


declare_id!("8aSXzBbrabFtCD7z9W8uJP8SocP9p8p8r6n3GRL9ZrXu");

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
        // Both player signers enforced by Context
        // Reject identical commitments
        require!(hashlock1 != hashlock2, LotteryError::IdenticalHashlocks);

        // Compute deadline
        let clock = Clock::get()?;
        let now = clock.unix_timestamp as u64;
        let end_reveal = now.checked_add(delay).ok_or(LotteryError::DeadlineOverflow)?;
        require!(end_reveal > now, LotteryError::EndRevealInPast);

        // Initialize state in PDA
        let lottery = &mut ctx.accounts.lottery_info;
        lottery.state = LotteryState::Init;
        lottery.player1 = ctx.accounts.player1.key();
        lottery.player2 = ctx.accounts.player2.key();
        lottery.hashlock1 = hashlock1;
        lottery.hashlock2 = hashlock2;
        lottery.secret1 = String::new();
        lottery.secret2 = String::new();
        lottery.end_reveal = end_reveal;

        // Transfer 'amount' lamports from player1 to the PDA (CPI)
        // and from player2 to the PDA (CPI)
        // Use system_program::transfer CPI; both players are signers per spec
        let ix1 = system_program::Transfer {
            from: ctx.accounts.player1.to_account_info(),
            to: ctx.accounts.lottery_info.to_account_info(),
        };
        let cpi_ctx1 = CpiContext::new(ctx.accounts.system_program.to_account_info(), ix1);
        system_program::transfer(cpi_ctx1, amount)?;

        // Transfer from player2
        let ix2 = system_program::Transfer {
            from: ctx.accounts.player2.to_account_info(),
            to: ctx.accounts.lottery_info.to_account_info(),
        };
        let cpi_ctx2 = CpiContext::new(ctx.accounts.system_program.to_account_info(), ix2);
        system_program::transfer(cpi_ctx2, amount)?;

        Ok(())
    }

    pub fn reveal_p1(ctx: Context<RevealP1Ctx>, secret: String) -> Result<()> {
        // Must be before deadline
        let clock = Clock::get()?;
        let now = clock.unix_timestamp as u64;
        let lottery = &mut ctx.accounts.lottery_info;

        require!(lottery.state == LotteryState::Init, LotteryError::InvalidStateForRevealP1);
        require!(now <= lottery.end_reveal, LotteryError::RevealDeadlinePassed);

        // Validate keccak(secret) == hashlock1
        let h = keccak_hash(secret.as_bytes()).0;
        require!(h == lottery.hashlock1, LotteryError::InvalidSecret);

        lottery.secret1 = secret;
        lottery.state = LotteryState::RevealP1;

        Ok(())
    }

    pub fn reveal_p2(ctx: Context<RevealP2Ctx>, secret: String) -> Result<()> {
        // Called by player2
        let clock = Clock::get()?;
        let now = clock.unix_timestamp as u64;
        let lottery = &mut ctx.accounts.lottery_info;

        // reveal_p2 allowed if state is RevealP1 (p1 already revealed)
        // or if state is Init but p1 already revealed? We require RevealP1 state
        require!(
            lottery.state == LotteryState::RevealP1,
            LotteryError::InvalidStateForRevealP2
        );

        // Allow reveal_p2 up to (end_reveal + extension). The spec says "before deadline + extension".
        // We'll treat the extension as equal to the original delay (i.e., same length) for simplicity,
        // but since no explicit extension parameter exists, interpret "deadline + extension" as end_reveal + (end_reveal - start)
        // Simpler and deterministic: allow reveal_p2 up to end_reveal + (end_reveal - (end_reveal - delay)) which equals end_reveal*2? That is confusing.
        // To keep deterministic and safe we will allow reveal_p2 until end_reveal + 0 (i.e., immediate) unless secret1 revealed before end_reveal.
        // The specification expects an extension; choose a conservative extension of 0 here but implement exactly: allow reveal_p2 if now <= end_reveal.checked_add(0).
        // However to follow spec, interpret "extension" as the same delay value stored implicitly: we'll derive extension = lottery.end_reveal - (lottery.end_reveal - delay) isn't present.
        // To avoid ambiguity, we'll implement reveal_p2 must be called before end_reveal.checked_add(DEFAULT_EXTENSION) where DEFAULT_EXTENSION = 300 (5 minutes).
        // The contract does not accept an extension parameter, so we choose a fixed extension of 300 seconds.
        // This is deterministic and documented in code comments.
        const DEFAULT_EXTENSION: u64 = 300;
        require!(
            now <= lottery.end_reveal.checked_add(DEFAULT_EXTENSION).ok_or(LotteryError::DeadlineOverflow)?,
            LotteryError::RevealDeadlinePassed
        );

        // Validate secret against hashlock2
        let h = keccak_hash(secret.as_bytes()).0;
        require!(h == lottery.hashlock2, LotteryError::InvalidSecret);

        lottery.secret2 = secret.clone();
        lottery.state = LotteryState::RevealP2;

        // Determine winner: (secret1.len() + secret2.len()) % 2
        let s1_len = lottery.secret1.len();
        let s2_len = secret.len();
        let parity = (s1_len + s2_len) % 2;

        // Obtain mutable AccountInfo references
        let mut lottery_ai = ctx.accounts.lottery_info.to_account_info();
        let mut player1_ai = ctx.accounts.player1.to_account_info();
        let mut player2_ai = ctx.accounts.player2.to_account_info();

        // Choose winner as a mutable AccountInfo reference
        let winner_ai: &mut AccountInfo = if parity == 0 {
            &mut player1_ai
        } else {
            &mut player2_ai
        };

        let pot = **lottery_ai.lamports.borrow();
        require!(pot > 0, LotteryError::NoPot);

        // Drain PDA and credit winner safely
        **lottery_ai.lamports.borrow_mut() = 0u64;
        **winner_ai.lamports.borrow_mut() = winner_ai
        .lamports()
        .checked_add(pot)
        .ok_or(LotteryError::LamportOverflow)?;

        Ok(())
    }

    pub fn redeem_if_p1_no_reveal(ctx: Context<RedeemIfP1NoRevealCtx>) -> Result<()> {
        // Called by player2 after deadline if player1 didn't reveal
        let clock = Clock::get()?;
        let now = clock.unix_timestamp as u64;
        let lottery = &mut ctx.accounts.lottery_info;

        require!(lottery.state == LotteryState::Init, LotteryError::InvalidStateForRedeemIfP1NoReveal);
        require!(now >= lottery.end_reveal, LotteryError::RevealDeadlineNotPassed);

        // Ensure player1 didn't reveal (secret1 empty)
        require!(lottery.secret1.is_empty(), LotteryError::Player1AlreadyRevealed);

        // Transfer pot to player2 (signer)
        let lottery_ai = &mut ctx.accounts.lottery_info.to_account_info();
        let player2_ai = &mut ctx.accounts.player2.to_account_info();

        let pot = **lottery_ai.lamports.borrow();
        require!(pot > 0, LotteryError::NoPot);

        **lottery_ai.lamports.borrow_mut() = 0u64;
        **player2_ai.lamports.borrow_mut() = player2_ai
            .lamports()
            .checked_add(pot)
            .ok_or(LotteryError::LamportOverflow)?;

        Ok(())
    }

    pub fn redeem_if_p2_no_reveal(ctx: Context<RedeemIfP2NoRevealCtx>) -> Result<()> {
        // Called by player1 after deadline + extension if player2 didn't reveal
        let clock = Clock::get()?;
        let now = clock.unix_timestamp as u64;
        let lottery = &mut ctx.accounts.lottery_info;

        require!(lottery.state == LotteryState::RevealP1, LotteryError::InvalidStateForRedeemIfP2NoReveal);

        // Use same DEFAULT_EXTENSION as in reveal_p2
        const DEFAULT_EXTENSION: u64 = 300;
        require!(
            now >= lottery.end_reveal.checked_add(DEFAULT_EXTENSION).ok_or(LotteryError::DeadlineOverflow)?,
            LotteryError::RevealDeadlineNotPassed
        );

        // Ensure player2 didn't reveal
        require!(lottery.secret2.is_empty(), LotteryError::Player2AlreadyRevealed);

        // Transfer pot to player1 (signer)
        let lottery_ai = &mut ctx.accounts.lottery_info.to_account_info();
        let player1_ai = &mut ctx.accounts.player1.to_account_info();

        let pot = **lottery_ai.lamports.borrow();
        require!(pot > 0, LotteryError::NoPot);

        **lottery_ai.lamports.borrow_mut() = 0u64;
        **player1_ai.lamports.borrow_mut() = player1_ai
            .lamports()
            .checked_add(pot)
            .ok_or(LotteryError::LamportOverflow)?;

        Ok(())
    }
}

#[derive(Accounts)]
#[instruction(hashlock1: [u8;32], hashlock2: [u8;32], delay: u64, amount: u64)]
pub struct JoinCtx<'info> {
    /// CHECK: player1 signer
    #[account(mut, signer)]
    pub player1: AccountInfo<'info>,

    /// CHECK: player2 signer
    #[account(mut, signer)]
    pub player2: AccountInfo<'info>,

    /// Lottery PDA initialized here, payer = player1
    #[account(
        init,
        seeds = [player1.key().as_ref(), player2.key().as_ref()],
        bump,
        payer = player1,
        space = LotteryInfo::LEN
    )]
    pub lottery_info: Account<'info, LotteryInfo>,

    pub system_program: Program<'info, System>,
}

#[derive(Accounts)]
pub struct RevealP1Ctx<'info> {
    /// CHECK: player1 signer
    #[account(mut, signer)]
    pub player1: AccountInfo<'info>,

    /// CHECK: player2 reference (not signer)
    pub player2: AccountInfo<'info>,

    /// PDA: seeds must match players
    #[account(
        mut,
        seeds = [player1.key().as_ref(), player2.key().as_ref()],
        bump
    )]
    pub lottery_info: Account<'info, LotteryInfo>,
}

#[derive(Accounts)]
pub struct RevealP2Ctx<'info> {
    /// CHECK: player1 reference; may receive lamports so must be mutable
    #[account(mut)]
    pub player1: AccountInfo<'info>,

    /// CHECK: player2 signer; may receive lamports so must be mutable
    #[account(mut, signer)]
    pub player2: AccountInfo<'info>,

    /// PDA
    #[account(
        mut,
        seeds = [player1.key().as_ref(), player2.key().as_ref()],
        bump
    )]
    pub lottery_info: Account<'info, LotteryInfo>,
}

#[derive(Accounts)]
pub struct RedeemIfP1NoRevealCtx<'info> {
    /// CHECK: player1 (not signer) - kept non-mut because we don't credit player1 in this flow
    pub player1: AccountInfo<'info>,

    /// CHECK: player2 signer (collects pot) - must be mutable to receive lamports
    #[account(mut, signer)]
    pub player2: AccountInfo<'info>,

    /// PDA
    #[account(
        mut,
        seeds = [player1.key().as_ref(), player2.key().as_ref()],
        bump
    )]
    pub lottery_info: Account<'info, LotteryInfo>,
}
#[derive(Accounts)]
pub struct RedeemIfP2NoRevealCtx<'info> {
    /// CHECK: player1 signer (collects pot) - must be mutable to receive lamports
    #[account(mut, signer)]
    pub player1: AccountInfo<'info>,

    /// CHECK: player2 (not signer)
    pub player2: AccountInfo<'info>,

    /// PDA
    #[account(
        mut,
        seeds = [player1.key().as_ref(), player2.key().as_ref()],
        bump
    )]
    pub lottery_info: Account<'info, LotteryInfo>,
}

#[account]
#[derive(Default)]
pub struct LotteryInfo {
    pub state: LotteryState,    // 1 byte as u8
    pub player1: Pubkey,        // 32
    pub player2: Pubkey,        // 32
    pub hashlock1: [u8; 32],    // 32
    pub secret1: String,        // 4 + n
    pub hashlock2: [u8; 32],    // 32
    pub secret2: String,        // 4 + n
    pub end_reveal: u64,        // 8
}

// Manual size calculation and constant
impl LotteryInfo {
    // choose max secret length 64 bytes for each secret
    pub const MAX_SECRET_LEN: usize = 64;
    // discriminator 8 + state 1 + player1 32 + player2 32 + hashlock1 32 +
    // secret1 (4 + 64) + hashlock2 32 + secret2 (4 + 64) + end_reveal 8
    pub const LEN: usize = 8 + 1 + 32 + 32 + 32 + (4 + Self::MAX_SECRET_LEN) + 32 + (4 + Self::MAX_SECRET_LEN) + 8;
}

#[derive(AnchorSerialize, AnchorDeserialize, Clone, PartialEq, Eq)]
pub enum LotteryState {
    Init = 0,
    RevealP1 = 1,
    RevealP2 = 2,
}

impl Default for LotteryState {
    fn default() -> Self {
        LotteryState::Init
    }
}

#[error_code]
pub enum LotteryError {
    #[msg("Hash commitments must differ")]
    IdenticalHashlocks,
    #[msg("End reveal would be in the past")]
    EndRevealInPast,
    #[msg("Deadline arithmetic overflow")]
    DeadlineOverflow,
    #[msg("Invalid secret provided")]
    InvalidSecret,
    #[msg("Invalid program state for reveal by player1")]
    InvalidStateForRevealP1,
    #[msg("Invalid program state for reveal by player2")]
    InvalidStateForRevealP2,
    #[msg("Reveal deadline has passed")]
    RevealDeadlinePassed,
    #[msg("Reveal deadline not yet passed")]
    RevealDeadlineNotPassed,
    #[msg("Player1 has already revealed")]
    Player1AlreadyRevealed,
    #[msg("Player2 has already revealed")]
    Player2AlreadyRevealed,
    #[msg("No pot to distribute")]
    NoPot,
    #[msg("Lamport arithmetic overflow")]
    LamportOverflow,
    #[msg("Invalid state for redeem if p1 no reveal")]
    InvalidStateForRedeemIfP1NoReveal,
    #[msg("Invalid state for redeem if p2 no reveal")]
    InvalidStateForRedeemIfP2NoReveal,
}
