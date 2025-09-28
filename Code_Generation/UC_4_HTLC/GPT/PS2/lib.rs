use anchor_lang::prelude::*;
use anchor_lang::solana_program::keccak::hash as keccak_hash;
use anchor_lang::system_program;

declare_id!("4yoVCMUwibH17JvUPqJEXXw35aa5SuecjPp4Yi4FeeJF");

#[program]
pub mod htlc_gpt {
    use super::*;

    pub fn initialize(
        ctx: Context<InitializeCtx>,
        hashed_secret: [u8; 32],
        delay: u64,
        amount: u64,
    ) -> Result<()> {
        if amount == 0 {
            return Err(ErrorCode::AmountMustBeNonZero.into());
        }

        let clock = Clock::get()?;
        let htlc = &mut ctx.accounts.htlc_info;
        htlc.owner = ctx.accounts.owner.key();
        htlc.verifier = ctx.accounts.verifier.key();
        htlc.hashed_secret = hashed_secret;
        htlc.reveal_timeout = clock
            .slot
            .checked_add(delay)
            .ok_or(ErrorCode::Overflow)?;
        htlc.amount = amount;

        // Transfer lamports from owner → PDA
        let cpi_accounts = system_program::Transfer {
            from: ctx.accounts.owner.to_account_info(),
            to: ctx.accounts.htlc_info.to_account_info(),
        };
        let cpi_ctx = CpiContext::new(ctx.accounts.system_program.to_account_info(), cpi_accounts);
        system_program::transfer(cpi_ctx, amount)?;

        Ok(())
    }

    pub fn reveal(ctx: Context<RevealCtx>, secret: String) -> Result<()> {
        let (amount, owner_key, verifier_key, bump) = {
            let htlc = &mut ctx.accounts.htlc_info;
            let clock = Clock::get()?;
            if clock.slot > htlc.reveal_timeout {
                return Err(ErrorCode::RevealDeadlinePassed.into());
            }
            if htlc.amount == 0 {
                return Err(ErrorCode::AlreadyClaimed.into());
            }

            let secret_bytes = decode_secret_string(&secret)?;
            let computed = keccak_hash(&secret_bytes).0;
            if computed != htlc.hashed_secret {
                return Err(ErrorCode::HashMismatch.into());
            }

            (htlc.amount, htlc.owner, htlc.verifier, ctx.bumps.htlc_info)
        };

        // Transfer from PDA → owner
        let seeds = &[owner_key.as_ref(), verifier_key.as_ref(), &[bump]];
        let signer_seeds = &[seeds.as_ref()];

        let cpi_accounts = system_program::Transfer {
            from: ctx.accounts.htlc_info.to_account_info(),
            to: ctx.accounts.owner.to_account_info(),
        };
        let cpi_ctx =
            CpiContext::new_with_signer(ctx.accounts.system_program.to_account_info(), cpi_accounts, signer_seeds);
        system_program::transfer(cpi_ctx, amount)?;

        let htlc = &mut ctx.accounts.htlc_info;
        htlc.amount = 0;
        htlc.hashed_secret = [0u8; 32];
        htlc.reveal_timeout = 0;

        Ok(())
    }

    pub fn timeout(ctx: Context<TimeoutCtx>) -> Result<()> {
        let (amount, owner_key, verifier_key, bump) = {
            let htlc = &mut ctx.accounts.htlc_info;
            let clock = Clock::get()?;
            if clock.slot <= htlc.reveal_timeout {
                return Err(ErrorCode::RevealStillActive.into());
            }
            if htlc.amount == 0 {
                return Err(ErrorCode::AlreadyClaimed.into());
            }

            (htlc.amount, htlc.owner, htlc.verifier, ctx.bumps.htlc_info)
        };

        // Transfer from PDA → verifier
        let seeds = &[owner_key.as_ref(), verifier_key.as_ref(), &[bump]];
        let signer_seeds = &[seeds.as_ref()];

        let cpi_accounts = system_program::Transfer {
            from: ctx.accounts.htlc_info.to_account_info(),
            to: ctx.accounts.verifier.to_account_info(),
        };
        let cpi_ctx =
            CpiContext::new_with_signer(ctx.accounts.system_program.to_account_info(), cpi_accounts, signer_seeds);
        system_program::transfer(cpi_ctx, amount)?;

        let htlc = &mut ctx.accounts.htlc_info;
        htlc.amount = 0;
        htlc.hashed_secret = [0u8; 32];
        htlc.reveal_timeout = 0;

        Ok(())
    }
}

