use anchor_lang::prelude::*;

declare_id!("CxgPhWHk1t8ux9U4zj6C7ZiZ7qDeaKWLsxRG1ZZzYRsD");

#[program]
pub mod storage {
    use super::*;

    /// Initialize: creates the two PDAs (string + bytes) for the signer with
    /// minimal empty space. User pays for account creation.
    pub fn initialize(ctx: Context<InitializeCtx>) -> Result<()> {
        // Nothing to store initially — accounts are created with empty payload space.
        // Anchor has already created the PDAs in the CPI because of the `init` constraints.
        Ok(())
    }

    /// Store a string into the user's string PDA. The account will be reallocated
    /// to exactly fit `8 + 4 + data_to_store.len()` bytes (discriminator + length + payload).
    pub fn store_string(ctx: Context<StoreStringCtx>, data_to_store: String) -> Result<()> {
        let pda = &mut ctx.accounts.string_storage_pda;
        // Overwrite stored string (Anchor's realloc happened before entering this handler).
        pda.my_string = data_to_store;
        Ok(())
    }

    /// Store arbitrary bytes (Vec<u8>) into the user's bytes PDA. The account will be reallocated
    /// to exactly fit `8 + 4 + data_to_store.len()` bytes (discriminator + length + payload).
    pub fn store_bytes(ctx: Context<StoreBytesCtx>, data_to_store: Vec<u8>) -> Result<()> {
        let pda = &mut ctx.accounts.bytes_storage_pda;
        pda.my_bytes = data_to_store;
        Ok(())
    }
}

/// Account data stored on-chain for strings
#[account]
pub struct MemoryStringPDA {
    pub my_string: String, // serialized as 4-byte length prefix + bytes
}

impl MemoryStringPDA {
    /// Minimum allocated space for an empty String account: discriminator (8) + length prefix (4)
    pub const INIT_SPACE: usize = 8 + 4;

    /// Utility: compute total space needed for `len` UTF-8 bytes.
    pub fn space_for(len: usize) -> usize {
        8 + 4 + len
    }
}

/// Account data stored on-chain for bytes
#[account]
pub struct MemoryBytesPDA {
    pub my_bytes: Vec<u8>, // serialized as 4-byte length prefix + bytes
}

impl MemoryBytesPDA {
    /// Minimum allocated space for an empty Vec<u8> account
    pub const INIT_SPACE: usize = 8 + 4;

    /// Utility: compute total space needed for `len` bytes.
    pub fn space_for(len: usize) -> usize {
        8 + 4 + len
    }
}

//
// -- Accounts (contexts) --
//

/// Initialize: create both PDAs with minimal space (empty string / empty bytes).
/// Account order follows your specified ordering:
/// user (signer), string_storage_pda (PDA), bytes_storage_pda (PDA), system_program
#[derive(Accounts)]
pub struct InitializeCtx<'info> {
    #[account(mut)]
    pub user: Signer<'info>,

    #[account(
        init_if_needed,
        payer = user,
        space = MemoryStringPDA::INIT_SPACE,
        seeds = [b"storage_string", user.key().as_ref()],
        bump
    )]
    pub string_storage_pda: Account<'info, MemoryStringPDA>,

    #[account(
        init_if_needed,
        payer = user,
        space = MemoryBytesPDA::INIT_SPACE,
        seeds = [b"storage_bytes", user.key().as_ref()],
        bump
    )]
    pub bytes_storage_pda: Account<'info, MemoryBytesPDA>,

    pub system_program: Program<'info, System>,
}


/// Store string:
/// The struct uses the `#[instruction(...)]` attribute so we can reference the
/// instruction's argument (`data_to_store`) when evaluating `realloc` at validation time.
///
/// Account order follows your specified ordering:
/// user (signer), string_storage_pda (PDA), system_program
#[derive(Accounts)]
#[instruction(data_to_store: String)]
pub struct StoreStringCtx<'info> {
    /// The signer who must pay for any extra rent when reallocating
    #[account(mut)]
    pub user: Signer<'info>,

    /// PDA for storing the String. Reallocated to exactly fit the provided string.
    /// `realloc::payer = user` instructs Anchor to transfer lamports from `user` if extra rent-exempt lamports
    /// are required. `realloc::zero = false` preserves existing bytes not overwritten (faster).
    #[account(
        mut,
        seeds = [b"storage_string", user.key().as_ref()],
        bump,
        realloc = 8 + 4 + data_to_store.len(),
        realloc::payer = user,
        realloc::zero = false
    )]
    pub string_storage_pda: Account<'info, MemoryStringPDA>,

    /// System program required by realloc CPI
    pub system_program: Program<'info, System>,
}

/// Store bytes:
/// Account order: user (signer), bytes_storage_pda (PDA), system_program
#[derive(Accounts)]
#[instruction(data_to_store: Vec<u8>)]
pub struct StoreBytesCtx<'info> {
    /// The signer who must pay for any extra rent when reallocating
    #[account(mut)]
    pub user: Signer<'info>,

    /// PDA for storing the Vec<u8>. Reallocated to exactly fit the provided bytes.
    #[account(
        mut,
        seeds = [b"storage_bytes", user.key().as_ref()],
        bump,
        realloc = 8 + 4 + data_to_store.len(),
        realloc::payer = user,
        realloc::zero = false
    )]
    pub bytes_storage_pda: Account<'info, MemoryBytesPDA>,

    /// System program required by realloc CPI
    pub system_program: Program<'info, System>,
}

