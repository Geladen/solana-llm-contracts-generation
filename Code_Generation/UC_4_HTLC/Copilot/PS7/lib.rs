use anchor_lang::prelude::*;
use anchor_lang::solana_program::{keccak, system_instruction, program::invoke_signed, clock};
use std::str;

declare_id!("2oB4E4tgekzAQJwzs16XfwdxwoqhdeJoCNXvoTFAov59");

/// Helpers (outside #[program])
fn bytes_to_hex(bytes: &[u8]) -> String {
    const HEX: &[u8; 16] = b"0123456789abcdef";
    let mut out = String::with_capacity(bytes.len() * 2);
    for &b in bytes {
        let hi = (b >> 4) as usize;
        let lo = (b & 0x0f) as usize;
        out.push(HEX[hi] as char);
        out.push(HEX[lo] as char);
    }
    out
}

fn hex_decode_str(s: &str) -> std::result::Result<Vec<u8>, ()> {
    let s = s.trim();
    if s.len() % 2 != 0 {
        return Err(());
    }
    let mut out = Vec::with_capacity(s.len() / 2);
    let bytes = s.as_bytes();
    for i in (0..s.len()).step_by(2) {
        let hi = char_hex_value(bytes[i])?;
        let lo = char_hex_value(bytes[i + 1])?;
        out.push((hi << 4) | lo);
    }
    Ok(out)
}

fn char_hex_value(b: u8) -> std::result::Result<u8, ()> {
    match b {
        b'0'..=b'9' => Ok(b - b'0'),
        b'a'..=b'f' => Ok(b - b'a' + 10),
        b'A'..=b'F' => Ok(b - b'A' + 10),
        _ => Err(()),
    }
}

#[program]
pub mod htlc {
    use super::*;

    pub fn initialize(
        ctx: Context<InitializeCtx>,
        hashed_secret: [u8; 32],
        delay: u64,
        amount: u64,
    ) -> Result<()> {
        let current_slot = Clock::get()?.slot;
        let reveal_timeout = current_slot.checked_add(delay).ok_or(ErrorCode::SlotOverflow)?;

        let owner_key = ctx.accounts.owner.key();
        let verifier_key = ctx.accounts.verifier.key();

        // derive expected PDA and bump and verify it matches provided account
        let (expected_pda, bump_seed) =
            Pubkey::find_program_address(&[owner_key.as_ref(), verifier_key.as_ref()], ctx.program_id);
        require_keys_eq!(expected_pda, ctx.accounts.htlc_info.to_account_info().key(), ErrorCode::InvalidPda);

        // initialize state
        {
            let htlc = &mut ctx.accounts.htlc_info;
            htlc.owner = owner_key;
            htlc.verifier = verifier_key;
            htlc.hashed_secret = hashed_secret;
            htlc.reveal_timeout = reveal_timeout;
            htlc.amount = amount;
            htlc.bump = bump_seed;
        }

        // transfer lamports from owner -> PDA (owner signs)
        let cpi_ctx = CpiContext::new(
            ctx.accounts.system_program.to_account_info(),
            anchor_lang::system_program::Transfer {
                from: ctx.accounts.owner.to_account_info(),
                to: ctx.accounts.htlc_info.to_account_info(),
            },
        );
        anchor_lang::system_program::transfer(cpi_ctx, amount)?;

        Ok(())
    }

