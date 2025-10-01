use anchor_lang::prelude::*;
use anchor_lang::solana_program::{
    program::{invoke, invoke_signed},
    system_instruction,
};
use sha2::{Digest, Sha256};

declare_id!("2hPuBtgXx6thXdRqkphRTvoz4WdUWibWtkgvFLkqu23e");

#[program]
pub mod storage_copilot {
    use super::*;

    pub fn initialize(ctx: Context<InitializeCtx>) -> Result<()> {
        Ok(())
    }

    pub fn store_string(ctx: Context<StoreStringCtx>, data_to_store: String) -> Result<()> {
        // 1) Update typed account so try_serialize on Account<> reflects intended state.
        ctx.accounts.string_storage_pda.my_string = data_to_store.clone();

        // 2) Serialize the concrete Account<> (Anchor AccountSerialize) to compute exact payload.
        let mut payload: Vec<u8> = Vec::new();
        ctx.accounts
            .string_storage_pda
            .try_serialize(&mut payload)?; // Borsh payload (no discriminator)

        // 3) Compute required size (discriminator + payload)
        let required_size = discriminator_len() + payload.len();

        // 4) Derive bump and build signer seeds (include bump)
        let (_pda, bump) = Pubkey::find_program_address(
            &[b"storage_string", ctx.accounts.user.key.as_ref()],
            ctx.program_id,
        );
        let bump_slice = &[bump];
        let seeds: [&[u8]; 3] = [b"storage_string", ctx.accounts.user.key.as_ref(), bump_slice];
        let signer_seeds: &[&[u8]] = &seeds;

        // 5) Resize & manage lamports (fund before realloc for growth)
        perform_realloc_and_rent_management(
            &mut ctx.accounts.string_storage_pda.to_account_info(),
            &ctx.accounts.user.to_account_info(),
            signer_seeds,
            required_size,
        )?;

        // 6) Write discriminator + payload into account buffer
        write_account_data_and_clear_rest(
            &mut ctx.accounts.string_storage_pda.to_account_info(),
            "MemoryStringPDA",
            &payload,
        )?;

        Ok(())
    }

    pub fn store_bytes(ctx: Context<StoreBytesCtx>, data_to_store: Vec<u8>) -> Result<()> {
        // 1) Update typed account
        ctx.accounts.bytes_storage_pda.my_bytes = data_to_store.clone();

        // 2) Serialize Account<> payload
        let mut payload: Vec<u8> = Vec::new();
        ctx.accounts
            .bytes_storage_pda
            .try_serialize(&mut payload)?;

        // 3) Required size
        let required_size = discriminator_len() + payload.len();

        // 4) Derive bump and signer seeds
        let (_pda, bump) = Pubkey::find_program_address(
            &[b"storage_bytes", ctx.accounts.user.key.as_ref()],
            ctx.program_id,
        );
        let bump_slice = &[bump];
        let seeds: [&[u8]; 3] = [b"storage_bytes", ctx.accounts.user.key.as_ref(), bump_slice];
        let signer_seeds: &[&[u8]] = &seeds;

        // 5) Resize & rent management
        perform_realloc_and_rent_management(
            &mut ctx.accounts.bytes_storage_pda.to_account_info(),
            &ctx.accounts.user.to_account_info(),
            signer_seeds,
            required_size,
        )?;

        // 6) Write discriminator + payload and zero any remaining bytes
        write_account_data_and_clear_rest(
            &mut ctx.accounts.bytes_storage_pda.to_account_info(),
            "MemoryBytesPDA",
            &payload,
        )?;

        Ok(())
    }
}

fn discriminator_len() -> usize {
    8usize
}

/// Compute Anchor-style 8-byte discriminator for "account:<StructName>"
fn anchor_discriminator_for(name: &str) -> [u8; 8] {
    let full = format!("account:{}", name);
    let mut hasher = Sha256::new();
    hasher.update(full.as_bytes());
    let hash = hasher.finalize();
    let mut disc = [0u8; 8];
    disc.copy_from_slice(&hash[..8]);
    disc
}

