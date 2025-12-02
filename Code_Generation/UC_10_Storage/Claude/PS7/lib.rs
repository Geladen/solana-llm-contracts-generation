use anchor_lang::prelude::*;

declare_id!("ArrhntCf57D4L9QhCBxjwscy14uamLWM6rHx9Wp2i7Mt");

#[program]
pub mod storage {
    use super::*;

    /// Initialize the storage system for a user
    /// Creates both string and bytes storage PDAs
    /// Handles reinitialization gracefully
    pub fn initialize(ctx: Context<InitializeCtx>) -> Result<()> {
        // Initialize with empty data if accounts are newly created
        let string_storage = &mut ctx.accounts.string_storage_pda;
        let bytes_storage = &mut ctx.accounts.bytes_storage_pda;
        
        // Set default empty values (handles both new initialization and reinitialization)
        string_storage.my_string = String::new();
        bytes_storage.my_bytes = Vec::new();
        
        Ok(())
    }

    /// Store a string in the user's string storage PDA
    /// Supports dynamic resizing for arbitrary-length strings
    pub fn store_string(ctx: Context<StoreStringCtx>, data_to_store: String) -> Result<()> {
        let string_storage = &mut ctx.accounts.string_storage_pda;
        string_storage.my_string = data_to_store;
        Ok(())
    }

    /// Store bytes in the user's bytes storage PDA
    /// Supports dynamic resizing for arbitrary-length byte sequences
    pub fn store_bytes(ctx: Context<StoreBytesCtx>, data_to_store: Vec<u8>) -> Result<()> {
        let bytes_storage = &mut ctx.accounts.bytes_storage_pda;
        bytes_storage.my_bytes = data_to_store;
        Ok(())
    }
}

// State Structures
#[account]
pub struct MemoryStringPDA {
    pub my_string: String,
}

#[account]
pub struct MemoryBytesPDA {
    pub my_bytes: Vec<u8>,
}

// Context Structures
#[derive(Accounts)]
pub struct InitializeCtx<'info> {
    #[account(mut)]
    pub user: Signer<'info>,
    
    #[account(
        init_if_needed,
        payer = user,
        space = 8 + 4 + 0, // discriminator + string length prefix + empty string
        seeds = [b"storage_string", user.key().as_ref()],
        bump
    )]
    pub string_storage_pda: Account<'info, MemoryStringPDA>,
    
    #[account(
        init_if_needed,
        payer = user,
        space = 8 + 4 + 0, // discriminator + vec length prefix + empty vec
        seeds = [b"storage_bytes", user.key().as_ref()],
        bump
    )]
    pub bytes_storage_pda: Account<'info, MemoryBytesPDA>,
    
    pub system_program: Program<'info, System>,
}

#[derive(Accounts)]
#[instruction(data_to_store: String)]
pub struct StoreStringCtx<'info> {
    #[account(mut)]
    pub user: Signer<'info>,
    
    #[account(
        mut,
        seeds = [b"storage_string", user.key().as_ref()],
        bump,
        realloc = 8 + 4 + data_to_store.len(),
        realloc::payer = user,
        realloc::zero = false,
    )]
    pub string_storage_pda: Account<'info, MemoryStringPDA>,
    
    pub system_program: Program<'info, System>,
}

#[derive(Accounts)]
#[instruction(data_to_store: Vec<u8>)]
pub struct StoreBytesCtx<'info> {
    #[account(mut)]
    pub user: Signer<'info>,
    
    #[account(
        mut,
        seeds = [b"storage_bytes", user.key().as_ref()],
        bump,
        realloc = 8 + 4 + data_to_store.len(),
        realloc::payer = user,
        realloc::zero = false,
    )]
    pub bytes_storage_pda: Account<'info, MemoryBytesPDA>,
    
    pub system_program: Program<'info, System>,
}