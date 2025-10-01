use anchor_lang::prelude::*;

declare_id!("GcaSzKK3XHsY1PvLoU91Hr5KnUm7VN5yuv61tomVNCu8");


#[program]
pub mod storage {
    use super::*;

    /// Initialize the storage accounts for a user
    /// Creates both string and bytes PDA accounts with empty data
    pub fn initialize(ctx: Context<InitializeCtx>) -> Result<()> {
        // Anchor automatically initializes the account data with Default values
        // No explicit initialization needed since we derive Default
        Ok(())
    }

    /// Store a string in the user's string PDA with dynamic resizing
    pub fn store_string(ctx: Context<StoreStringCtx>, data_to_store: String) -> Result<()> {
        let string_pda = &mut ctx.accounts.string_storage_pda;
        
        // Calculate required space: 8 bytes discriminator + 4 bytes string length + string data
        let required_space = 8 + 4 + data_to_store.len();
        let current_space = string_pda.to_account_info().data_len();
        
        // Resize account if needed
        if required_space > current_space {
            let rent = Rent::get()?;
            let new_minimum_balance = rent.minimum_balance(required_space);
            let current_balance = string_pda.to_account_info().lamports();
            
            if new_minimum_balance > current_balance {
                let additional_rent = new_minimum_balance - current_balance;
                
                // Transfer additional rent from user
                let cpi_context = CpiContext::new(
                    ctx.accounts.system_program.to_account_info(),
                    anchor_lang::system_program::Transfer {
                        from: ctx.accounts.user.to_account_info(),
                        to: string_pda.to_account_info(),
                    },
                );
                anchor_lang::system_program::transfer(cpi_context, additional_rent)?;
            }
            
            // Resize the account
            string_pda.to_account_info().realloc(required_space, false)?;
        }
        
        // Store the string data
        string_pda.my_string = data_to_store;
        
        Ok(())
    }

    /// Store bytes in the user's bytes PDA with dynamic resizing
    pub fn store_bytes(ctx: Context<StoreBytesCtx>, data_to_store: Vec<u8>) -> Result<()> {
        let bytes_pda = &mut ctx.accounts.bytes_storage_pda;
        
        // Calculate required space: 8 bytes discriminator + 4 bytes vec length + byte data
        let required_space = 8 + 4 + data_to_store.len();
        let current_space = bytes_pda.to_account_info().data_len();
        
        // Resize account if needed
        if required_space > current_space {
            let rent = Rent::get()?;
            let new_minimum_balance = rent.minimum_balance(required_space);
            let current_balance = bytes_pda.to_account_info().lamports();
            
            if new_minimum_balance > current_balance {
                let additional_rent = new_minimum_balance - current_balance;
                
                // Transfer additional rent from user
                let cpi_context = CpiContext::new(
                    ctx.accounts.system_program.to_account_info(),
                    anchor_lang::system_program::Transfer {
                        from: ctx.accounts.user.to_account_info(),
                        to: bytes_pda.to_account_info(),
                    },
                );
                anchor_lang::system_program::transfer(cpi_context, additional_rent)?;
            }
            
            // Resize the account
            bytes_pda.to_account_info().realloc(required_space, false)?;
        }
        
        // Store the byte data
        bytes_pda.my_bytes = data_to_store;
        
        Ok(())
    }
}

#[derive(Accounts)]
pub struct InitializeCtx<'info> {
    #[account(mut)]
    pub user: Signer<'info>,
    
    #[account(
        init_if_needed,
        payer = user,
        space = 8 + 4, // 8 bytes discriminator + 4 bytes for empty string length
        seeds = [b"storage_string", user.key().as_ref()],
        bump
    )]
    pub string_storage_pda: Account<'info, MemoryStringPDA>,
    
    #[account(
        init_if_needed,
        payer = user,
        space = 8 + 4, // 8 bytes discriminator + 4 bytes for empty vec length
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
pub enum StorageError {
    #[msg("Insufficient funds for account resizing")]
    InsufficientFunds,
    #[msg("Account resize failed")]
    ResizeFailed,
}