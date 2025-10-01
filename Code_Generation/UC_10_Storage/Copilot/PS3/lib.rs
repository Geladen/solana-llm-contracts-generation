use anchor_lang::prelude::*;
use anchor_lang::solana_program::keccak::hashv;
use anchor_lang::solana_program::program::{invoke, invoke_signed};
use anchor_lang::solana_program::system_instruction;

declare_id!("HEHRSNeRbaB8cohktmxer4WpNaUEvKzF9R1jVCdrnnMn");

#[program]
pub mod storage {
    use super::*;

    pub fn initialize(ctx: Context<InitializeCtx>) -> Result<()> {
        // init_if_needed ensures idempotent creation
        Ok(())
    }

    pub fn store_string(ctx: Context<StoreStringCtx>, data_to_store: String) -> Result<()> {
        let payer = &ctx.accounts.user;

        // compute desired size: 8 (discriminator) + 4 (length prefix) + payload bytes
        let data_len = data_to_store.as_bytes().len();
        let desired_size: usize = 8 + 4 + data_len;

        // prepare nested seeds for invoke_signed: &[&[&[u8]]]
        let bump = ctx.bumps.string_storage_pda;
        let seed0: &[&[u8]] = &[
            b"storage_string",
            payer.key.as_ref(),
            &[bump],
        ];
        let signer_seeds_nested: &[&[&[u8]]] = &[seed0];

        // clone PDA AccountInfo for CPIs (owned clone)
        let pda_clone = ctx.accounts.string_storage_pda.to_account_info().clone();

        // Resize (top-up lamports, assign->allocate->assign-back) and write discriminator
        resize_account_if_needed_with_payer_and_sysprog(
            pda_clone,
            payer.to_account_info(),
            ctx.accounts.system_program.to_account_info(),
            desired_size,
            ctx.program_id,
            signer_seeds_nested,
            "MemoryStringPDA",
        )?;

        // After allocator/discriminator write, write the actual payload via typed account (fresh borrow)
        ctx.accounts.string_storage_pda.my_string = data_to_store;

        Ok(())
    }

    pub fn store_bytes(ctx: Context<StoreBytesCtx>, data_to_store: Vec<u8>) -> Result<()> {
        let payer = &ctx.accounts.user;

        // compute desired size: 8 (discriminator) + 4 (length prefix) + payload bytes
        let data_len = data_to_store.len();
        let desired_size: usize = 8 + 4 + data_len;

        // prepare nested seeds for invoke_signed: &[&[&[u8]]]
        let bump = ctx.bumps.bytes_storage_pda;
        let seed0: &[&[u8]] = &[
            b"storage_bytes",
            payer.key.as_ref(),
            &[bump],
        ];
        let signer_seeds_nested: &[&[&[u8]]] = &[seed0];

        // clone PDA AccountInfo for CPIs (owned clone)
        let pda_clone = ctx.accounts.bytes_storage_pda.to_account_info().clone();

        // Resize (top-up lamports, assign->allocate->assign-back) and write discriminator
        resize_account_if_needed_with_payer_and_sysprog(
            pda_clone,
            payer.to_account_info(),
            ctx.accounts.system_program.to_account_info(),
            desired_size,
            ctx.program_id,
            signer_seeds_nested,
            "MemoryBytesPDA",
        )?;

        // After allocator/discriminator write, write the actual payload via typed account (fresh borrow)
        ctx.accounts.bytes_storage_pda.my_bytes = data_to_store;

        Ok(())
    }
}

