use anchor_lang::prelude::*;
use std::mem;

declare_id!("BYqavhdgy8t5uLPx6ZbXip6FfMLNwFw6YRSPtRq7Hxt3");

#[program]
pub mod storage {
    use super::*;

    /// Initialize storage accounts for a user
    /// Creates both string and bytes PDAs but doesn't store any data initially
    /// Handles reinitialization gracefully by resetting existing accounts
    pub fn initialize(ctx: Context<InitializeCtx>) -> Result<()> {
        // Reset string storage to empty state
        let string_storage = &mut ctx.accounts.string_storage_pda;
        string_storage.my_string = String::new();
        
        // Reset bytes storage to empty state
        let bytes_storage = &mut ctx.accounts.bytes_storage_pda;
        bytes_storage.my_bytes = Vec::new();
        
        Ok(())
    }

    /// Store a string in the user's string PDA with dynamic resizing
    pub fn store_string(ctx: Context<StoreStringCtx>, data_to_store: String) -> Result<()> {
        let string_storage = &mut ctx.accounts.string_storage_pda;
        
        // Calculate required space for the new string
        let current_len = string_storage.my_string.len();
        let new_len = data_to_store.len();
        
        // If new string is larger, we need to reallocate
        if new_len > current_len {
            let additional_space = new_len - current_len;
            let rent = Rent::get()?;
            let additional_lamports = rent.minimum_balance(additional_space);
            
            // Reallocate account with additional space
            let account_info = string_storage.to_account_info();
            let new_space = account_info.data_len() + additional_space;
            
            // Transfer additional rent if needed
            if additional_lamports > 0 {
                let cpi_context = CpiContext::new(
                    ctx.accounts.system_program.to_account_info(),
                    anchor_lang::system_program::Transfer {
                        from: ctx.accounts.user.to_account_info(),
                        to: account_info.clone(),
                    },
                );
                anchor_lang::system_program::transfer(cpi_context, additional_lamports)?;
            }
            
            // Reallocate the account
            account_info.realloc(new_space, false)?;
        }
        
        // Store the new string
        string_storage.my_string = data_to_store;
        
        Ok(())
    }

    /// Store bytes in the user's bytes PDA with dynamic resizing
    pub fn store_bytes(ctx: Context<StoreBytesCtx>, data_to_store: Vec<u8>) -> Result<()> {
        let bytes_storage = &mut ctx.accounts.bytes_storage_pda;
        
        // Calculate required space for the new byte array
        let current_len = bytes_storage.my_bytes.len();
        let new_len = data_to_store.len();
        
        // If new byte array is larger, we need to reallocate
        if new_len > current_len {
            let additional_space = new_len - current_len;
            let rent = Rent::get()?;
            let additional_lamports = rent.minimum_balance(additional_space);
            
            // Reallocate account with additional space
            let account_info = bytes_storage.to_account_info();
            let new_space = account_info.data_len() + additional_space;
            
            // Transfer additional rent if needed
            if additional_lamports > 0 {
                let cpi_context = CpiContext::new(
                    ctx.accounts.system_program.to_account_info(),
                    anchor_lang::system_program::Transfer {
                        from: ctx.accounts.user.to_account_info(),
                        to: account_info.clone(),
                    },
                );
                anchor_lang::system_program::transfer(cpi_context, additional_lamports)?;
            }
            
            // Reallocate the account
            account_info.realloc(new_space, false)?;
        }
        
        // Store the new byte array
        bytes_storage.my_bytes = data_to_store;
        
        Ok(())
    }
}

// Context Structures

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

#[derive(Accounts)]
pub struct StoreStringCtx<'info> {
    #[account(mut)]
    pub user: Signer<'info>,

    #[account(
        mut,
        seeds = [b"storage_string", user.key().as_ref()],
        bump
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
        seeds = [b"storage_bytes", user.key().as_ref()],
        bump
    )]
    pub bytes_storage_pda: Account<'info, MemoryBytesPDA>,

    pub system_program: Program<'info, System>,
}

// Account State Structures

#[account]
pub struct MemoryStringPDA {
    pub my_string: String,
}

impl MemoryStringPDA {
    // Initial space: discriminator (8) + string length prefix (4) + initial empty string (0)
    pub const INIT_SPACE: usize = 8 + 4 + 0;
    
    // Calculate space needed for a string of given length
    pub fn space_for_string(string_len: usize) -> usize {
        8 + 4 + string_len
    }
}

#[account]
pub struct MemoryBytesPDA {
    pub my_bytes: Vec<u8>,
}

impl MemoryBytesPDA {
    // Initial space: discriminator (8) + vector length prefix (4) + initial empty vector (0)
    pub const INIT_SPACE: usize = 8 + 4 + 0;
    
    // Calculate space needed for a byte array of given length
    pub fn space_for_bytes(bytes_len: usize) -> usize {
        8 + 4 + bytes_len
    }
}

// Error Definitions
#[error_code]
pub enum StorageError {
    #[msg("Storage operation failed")]
    StorageOperationFailed,
    
    #[msg("Insufficient funds for storage expansion")]
    InsufficientFunds,
    
    #[msg("Invalid data length")]
    InvalidDataLength,
}

// Additional helper functions for efficient operations
impl MemoryStringPDA {
    /// Get the current storage capacity
    pub fn get_capacity(&self) -> usize {
        self.my_string.capacity()
    }
    
    /// Check if string is empty
    pub fn is_empty(&self) -> bool {
        self.my_string.is_empty()
    }
    
    /// Get string length
    pub fn len(&self) -> usize {
        self.my_string.len()
    }
}

impl MemoryBytesPDA {
    /// Get the current storage capacity
    pub fn get_capacity(&self) -> usize {
        self.my_bytes.capacity()
    }
    
    /// Check if bytes array is empty
    pub fn is_empty(&self) -> bool {
        self.my_bytes.is_empty()
    }
    
    /// Get bytes array length
    pub fn len(&self) -> usize {
        self.my_bytes.len()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use anchor_lang::prelude::*;

    #[test]
    fn test_string_pda_space_calculation() {
        assert_eq!(MemoryStringPDA::INIT_SPACE, 12); // 8 + 4 + 0
        assert_eq!(MemoryStringPDA::space_for_string(10), 22); // 8 + 4 + 10
    }

    #[test]
    fn test_bytes_pda_space_calculation() {
        assert_eq!(MemoryBytesPDA::INIT_SPACE, 12); // 8 + 4 + 0
        assert_eq!(MemoryBytesPDA::space_for_bytes(20), 32); // 8 + 4 + 20
    }
}