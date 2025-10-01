use anchor_lang::prelude::*;

declare_id!("YGigkLBGFtqtcqpJMre8XeHb2Ujshkbsus9FS9vPgWo");

#[program]
pub mod storage {
    use super::*;

    pub fn initialize(ctx: Context<InitializeCtx>) -> Result<()> {
        // PDAs will be created if missing (init_if_needed). No data to write initially.
        Ok(())
    }

    pub fn store_string(ctx: Context<StoreStringCtx>, data_to_store: String) -> Result<()> {
        // Safety cap (change as your policy requires)
        const MAX_STRING_BYTES: usize = 2 * 1024 * 1024; // 2 MiB

        let bytes_len = data_to_store.as_bytes().len();
        require!(bytes_len <= MAX_STRING_BYTES, StorageError::DataTooLarge);

        let account = &mut ctx.accounts.string_storage_pda;
        account.my_string = data_to_store;

        Ok(())
    }

    pub fn store_bytes(ctx: Context<StoreBytesCtx>, data_to_store: Vec<u8>) -> Result<()> {
        const MAX_BYTES: usize = 4 * 1024 * 1024; // 4 MiB

        let bytes_len = data_to_store.len();
        require!(bytes_len <= MAX_BYTES, StorageError::DataTooLarge);

        let account = &mut ctx.accounts.bytes_storage_pda;
        account.my_bytes = data_to_store;

        Ok(())
    }
}

/// Stores a String (length-prefixed) in an account
#[account]
pub struct MemoryStringPDA {
    pub my_string: String,
}

/// Stores raw bytes (Vec<u8>) in an account
#[account]
pub struct MemoryBytesPDA {
    pub my_bytes: Vec<u8>,
}

/* ---------------------------
   Contexts and account constraints
   --------------------------- */

#[derive(Accounts)]
pub struct InitializeCtx<'info> {
    /// signer
    #[account(mut)]
    pub user: Signer<'info>,

    /// String PDA: init only if needed
    #[account(
        init_if_needed,
        payer = user,
        seeds = [b"storage_string", user.key().as_ref()],
        bump,
        // discriminator (8) + 4 (length prefix) + 0 bytes initially
        space = 8 + 4 + 0
    )]
    pub string_storage_pda: Account<'info, MemoryStringPDA>,

    /// Bytes PDA: init only if needed
    #[account(
        init_if_needed,
        payer = user,
        seeds = [b"storage_bytes", user.key().as_ref()],
        bump,
        space = 8 + 4 + 0
    )]
    pub bytes_storage_pda: Account<'info, MemoryBytesPDA>,

    pub system_program: Program<'info, System>,
}

/// `store_string` needs the instruction argument in the context so attributes can reference it.
#[derive(Accounts)]
#[instruction(data_to_store: String)]
pub struct StoreStringCtx<'info> {
    /// signer
    #[account(mut)]
    pub user: Signer<'info>,

    /// Reallocate the PDA to exactly discriminator + 4 + data length
    #[account(
        mut,
        seeds = [b"storage_string", user.key().as_ref()],
        bump,
        // discriminator (8) + length prefix (4) + bytes
        realloc = 8 + 4 + data_to_store.len(),
        realloc::payer = user,
        // zero new memory to ensure no residual bytes remain and serialized data is clean
        realloc::zero = true
    )]
    pub string_storage_pda: Account<'info, MemoryStringPDA>,

    pub system_program: Program<'info, System>,
}

/// `store_bytes` needs the instruction argument in the context so attributes can reference it.
#[derive(Accounts)]
#[instruction(data_to_store: Vec<u8>)]
pub struct StoreBytesCtx<'info> {
    /// signer
    #[account(mut)]
    pub user: Signer<'info>,

    #[account(
        mut,
        seeds = [b"storage_bytes", user.key().as_ref()],
        bump,
        realloc = 8 + 4 + data_to_store.len(),
        realloc::payer = user,
        realloc::zero = true
    )]
    pub bytes_storage_pda: Account<'info, MemoryBytesPDA>,

    pub system_program: Program<'info, System>,
}

/* ---------------------------
   Errors
   --------------------------- */

#[error_code]
pub enum StorageError {
    #[msg("Data too large for storage policy")]
    DataTooLarge,
}