/// Helper: safely resize PDA account when desired_size > current size.
///
/// Steps:
/// 1) Top-up lamports from payer to meet rent-exempt requirement for desired_size.
/// 2) invoke_signed: assign PDA owner -> system program.
/// 3) invoke_signed: allocate the new data length.
/// 4) invoke_signed: assign PDA owner back to the program.
/// 5) Write Anchor discriminator (first 8 bytes) for the specified account struct name so Anchor can deserialize.
fn resize_account_if_needed_with_payer_and_sysprog<'a>(
    account_info: AccountInfo<'a>, // cloned PDA AccountInfo for CPIs
    payer_info: AccountInfo<'a>,
    system_program_info: AccountInfo<'a>,
    desired_size: usize,
    program_id: &Pubkey,
    signer_seeds_nested: &[&[&[u8]]], // nested seeds required by invoke_signed
    account_struct_name: &str,
) -> Result<()> {
    let current_len = account_info.data_len();
    if desired_size <= current_len {
        return Ok(());
    }

    // Compute lamports required for rent-exemption at desired_size
    let rent = Rent::get()?;
    let required_lamports = rent.minimum_balance(desired_size);
    let current_lamports = **account_info.lamports.borrow();

    // If PDA lacks required lamports, transfer the difference from payer (payer must sign)
    if current_lamports < required_lamports {
        let transfer_amount = required_lamports - current_lamports;
        let ix = system_instruction::transfer(payer_info.key, account_info.key, transfer_amount);
        // include system_program in accounts for transfer CPI
        invoke(
            &ix,
            &[
                payer_info.clone(),
                account_info.clone(),
                system_program_info.clone(),
            ],
        )
        .map_err(|_| error!(ErrorCode::LamportsTransferFailed))?;
    }

    // 1) Assign PDA owner to system program (program -> system)
    let assign_to_system_ix =
        system_instruction::assign(account_info.key, &anchor_lang::solana_program::system_program::ID);
    invoke_signed(
        &assign_to_system_ix,
        &[account_info.clone(), system_program_info.clone()],
        signer_seeds_nested,
    )
    .map_err(|_| error!(ErrorCode::AccountAssignFailed))?;

    // 2) Allocate new size
    let allocate_ix = system_instruction::allocate(account_info.key, desired_size as u64);
    invoke_signed(
        &allocate_ix,
        &[account_info.clone(), system_program_info.clone()],
        signer_seeds_nested,
    )
    .map_err(|_| error!(ErrorCode::AccountAllocateFailed))?;

    // 3) Reassign owner back to this program
    let assign_back_ix = system_instruction::assign(account_info.key, program_id);
    invoke_signed(
        &assign_back_ix,
        &[account_info.clone(), system_program_info.clone()],
        signer_seeds_nested,
    )
    .map_err(|_| error!(ErrorCode::AccountReassignFailed))?;

    // 4) Write Anchor discriminator (first 8 bytes) so Anchor client-side can deserialize
    // Anchor discriminator = first 8 bytes of Keccak256("account:<StructName>")
    let discr_string = format!("account:{}", account_struct_name);
    let hash = hashv(&[discr_string.as_bytes()]);
    let discr = &hash.0[..8];

    {
        let mut data = account_info.data.borrow_mut();
        if data.len() < 8 {
            return Err(error!(ErrorCode::AccountAllocateFailed));
        }
        data[..8].copy_from_slice(discr);
    }

    Ok(())
}

#[derive(Accounts)]
pub struct InitializeCtx<'info> {
    /// payer / user must sign
    #[account(mut)]
    pub user: Signer<'info>,

    /// String PDA: create if missing with minimal initial space (8 + 4 + 0)
    #[account(
        init_if_needed,
        payer = user,
        space = 8 + 4 + 0,
        seeds = [b"storage_string", user.key.as_ref()],
        bump
    )]
    pub string_storage_pda: Account<'info, MemoryStringPDA>,

    /// Bytes PDA: create if missing with minimal initial space (8 + 4 + 0)
    #[account(
        init_if_needed,
        payer = user,
        space = 8 + 4 + 0,
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

    /// mutable PDA account
    #[account(mut, seeds = [b"storage_string", user.key.as_ref()], bump)]
    pub string_storage_pda: Account<'info, MemoryStringPDA>,

    pub system_program: Program<'info, System>,
}

#[derive(Accounts)]
pub struct StoreBytesCtx<'info> {
    #[account(mut)]
    pub user: Signer<'info>,

    /// mutable PDA account
    #[account(mut, seeds = [b"storage_bytes", user.key.as_ref()], bump)]
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
    #[msg("Failed to assign account owner to system program.")]
    AccountAssignFailed,
    #[msg("Failed to allocate account space.")]
    AccountAllocateFailed,
    #[msg("Failed to reassign account owner back to program.")]
    AccountReassignFailed,
    #[msg("Failed to transfer lamports from payer to PDA.")]
    LamportsTransferFailed,
}