    pub fn reveal(ctx: Context<RevealCtx>, secret: String) -> Result<()> {
        // capture minimal state then drop mutable borrow quickly
        let (owner_key, verifier_key, stored_hash, amount) = {
            let htlc = &mut ctx.accounts.htlc_info;
            require_keys_eq!(ctx.accounts.owner.key(), htlc.owner, ErrorCode::InvalidOwner);
            let current_slot = Clock::get()?.slot;
            require!(current_slot < htlc.reveal_timeout, ErrorCode::RevealAfterTimeout);
            (htlc.owner, htlc.verifier, htlc.hashed_secret, htlc.amount)
        };

        // recompute PDA & bump immediately and verify (prevents seed mismatch)
        let (expected_pda, bump) =
            Pubkey::find_program_address(&[owner_key.as_ref(), verifier_key.as_ref()], ctx.program_id);
        require_keys_eq!(expected_pda, ctx.accounts.htlc_info.to_account_info().key(), ErrorCode::InvalidPda);

        // Build list of candidate byte interpretations and their labels
        let mut computed = Vec::<(String, [u8; 32])>::new();

        // 1) exact UTF-8
        let b_utf8 = secret.as_bytes();
        computed.push(("utf8_exact".to_string(), keccak::hash(b_utf8).0));

        // 2) trimmed UTF-8
        let secret_trim = secret.trim();
        if secret_trim.as_bytes() != b_utf8 {
            computed.push(("utf8_trim".to_string(), keccak::hash(secret_trim.as_bytes()).0));
        }

        // 3) strip trailing newline / carriage returns
        let secret_strip = secret.trim_end_matches(|c| c == '\n' || c == '\r');
        if secret_strip.as_bytes() != b_utf8 && secret_strip != secret_trim {
            computed.push(("utf8_strip_newline".to_string(), keccak::hash(secret_strip.as_bytes()).0));
        }

        // 4) try hex decode
        if let Ok(decoded_hex) = hex_decode_str(secret.as_str()) {
            computed.push(("hex".to_string(), keccak::hash(&decoded_hex).0));
        }

        // Check candidates
        let mut matched = false;
        for (label, hash) in &computed {
            if *hash == stored_hash {
                matched = true;
                msg!("reveal: matched candidate={}", label);
                break;
            }
        }

        if !matched {
            // Log stored and computed hashes for debugging
            msg!("reveal: stored_hash={}", bytes_to_hex(&stored_hash));
            for (label, hash) in &computed {
                msg!("reveal: candidate={} hash={}", label, bytes_to_hex(hash));
            }
        }

        require!(matched, ErrorCode::SecretMismatch);

        // Log PDA + bump
        msg!("reveal: pda={} bump={}", ctx.accounts.htlc_info.key(), bump);

        // Build transfer instruction and invoke_signed
        let ix = system_instruction::transfer(
            ctx.accounts.htlc_info.to_account_info().key,
            ctx.accounts.owner.to_account_info().key,
            amount,
        );

        let seed_slice: &[&[u8]] = &[
            owner_key.as_ref(),
            verifier_key.as_ref(),
            &[bump],
        ];

        invoke_signed(
            &ix,
            &[
                ctx.accounts.htlc_info.to_account_info(),
                ctx.accounts.owner.to_account_info(),
                ctx.accounts.system_program.to_account_info(),
            ],
            &[seed_slice],
        )?;

        // zero amount under small mutable scope
        {
            let htlc = &mut ctx.accounts.htlc_info;
            require!(htlc.amount == amount, ErrorCode::AmountMismatch);
            htlc.amount = 0;
        }

        Ok(())
    }

    pub fn timeout(ctx: Context<TimeoutCtx>) -> Result<()> {
        // capture minimal state then drop borrow
        let (owner_key, verifier_key, _timeout_slot, amount) = {
            let htlc = &mut ctx.accounts.htlc_info;
            require_keys_eq!(ctx.accounts.verifier.key(), htlc.verifier, ErrorCode::InvalidVerifier);
            let current_slot = Clock::get()?.slot;
            require!(current_slot >= htlc.reveal_timeout, ErrorCode::TimeoutNotReached);
            (htlc.owner, htlc.verifier, htlc.reveal_timeout, htlc.amount)
        };

        // recompute bump and verify PDA
        let (expected_pda, bump) =
            Pubkey::find_program_address(&[owner_key.as_ref(), verifier_key.as_ref()], ctx.program_id);
        require_keys_eq!(expected_pda, ctx.accounts.htlc_info.to_account_info().key(), ErrorCode::InvalidPda);

        msg!("timeout: pda={} bump={}", ctx.accounts.htlc_info.key(), bump);

        // transfer
        let ix = system_instruction::transfer(
            ctx.accounts.htlc_info.to_account_info().key,
            ctx.accounts.verifier.to_account_info().key,
            amount,
        );

        let seed_slice: &[&[u8]] = &[
            owner_key.as_ref(),
            verifier_key.as_ref(),
            &[bump],
        ];

        invoke_signed(
            &ix,
            &[
                ctx.accounts.htlc_info.to_account_info(),
                ctx.accounts.verifier.to_account_info(),
                ctx.accounts.system_program.to_account_info(),
            ],
            &[seed_slice],
        )?;

        // zero amount
        {
            let htlc = &mut ctx.accounts.htlc_info;
            require!(htlc.amount == amount, ErrorCode::AmountMismatch);
            htlc.amount = 0;
        }

        Ok(())
    }
}

