use anchor_lang::prelude::*;
use sha2::{Digest, Sha256};
use core::any::type_name;

use anchor_lang::solana_program::{
    account_info::AccountInfo,
    program::invoke_signed,
    system_instruction,
    sysvar::rent::Rent,
    pubkey::Pubkey,
};

declare_id!("5dNQdzCh7fbFSa5Gpy3ASZss7uPDQa9dke4TjmyhyE6M");


const MIN_PDA_SPACE: usize = 8; // Anchor discriminator only

#[program]
pub mod storage {
    use super::*;

    /// Initialize PDAs if missing. Idempotent.
    /// Creates minimal accounts (8 bytes) and writes Anchor discriminators.
    pub fn initialize(ctx: Context<InitializeCtx>) -> Result<()> {
        let program_id = ctx.program_id;

        // STRING PDA
        {
            let string_info = ctx.accounts.string_storage_pda.to_account_info();
            let (derived, bump) = Pubkey::find_program_address(
                &[b"storage_string", ctx.accounts.user.key.as_ref()],
                program_id,
            );
            require_keys_eq!(derived, *string_info.key);

            if *string_info.owner != *program_id || string_info.data_len() < MIN_PDA_SPACE {
                let rent = Rent::get()?;
                let lamports = rent.minimum_balance(MIN_PDA_SPACE);

                let create_ix = system_instruction::create_account(
                    ctx.accounts.user.key,
                    string_info.key,
                    lamports,
                    MIN_PDA_SPACE as u64,
                    program_id,
                );

                invoke_signed(
                    &create_ix,
                    &[ctx.accounts.user.to_account_info(), string_info.clone()],
                    &[&[b"storage_string", ctx.accounts.user.key.as_ref(), &[bump]]],
                )?;

                // write discriminator
                let disc = anchor_discriminator::<MemoryStringPDA>();
                string_info.data.borrow_mut()[..8].copy_from_slice(&disc);
            }
        }

        // BYTES PDA
        {
            let bytes_info = ctx.accounts.bytes_storage_pda.to_account_info();
            let (derived, bump) = Pubkey::find_program_address(
                &[b"storage_bytes", ctx.accounts.user.key.as_ref()],
                program_id,
            );
            require_keys_eq!(derived, *bytes_info.key);

            if *bytes_info.owner != *program_id || bytes_info.data_len() < MIN_PDA_SPACE {
                let rent = Rent::get()?;
                let lamports = rent.minimum_balance(MIN_PDA_SPACE);

                let create_ix = system_instruction::create_account(
                    ctx.accounts.user.key,
                    bytes_info.key,
                    lamports,
                    MIN_PDA_SPACE as u64,
                    program_id,
                );

                invoke_signed(
                    &create_ix,
                    &[ctx.accounts.user.to_account_info(), bytes_info.clone()],
                    &[&[b"storage_bytes", ctx.accounts.user.key.as_ref(), &[bump]]],
                )?;

                // write discriminator
                let disc = anchor_discriminator::<MemoryBytesPDA>();
                bytes_info.data.borrow_mut()[..8].copy_from_slice(&disc);
            }
        }

        Ok(())
    }

    /// Store a String into the user's string PDA. Requires user signature.
    /// If PDA too small, top up from user and allocate to new size, then write serialized struct at offset 8.
    pub fn store_string(ctx: Context<StoreStringCtx>, data_to_store: String) -> Result<()> {
        let body = MemoryStringPDA { my_string: data_to_store };
        let body_serialized = body.try_to_vec().map_err(|_| error!(ErrorCode::SerializeFail))?;
        let required_len = 8usize + body_serialized.len();

        let user_info = ctx.accounts.user.to_account_info().clone();
        let pda_info = ctx.accounts.string_storage_pda.to_account_info().clone();

        let (_pda_key, bump) = Pubkey::find_program_address(
            &[b"storage_string", ctx.accounts.user.key.as_ref()],
            ctx.program_id,
        );
        let seeds: &[&[u8]] = &[
            b"storage_string",
            ctx.accounts.user.key.as_ref(),
            &[bump],
        ];

        if pda_info.data_len() < required_len {
            resize_account_with_topup(user_info.clone(), pda_info.clone(), required_len, seeds)?;
        }

        let mut data = pda_info.data.borrow_mut();
        data[8..8 + body_serialized.len()].copy_from_slice(&body_serialized);

        // Update in-memory Anchor account for remainder of instruction
        ctx.accounts.string_storage_pda.my_string = body.my_string;

        Ok(())
    }

