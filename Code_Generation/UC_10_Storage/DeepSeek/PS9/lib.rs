use anchor_lang::prelude::*;
use anchor_lang::system_program;

declare_id!("7mjEb7E3mEbk6BwcL3tcS42qdqQSLQqPLEPDYcoE7SKv");

#[program]
pub mod storage {
    use super::*;

    pub fn initialize(ctx: Context<Initialize>) -> Result<()> {
        msg!("Initializing storage accounts");
        
        // Check if accounts are already initialized by examining their data
        let string_account_info = ctx.accounts.string_storage_pda.to_account_info();
        let bytes_account_info = ctx.accounts.bytes_storage_pda.to_account_info();
        
        // If accounts already have data (more than just discriminator), skip initialization
        if string_account_info.data_len() > 8 && bytes_account_info.data_len() > 8 {
            msg!("Accounts already initialized, skipping");
            return Ok(());
        }

        // Initialize string storage with empty string
        let string_storage = &mut ctx.accounts.string_storage_pda;
        string_storage.my_string = String::new();

        // Initialize bytes storage with empty vector
        let bytes_storage = &mut ctx.accounts.bytes_storage_pda;
        bytes_storage.my_bytes = Vec::new();

        msg!("Storage accounts initialized successfully");
        Ok(())
    }

    pub fn store_string(ctx: Context<StoreString>, data_to_store: String) -> Result<()> {
        msg!("Storing string of length: {}", data_to_store.len());
        
        let account_info = ctx.accounts.string_storage_pda.to_account_info();
        let required_space = 8 + 4 + data_to_store.len();
        let current_space = account_info.data_len();
        
        if current_space < required_space {
            msg!("Resizing from {} to {} bytes", current_space, required_space);
            
            let rent = Rent::get()?;
            let new_minimum_balance = rent.minimum_balance(required_space);
            let current_balance = account_info.lamports();
            
            account_info.realloc(required_space, false)?;
            
            if current_balance < new_minimum_balance {
                let additional_lamports = new_minimum_balance - current_balance;
                system_program::transfer(
                    CpiContext::new(
                        ctx.accounts.system_program.to_account_info(),
                        system_program::Transfer {
                            from: ctx.accounts.user.to_account_info(),
                            to: account_info.clone(),
                        },
                    ),
                    additional_lamports,
                )?;
            }
        }

        let string_storage = &mut ctx.accounts.string_storage_pda;
        string_storage.my_string = data_to_store;
        
        msg!("String stored successfully");
        Ok(())
    }

    pub fn store_bytes(ctx: Context<StoreBytes>, data_to_store: Vec<u8>) -> Result<()> {
        msg!("Storing bytes of length: {}", data_to_store.len());
        
        let account_info = ctx.accounts.bytes_storage_pda.to_account_info();
        let required_space = 8 + 4 + data_to_store.len();
        let current_space = account_info.data_len();
        
        if current_space < required_space {
            msg!("Resizing from {} to {} bytes", current_space, required_space);
            
            let rent = Rent::get()?;
            let new_minimum_balance = rent.minimum_balance(required_space);
            let current_balance = account_info.lamports();
            
            account_info.realloc(required_space, false)?;
            
            if current_balance < new_minimum_balance {
                let additional_lamports = new_minimum_balance - current_balance;
                system_program::transfer(
                    CpiContext::new(
                        ctx.accounts.system_program.to_account_info(),
                        system_program::Transfer {
                            from: ctx.accounts.user.to_account_info(),
                            to: account_info.clone(),
                        },
                    ),
                    additional_lamports,
                )?;
            }
        }

        let bytes_storage = &mut ctx.accounts.bytes_storage_pda;
        bytes_storage.my_bytes = data_to_store;
        
        msg!("Bytes stored successfully");
        Ok(())
    }
}

#[derive(Accounts)]
pub struct Initialize<'info> {
    #[account(mut)]
    pub user: Signer<'info>,
    
    #[account(
        init_if_needed,
        payer = user,
        space = 12,
        seeds = [b"storage_string", user.key().as_ref()],
        bump
    )]
    pub string_storage_pda: Account<'info, MemoryStringPda>,
    
    #[account(
        init_if_needed,
        payer = user,
        space = 12,
        seeds = [b"storage_bytes", user.key().as_ref()],
        bump
    )]
    pub bytes_storage_pda: Account<'info, MemoryBytesPda>,
    
    pub system_program: Program<'info, System>,
}

#[derive(Accounts)]
pub struct StoreString<'info> {
    #[account(mut)]
    pub user: Signer<'info>,
    
    #[account(
        mut,
        seeds = [b"storage_string", user.key().as_ref()],
        bump
    )]
    pub string_storage_pda: Account<'info, MemoryStringPda>,
    
    pub system_program: Program<'info, System>,
}

#[derive(Accounts)]
pub struct StoreBytes<'info> {
    #[account(mut)]
    pub user: Signer<'info>,
    
    #[account(
        mut,
        seeds = [b"storage_bytes", user.key().as_ref()],
        bump
    )]
    pub bytes_storage_pda: Account<'info, MemoryBytesPda>,
    
    pub system_program: Program<'info, System>,
}

#[account]
pub struct MemoryStringPda {
    pub my_string: String,
}

#[account]
pub struct MemoryBytesPda {
    pub my_bytes: Vec<u8>,
}