/// Accounts for initialize
#[derive(Accounts)]
#[instruction(hashed_secret: [u8; 32], delay: u64, amount: u64)]
pub struct InitializeCtx<'info> {
    #[account(mut)]
    pub owner: Signer<'info>,

    /// CHECK: verifier only stored in PDA
    pub verifier: UncheckedAccount<'info>,

    #[account(
        init,
        payer = owner,
        space = 8 + HtlcPDA::LEN,
        seeds = [owner.key().as_ref(), verifier.key().as_ref()],
        bump
    )]
    pub htlc_info: Account<'info, HtlcPDA>,

    pub system_program: Program<'info, System>,
}


/// Accounts for reveal
#[derive(Accounts)]
pub struct RevealCtx<'info> {
    #[account(mut)]
    pub owner: Signer<'info>,

    /// CHECK: verifier only used for PDA seeds
    pub verifier: UncheckedAccount<'info>,

    #[account(
        mut,
        seeds = [owner.key().as_ref(), verifier.key().as_ref()],
        bump,
        has_one = owner,
        has_one = verifier
    )]
    pub htlc_info: Account<'info, HtlcPDA>,

    pub system_program: Program<'info, System>,
}

/// Accounts for timeout
#[derive(Accounts)]
pub struct TimeoutCtx<'info> {
    #[account(mut)]
    pub verifier: Signer<'info>,

    /// CHECK: owner only used for PDA seeds
    pub owner: UncheckedAccount<'info>,

    #[account(
        mut,
        seeds = [owner.key().as_ref(), verifier.key().as_ref()],
        bump,
        has_one = owner,
        has_one = verifier
    )]
    pub htlc_info: Account<'info, HtlcPDA>,

    pub system_program: Program<'info, System>,
}

/// PDA data
#[account]
pub struct HtlcPDA {
    pub owner: Pubkey,
    pub verifier: Pubkey,
    pub hashed_secret: [u8; 32],
    pub reveal_timeout: u64,
    pub amount: u64,
}

impl HtlcPDA {
    pub const LEN: usize = 32 + 32 + 32 + 8 + 8;
}

// Helper functions
fn decode_secret_string(s: &str) -> Result<Vec<u8>> {
    let trimmed = s.trim();
    let without_prefix = trimmed.strip_prefix("0x").unwrap_or(trimmed);

    if without_prefix.len() % 2 == 0 && without_prefix.chars().all(|c| c.is_ascii_hexdigit()) {
        return decode_hex(without_prefix);
    }

    Ok(trimmed.as_bytes().to_vec())
}

fn decode_hex(s: &str) -> Result<Vec<u8>> {
    fn val(c: u8) -> Option<u8> {
        match c {
            b'0'..=b'9' => Some(c - b'0'),
            b'a'..=b'f' => Some(10 + (c - b'a')),
            b'A'..=b'F' => Some(10 + (c - b'A')),
            _ => None,
        }
    }
    let bytes = s.as_bytes();
    let mut out = Vec::with_capacity(bytes.len() / 2);
    let mut i = 0;
    while i < bytes.len() {
        let hi = val(bytes[i]).ok_or(ErrorCode::InvalidHexSecret)?;
        let lo = val(bytes[i + 1]).ok_or(ErrorCode::InvalidHexSecret)?;
        out.push((hi << 4) | lo);
        i += 2;
    }
    Ok(out)
}

#[error_code]
pub enum ErrorCode {
    #[msg("Hash mismatch (keccak(secret) != stored commitment)")]
    HashMismatch,
    #[msg("Reveal deadline has already passed")]
    RevealDeadlinePassed,
    #[msg("Timeout has not been reached yet (reveal still active)")]
    RevealStillActive,
    #[msg("No funds available in HTLC / already claimed")]
    AlreadyClaimed,
    #[msg("Provided amount must be non-zero")]
    AmountMustBeNonZero,
    #[msg("Integer overflow")]
    Overflow,
    #[msg("Invalid hex in provided secret string")]
    InvalidHexSecret,
}
