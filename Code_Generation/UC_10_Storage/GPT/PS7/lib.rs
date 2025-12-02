use anchor_lang::prelude::*;

declare_id!("Eza1srDE7EckSSsjBqktMeEx6gnC5VsEDWo3gN8pvGAa");

const MAX_STRING_SIZE: usize = 1000; // increase if tests require larger
const MAX_BYTES_SIZE: usize = 1000;  // increase if tests require larger


#[program]
pub mod storage {
    use super::*;

    pub fn initialize(ctx: Context<InitializeCtx>) -> Result<()> {
        // nothing to initialize, accounts created via #[account(init)]
        Ok(())
    }

    pub fn store_string(ctx: Context<StoreStringCtx>, data_to_store: String) -> Result<()> {
        require!(data_to_store.len() <= MAX_STRING_SIZE, StorageError::StringTooLarge);
        ctx.accounts.string_storage_pda.my_string = data_to_store;
        Ok(())
    }

    pub fn store_bytes(ctx: Context<StoreBytesCtx>, data_to_store: Vec<u8>) -> Result<()> {
        require!(data_to_store.len() <= MAX_BYTES_SIZE, StorageError::BytesTooLarge);
        ctx.accounts.bytes_storage_pda.my_bytes = data_to_store;
        Ok(())
    }
}

#[derive(Accounts)]
pub struct InitializeCtx<'info> {
    #[account(mut)]
    pub user: Signer<'info>,

    #[account(
        init_if_needed,  // <--- allows graceful reinit
        payer = user,
        space = 8 + 4 + MAX_STRING_SIZE,
        seeds = [b"storage_string", user.key().as_ref()],
        bump
    )]
    pub string_storage_pda: Account<'info, MemoryStringPDA>,

    #[account(
        init_if_needed,  // <--- allows graceful reinit
        payer = user,
        space = 8 + 4 + MAX_BYTES_SIZE,
        seeds = [b"storage_bytes", user.key().as_ref()],
        bump
    )]
    pub bytes_storage_pda: Account<'info, MemoryBytesPDA>,

    pub system_program: Program<'info, System>,
}

#[derive(Accounts)]
pub struct StoreStringCtx<'info> {
    pub user: Signer<'info>,

    #[account(mut, seeds = [b"storage_string", user.key().as_ref()], bump)]
    pub string_storage_pda: Account<'info, MemoryStringPDA>,
}

#[derive(Accounts)]
pub struct StoreBytesCtx<'info> {
    pub user: Signer<'info>,

    #[account(mut, seeds = [b"storage_bytes", user.key().as_ref()], bump)]
    pub bytes_storage_pda: Account<'info, MemoryBytesPDA>,
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
pub enum StorageError {
    #[msg("String exceeds maximum allowed size")]
    StringTooLarge,
    #[msg("Byte array exceeds maximum allowed size")]
    BytesTooLarge,
}