#[account]
pub struct HtlcPda {
    pub owner: Pubkey,
    pub verifier: Pubkey,
    pub hashed_secret: [u8; 32],
    pub reveal_timeout: u64,
    pub amount: u64,
    pub bump: u8,
}

const HTLC_PDA_SPACE: usize = 8 + 32 + 32 + 32 + 8 + 8 + 1;

#[derive(Accounts)]
#[instruction(hashed_secret: [u8;32], delay: u64, amount: u64)]
pub struct InitializeCtx<'info> {
    /// CHECK: Owner signs and pays for initialization and transfer; signer constraint enforces key ownership and CPI transfer uses owner's signature.
    #[account(mut, signer)]
    pub owner: AccountInfo<'info>,

    /// CHECK: Verifier is used as PDA seed and referenced by the PDA; no deserialization checks needed here.
    pub verifier: AccountInfo<'info>,

    /// HTLC PDA that stores state and holds lamports.
    /// PDA seeds = [owner.key().as_ref(), verifier.key().as_ref()]
    #[account(
        init,
        payer = owner,
        space = HTLC_PDA_SPACE,
        seeds = [owner.key.as_ref(), verifier.key.as_ref()],
        bump
    )]
    pub htlc_info: Account<'info, HtlcPda>,

    pub system_program: Program<'info, System>,
}

#[derive(Accounts)]
pub struct RevealCtx<'info> {
    /// CHECK: Owner must sign; equality to stored owner is enforced in handler.
    #[account(signer)]
    pub owner: AccountInfo<'info>,

    /// CHECK: Verifier is a reference pubkey used as PDA seed; validated against PDA via has_one.
    pub verifier: AccountInfo<'info>,

    /// PDA must be mutable; seeds ensure correct PDA; has_one verifies relationships.
    #[account(
        mut,
        seeds = [owner.key.as_ref(), verifier.key.as_ref()],
        bump = htlc_info.bump,
        has_one = owner,
        has_one = verifier
    )]
    pub htlc_info: Account<'info, HtlcPda>,

    pub system_program: Program<'info, System>,
}

#[derive(Accounts)]
pub struct TimeoutCtx<'info> {
    /// CHECK: Verifier must sign to trigger timeout payout.
    #[account(signer)]
    pub verifier: AccountInfo<'info>,

    /// CHECK: Owner is a readonly reference; validated via PDA has_one.
    pub owner: AccountInfo<'info>,

    /// PDA must be mutable; seeds must match; has_one verifies relationships.
    #[account(
        mut,
        seeds = [owner.key.as_ref(), verifier.key.as_ref()],
        bump = htlc_info.bump,
        has_one = owner,
        has_one = verifier
    )]
    pub htlc_info: Account<'info, HtlcPda>,

    pub system_program: Program<'info, System>,
}

#[error_code]
pub enum ErrorCode {
    #[msg("Provided secret does not match stored hash")]
    SecretMismatch,
    #[msg("Current slot is after the reveal timeout")]
    RevealAfterTimeout,
    #[msg("Timeout has not been reached yet")]
    TimeoutNotReached,
    #[msg("Signer is not the recorded owner")]
    InvalidOwner,
    #[msg("Signer is not the recorded verifier")]
    InvalidVerifier,
    #[msg("Slot arithmetic overflow")]
    SlotOverflow,
    #[msg("HTLC amount mismatch expected")]
    AmountMismatch,
    #[msg("HTLC PDA account does not match expected PDA")]
    InvalidPda,
}