    /// Store bytes into the user's bytes PDA. Requires user signature.
    pub fn store_bytes(ctx: Context<StoreBytesCtx>, data_to_store: Vec<u8>) -> Result<()> {
        let body = MemoryBytesPDA { my_bytes: data_to_store };
        let body_serialized = body.try_to_vec().map_err(|_| error!(ErrorCode::SerializeFail))?;
        let required_len = 8usize + body_serialized.len();

        let user_info = ctx.accounts.user.to_account_info().clone();
        let pda_info = ctx.accounts.bytes_storage_pda.to_account_info().clone();

        let (_pda_key, bump) = Pubkey::find_program_address(
            &[b"storage_bytes", ctx.accounts.user.key.as_ref()],
            ctx.program_id,
        );
        let seeds: &[&[u8]] = &[
            b"storage_bytes",
            ctx.accounts.user.key.as_ref(),
            &[bump],
        ];

        if pda_info.data_len() < required_len {
            resize_account_with_topup(user_info.clone(), pda_info.clone(), required_len, seeds)?;
        }

        let mut data = pda_info.data.borrow_mut();
        data[8..8 + body_serialized.len()].copy_from_slice(&body_serialized);

        ctx.accounts.bytes_storage_pda.my_bytes = body.my_bytes;

        Ok(())
    }
}

/// Resize a program-owned account to new_len, preserving the 8-byte Anchor discriminator.
/// If the account lacks lamports for rent-exemption at new_len, transfer the exact difference from payer first.
fn resize_account_with_topup<'a>(
    payer: AccountInfo<'a>,
    target: AccountInfo<'a>,
    new_len: usize,
    seeds: &[&[u8]],
) -> Result<()> {
    let rent = Rent::get()?;
    let current_len = target.data_len();
    if new_len == current_len {
        return Ok(());
    }

    let required_lamports = rent.minimum_balance(new_len);
    let current_lamports = **target.lamports.borrow();

    if current_lamports < required_lamports {
        let diff = required_lamports - current_lamports;
        let transfer_ix = system_instruction::transfer(payer.key, target.key, diff);

        // payer signs transfer
        invoke_signed(
            &transfer_ix,
            &[payer.clone(), target.clone()],
            &[],
        ).map_err(|_| error!(ErrorCode::LamportTopupFailed))?;
    }

    // allocate (realloc) to new size; PDA signs with seeds
    let allocate_ix = system_instruction::allocate(target.key, new_len as u64);
    invoke_signed(
        &allocate_ix,
        &[target.clone()],
        &[seeds],
    ).map_err(|_| error!(ErrorCode::ReallocFailed))?;

    Ok(())
}

/// Compute Anchor 8-byte discriminator for an account type T:
/// first 8 bytes of SHA256("account:<TypeName>")
fn anchor_discriminator<T>() -> [u8; 8] {
    let full = type_name::<T>();
    let short = full.rsplit("::").next().unwrap_or(full);
    let label = format!("account:{}", short);
    let hash = Sha256::digest(label.as_bytes());
    let mut disc = [0u8; 8];
    disc.copy_from_slice(&hash[..8]);
    disc
}

#[derive(Accounts)]
pub struct InitializeCtx<'info> {
    #[account(mut)]
    pub user: Signer<'info>,

    /// CHECK: This AccountInfo may not exist yet; initialize creates the PDA, assigns it to the program,
    /// and writes the Anchor discriminator. We use raw AccountInfo here because Anchor cannot type-check
    /// an account that doesn't exist yet. After initialize returns the account will be program-owned and
    /// fetchable as a typed Account in subsequent instructions.
    #[account(mut)]
    pub string_storage_pda: AccountInfo<'info>,

    /// CHECK: This AccountInfo may not exist yet; initialize creates the PDA, assigns it to the program,
    /// and writes the Anchor discriminator. We use raw AccountInfo here because Anchor cannot type-check
    /// an account that doesn't exist yet. After initialize returns the account will be program-owned and
    /// fetchable as a typed Account in subsequent instructions.
    #[account(mut)]
    pub bytes_storage_pda: AccountInfo<'info>,

    pub system_program: Program<'info, System>,
}

#[derive(Accounts)]
pub struct StoreStringCtx<'info> {
    #[account(mut)]
    pub user: Signer<'info>,

    /// After initialize, this will be a typed account. We require mut because we'll update it in-memory.
    #[account(mut)]
    pub string_storage_pda: Account<'info, MemoryStringPDA>,

    pub system_program: Program<'info, System>,
}

#[derive(Accounts)]
pub struct StoreBytesCtx<'info> {
    #[account(mut)]
    pub user: Signer<'info>,

    #[account(mut)]
    pub bytes_storage_pda: Account<'info, MemoryBytesPDA>,

    pub system_program: Program<'info, System>,
}

#[account]
#[derive(Default)]
pub struct MemoryStringPDA {
    pub my_string: String,
}

#[account]
#[derive(Default)]
pub struct MemoryBytesPDA {
    pub my_bytes: Vec<u8>,
}

#[error_code]
pub enum ErrorCode {
    #[msg("Failed to serialize account data")]
    SerializeFail,
    #[msg("Lamport top-up transfer failed")]
    LamportTopupFailed,
    #[msg("Reallocation (allocate) instruction failed")]
    ReallocFailed,
}
