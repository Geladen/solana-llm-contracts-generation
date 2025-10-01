use anchor_lang::prelude::*;
use anchor_lang::system_program;
use std::mem::size_of;

declare_id!("CLzs2RXZUkgLvXG5gnAeBzjWa9SggDDmZ1n4vxyHDvhF");

#[program]
pub mod storage {
    use super::*;

    /// Initializes both storage PDAs for a user (idempotent - can be called multiple times)
    pub fn initialize(ctx: Context<InitializeCtx>) -> Result<()> {
        let string_storage = &mut ctx.accounts.string_storage_pda;
        let bytes_storage = &mut ctx.accounts.bytes_storage_pda;
        
        // Only initialize if the data is empty (first time initialization)
        if string_storage.my_string.is_empty() {
            string_storage.my_string = String::new();
        }
        
        if bytes_storage.my_bytes.is_empty() {
            bytes_storage.my_bytes = Vec::new();
        }
        
        msg!("Storage PDAs initialized/verified for user: {}", ctx.accounts.user.key());
        Ok(())
    }

    /// Stores a string in the user's string PDA, dynamically resizing account if needed
    pub fn store_string(ctx: Context<StoreStringCtx>, data_to_store: String) -> Result<()> {
        let string_storage = &mut ctx.accounts.string_storage_pda;
        
        // Calculate required space
        let new_space = MemoryStringPDA::calculate_space(&data_to_store);
        let current_space = string_storage.to_account_info().data_len();
        
        // Realloc account if more space needed
        if new_space > current_space {
            let rent = Rent::get()?;
            let required_lamports = rent.minimum_balance(new_space);
            let current_lamports = string_storage.to_account_info().lamports();
            
            if required_lamports > current_lamports {
                let additional_lamports = required_lamports - current_lamports;
                
                // Use Anchor's transfer CPI
                let cpi_context = CpiContext::new(
                    ctx.accounts.system_program.to_account_info(),
                    system_program::Transfer {
                        from: ctx.accounts.user.to_account_info(),
                        to: string_storage.to_account_info().clone(),
                    },
                );
                system_program::transfer(cpi_context, additional_lamports)?;
            }
            
            // Resize the account
            string_storage.to_account_info().realloc(new_space, false)?;
        }
        
        string_storage.my_string = data_to_store;
        msg!("String stored successfully. Length: {}", string_storage.my_string.len());
        Ok(())
    }

    /// Stores bytes in the user's bytes PDA, dynamically resizing account if needed
    pub fn store_bytes(ctx: Context<StoreBytesCtx>, data_to_store: Vec<u8>) -> Result<()> {
        let bytes_storage = &mut ctx.accounts.bytes_storage_pda;
        
        // Calculate required space
        let new_space = MemoryBytesPDA::calculate_space(&data_to_store);
        let current_space = bytes_storage.to_account_info().data_len();
        
        // Realloc account if more space needed
        if new_space > current_space {
            let rent = Rent::get()?;
            let required_lamports = rent.minimum_balance(new_space);
            let current_lamports = bytes_storage.to_account_info().lamports();
            
            if required_lamports > current_lamports {
                let additional_lamports = required_lamports - current_lamports;
                
                // Use Anchor's transfer CPI
                let cpi_context = CpiContext::new(
                    ctx.accounts.system_program.to_account_info(),
                    system_program::Transfer {
                        from: ctx.accounts.user.to_account_info(),
                        to: bytes_storage.to_account_info().clone(),
                    },
                );
                system_program::transfer(cpi_context, additional_lamports)?;
            }
            
            // Resize the account
            bytes_storage.to_account_info().realloc(new_space, false)?;
        }
        
        bytes_storage.my_bytes = data_to_store;
        msg!("Bytes stored successfully. Length: {}", bytes_storage.my_bytes.len());
        Ok(())
    }
}

// Account validation structs
#[derive(Accounts)]
pub struct InitializeCtx<'info> {
    #[account(mut)]
    pub user: Signer<'info>,
    
    #[account(
        init_if_needed,
        payer = user,
        space = MemoryStringPDA::INITIAL_SPACE,
        seeds = [b"storage_string", user.key().as_ref()],
        bump
    )]
    pub string_storage_pda: Account<'info, MemoryStringPDA>,
    
    #[account(
        init_if_needed,
        payer = user,
        space = MemoryBytesPDA::INITIAL_SPACE,
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
        seeds = [b"storage_bytes", user.key().as_ref()],
        bump,
    )]
    pub bytes_storage_pda: Account<'info, MemoryBytesPDA>,
    
    pub system_program: Program<'info, System>,
}

// State structures
#[account]
pub struct MemoryStringPDA {
    pub my_string: String,
}

#[account]
pub struct MemoryBytesPDA {
    pub my_bytes: Vec<u8>,
}

// Implementation with space calculation utilities
impl MemoryStringPDA {
    pub const INITIAL_SPACE: usize = 8 + 4; // Discriminator + empty string overhead
    
    pub fn calculate_space(string: &String) -> usize {
        8 + 4 + string.len() // Discriminator + length prefix + actual string bytes
    }
}

impl MemoryBytesPDA {
    pub const INITIAL_SPACE: usize = 8 + 4; // Discriminator + empty vec overhead
    
    pub fn calculate_space(bytes: &Vec<u8>) -> usize {
        8 + 4 + bytes.len() // Discriminator + length prefix + actual bytes
    }
}