use anchor_lang::prelude::*;

declare_id!("2fXU6zeQrRNcNN2hzUPWmNNY7sq9axMTjJb981tWF1dB");

#[program]
pub mod storage {
    use super::*;

    pub fn initialize(ctx: Context<InitializeCtx>) -> Result<()> {
        // This will work for both initial initialization and reinitialization
        // thanks to init_if_needed
        let string_account = &mut ctx.accounts.string_storage_pda;
        string_account.my_string = String::new(); // Reset to empty
        
        let bytes_account = &mut ctx.accounts.bytes_storage_pda;
        bytes_account.my_bytes = Vec::new(); // Reset to empty
        
        msg!("Storage PDAs initialized/reinitialized");
        Ok(())
    }

    pub fn store_string(ctx: Context<StoreStringCtx>, data_to_store: String) -> Result<()> {
        let string_account = &mut ctx.accounts.string_storage_pda;
        
        // Calculate required space
        let required_space = 8 + 4 + data_to_store.len();
        let account_info = string_account.to_account_info();
        let current_space = account_info.data_len();
        
        if current_space < required_space {
            let new_space = required_space.max(1024);
            let rent = Rent::get()?;
            let new_minimum_balance = rent.minimum_balance(new_space);
            let current_balance = account_info.lamports();
            
            account_info.realloc(new_space, false)?;
            
            if new_minimum_balance > current_balance {
                let additional_lamports = new_minimum_balance - current_balance;
                
                let cpi_accounts = anchor_lang::system_program::Transfer {
                    from: ctx.accounts.user.to_account_info(),
                    to: account_info.clone(),
                };
                let cpi_context = CpiContext::new(
                    ctx.accounts.system_program.to_account_info(), 
                    cpi_accounts
                );
                anchor_lang::system_program::transfer(cpi_context, additional_lamports)?;
            }
        }
        
        string_account.my_string = data_to_store;
        msg!("String stored: {} chars", string_account.my_string.len());
        Ok(())
    }

    pub fn store_bytes(ctx: Context<StoreBytesCtx>, data_to_store: Vec<u8>) -> Result<()> {
        let bytes_account = &mut ctx.accounts.bytes_storage_pda;
        
        // Calculate required space
        let required_space = 8 + 4 + data_to_store.len();
        let account_info = bytes_account.to_account_info();
        let current_space = account_info.data_len();
        
        if current_space < required_space {
            let new_space = required_space.max(1024);
            let rent = Rent::get()?;
            let new_minimum_balance = rent.minimum_balance(new_space);
            let current_balance = account_info.lamports();
            
            account_info.realloc(new_space, false)?;
            
            if new_minimum_balance > current_balance {
                let additional_lamports = new_minimum_balance - current_balance;
                
                let cpi_accounts = anchor_lang::system_program::Transfer {
                    from: ctx.accounts.user.to_account_info(),
                    to: account_info.clone(),
                };
                let cpi_context = CpiContext::new(
                    ctx.accounts.system_program.to_account_info(), 
                    cpi_accounts
                );
                anchor_lang::system_program::transfer(cpi_context, additional_lamports)?;
            }
        }
        
        bytes_account.my_bytes = data_to_store;
        msg!("Bytes stored: {} bytes", bytes_account.my_bytes.len());
        Ok(())
    }
}

#[account]
pub struct MemoryStringPDA {
    pub my_string: String,
}

#[account]
pub struct MemoryBytesPDA {
    pub my_bytes: Vec<u8>,
}

#[derive(Accounts)]
pub struct InitializeCtx<'info> {
    #[account(mut)]
    pub user: Signer<'info>,
    
    // Use init_if_needed to allow reinitialization
    #[account(
        init_if_needed,
        payer = user,
        space = 8 + 4 + 100,
        seeds = [b"storage_string", user.key().as_ref()],
        bump
    )]
    pub string_storage_pda: Account<'info, MemoryStringPDA>,
    
    #[account(
        init_if_needed,
        payer = user,
        space = 8 + 4 + 100,
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