/// Write discriminator + payload into the account's data buffer and zero any remaining bytes.
/// This guarantees no stale bytes remain from previous larger writes.
fn write_account_data_and_clear_rest<'info>(
    ai: &mut AccountInfo<'info>,
    account_name: &str,
    payload: &[u8],
) -> Result<()> {
    let total_payload_len = discriminator_len() + payload.len();
    let buffer_len = ai.data_len();
    if buffer_len < total_payload_len {
        return err!(ErrorCode::InsufficientAccountData);
    }

    let disc = anchor_discriminator_for(account_name);

    // Write discriminator and payload
    {
        let mut data = ai.data.borrow_mut();
        data[..8].copy_from_slice(&disc);
        data[8..8 + payload.len()].copy_from_slice(payload);

        // Zero any remaining bytes after the written payload up to buffer_len
        if buffer_len > total_payload_len {
            for b in data[8 + payload.len()..buffer_len].iter_mut() {
                *b = 0u8;
            }
        }
    }

    Ok(())
}

/// Realloc + rent management:
/// - Grow: fund PDA from payer first (payer signs transaction), then realloc
/// - Shrink: refund excess lamports from PDA -> payer via invoke_signed (PDA signs), then realloc
fn perform_realloc_and_rent_management<'info>(
    pda_ai: &mut AccountInfo<'info>,
    payer_ai: &AccountInfo<'info>,
    signer_seeds: &[&[u8]],
    required_size: usize,
) -> Result<()> {
    let rent = Rent::get()?;
    let current_len = pda_ai.data_len();
    if required_size == current_len {
        return Ok(());
    }

    let required_rent = rent.minimum_balance(required_size);
    let current_lamports = **pda_ai.lamports.borrow();

    if required_size > current_len {
        // Fund PDA from payer (payer signs) before realloc
        if current_lamports < required_rent {
            let diff = required_rent - current_lamports;
            invoke(
                &system_instruction::transfer(payer_ai.key, pda_ai.key, diff),
                &[payer_ai.clone(), pda_ai.clone()],
            )?;
        }

        // Then increase data size
        pda_ai.realloc(required_size, false)?;
    } else {
        // Shrink: refund excess lamports from PDA -> payer using PDA-signed CPI
        if current_lamports > required_rent {
            let excess = current_lamports - required_rent;
            invoke_signed(
                &system_instruction::transfer(pda_ai.key, payer_ai.key, excess),
                &[pda_ai.clone(), payer_ai.clone()],
                &[signer_seeds],
            )?;
        }

        // Then shrink
        pda_ai.realloc(required_size, false)?;
    }

    Ok(())
}

#[derive(Accounts)]
#[instruction()]
pub struct InitializeCtx<'info> {
    #[account(mut)]
    pub user: Signer<'info>,

    /// Create if missing; payer = user
    #[account(
        init_if_needed,
        payer = user,
        space = 8 + 4 + 0, // discriminator + length prefix + zero bytes
        seeds = [b"storage_string", user.key.as_ref()],
        bump
    )]
    pub string_storage_pda: Account<'info, MemoryStringPDA>,

    #[account(
        init_if_needed,
        payer = user,
        space = 8 + 4 + 0, // discriminator + length prefix + zero bytes
        seeds = [b"storage_bytes", user.key.as_ref()],
        bump
    )]
    pub bytes_storage_pda: Account<'info, MemoryBytesPDA>,

    pub system_program: Program<'info, System>,
}

#[derive(Accounts)]
pub struct StoreStringCtx<'info> {
    #[account(mut)]
    pub user: Signer<'info>,

    #[account(
        mut,
        seeds = [b"storage_string", user.key.as_ref()],
        bump,
    )]
    pub string_storage_pda: Account<'info, MemoryStringPDA>,

    pub system_program: Program<'info, System>,
}

#[derive(Accounts)]
pub struct StoreBytesCtx<'info> {
    #[account(mut)]
    pub user: Signer<'info>,

    #[account(
        mut,
        seeds = [b"storage_bytes", user.key.as_ref()],
        bump,
    )]
    pub bytes_storage_pda: Account<'info, MemoryBytesPDA>,

    pub system_program: Program<'info, System>,
}

#[account]
pub struct MemoryStringPDA {
    pub my_string: String,
}

#[account]
pub struct MemoryBytesPDA {
    pub my_bytes: Vec<u8>,
}

#[error_code]
pub enum ErrorCode {
    #[msg("Account data buffer is smaller than expected for write.")]
    InsufficientAccountData,
}
