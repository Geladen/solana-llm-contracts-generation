use anchor_lang::prelude::*;
use anchor_lang::system_program;

declare_id!("5P6EGZg8QhHeeNFfadHBJ9HQFRtUKjgPcWuH3CqefVbz");

#[program]
pub mod vault {
    use super::*;

    /// Initialize a new time-locked vault
    /// Only the owner can call this function
    pub fn initialize(
        ctx: Context<InitializeCtx>,
        wait_time: u64,
        initial_amount: u64,
    ) -> Result<()> {
        let vault_info = &mut ctx.accounts.vault_info;
        let owner = &ctx.accounts.owner;
        let recovery = &ctx.accounts.recovery;

        // Initialize vault info
        vault_info.owner = owner.key();
        vault_info.recovery = recovery.key();
        vault_info.receiver = Pubkey::default();
        vault_info.wait_time = wait_time;
        vault_info.request_time = 0;
        vault_info.amount = 0;
        vault_info.state = State::Idle;

        // Transfer initial amount from owner to vault PDA
        if initial_amount > 0 {
            let transfer_ix = system_program::Transfer {
                from: owner.to_account_info(),
                to: vault_info.to_account_info(),
            };
            
            let cpi_ctx = CpiContext::new(
                ctx.accounts.system_program.to_account_info(),
                transfer_ix,
            );
            
            system_program::transfer(cpi_ctx, initial_amount)?;
        }

        msg!("Vault initialized with {} lamports, wait time: {} slots", initial_amount, wait_time);
        Ok(())
    }

    /// Create a withdrawal request
    /// Only the owner can call this function
    pub fn withdraw(ctx: Context<WithdrawCtx>, amount: u64) -> Result<()> {
        let vault_info = &mut ctx.accounts.vault_info;
        let receiver = &ctx.accounts.receiver;
        let clock = Clock::get()?;

        // Check vault state
        require!(vault_info.state == State::Idle, VaultError::InvalidState);

        // Check if vault has sufficient funds
        let vault_balance = vault_info.to_account_info().lamports();
        require!(vault_balance >= amount, VaultError::InsufficientFunds);

        // Set withdrawal request
        vault_info.receiver = receiver.key();
        vault_info.request_time = clock.slot;
        vault_info.amount = amount;
        vault_info.state = State::Req;

        msg!("Withdrawal request created: {} lamports to {}, request time: {}", 
             amount, receiver.key(), clock.slot);
        Ok(())
    }

    /// Finalize withdrawal after wait time has elapsed
    /// Only the owner can call this function
    pub fn finalize(ctx: Context<FinalizeCtx>) -> Result<()> {
        let vault_info = &mut ctx.accounts.vault_info;
        let receiver = &mut ctx.accounts.receiver;
        let owner = &ctx.accounts.owner;
        let clock = Clock::get()?;

        // Check vault state
        require!(vault_info.state == State::Req, VaultError::InvalidState);

        // Check if receiver matches the one in the withdrawal request
        require!(vault_info.receiver == receiver.key(), VaultError::InvalidReceiver);

        // Check if wait time has elapsed
        let elapsed_time = clock.slot.saturating_sub(vault_info.request_time);
        require!(elapsed_time >= vault_info.wait_time, VaultError::WaitTimeNotElapsed);

        // Transfer funds from vault to receiver
        let amount = vault_info.amount;
        let vault_balance = vault_info.to_account_info().lamports();
        require!(vault_balance >= amount, VaultError::InsufficientFunds);

        // Create PDA seeds for CPI
        let owner_key = vault_info.owner;
        let seeds = &[owner_key.as_ref()];
        let (_, bump) = Pubkey::find_program_address(seeds, ctx.program_id);
        let signer_seeds = &[&[owner_key.as_ref(), &[bump]][..]];

        // Transfer lamports from vault PDA to receiver
        **vault_info.to_account_info().try_borrow_mut_lamports()? -= amount;
        **receiver.to_account_info().try_borrow_mut_lamports()? += amount;

        // Reset vault state
        vault_info.receiver = Pubkey::default();
        vault_info.request_time = 0;
        vault_info.amount = 0;
        vault_info.state = State::Idle;

        msg!("Withdrawal finalized: {} lamports transferred to {}", amount, receiver.key());
        Ok(())
    }

    /// Cancel pending withdrawal request
    /// Only the recovery key can call this function
    pub fn cancel(ctx: Context<CancelCtx>) -> Result<()> {
        let vault_info = &mut ctx.accounts.vault_info;

        // Check vault state
        require!(vault_info.state == State::Req, VaultError::InvalidState);

        // Reset withdrawal request
        let amount = vault_info.amount;
        let receiver = vault_info.receiver;
        
        vault_info.receiver = Pubkey::default();
        vault_info.request_time = 0;
        vault_info.amount = 0;
        vault_info.state = State::Idle;

        msg!("Withdrawal request cancelled: {} lamports to {} cancelled by recovery key", 
             amount, receiver);
        Ok(())
    }
}

// Context Structs

#[derive(Accounts)]
pub struct InitializeCtx<'info> {
    #[account(mut)]
    pub owner: Signer<'info>,
    
    /// CHECK: Recovery key is validated in program logic
    pub recovery: AccountInfo<'info>,
    
    #[account(
        init,
        payer = owner,
        space = 8 + VaultInfo::INIT_SPACE,
        seeds = [owner.key().as_ref()],
        bump
    )]
    pub vault_info: Account<'info, VaultInfo>,
    
    pub system_program: Program<'info, System>,
}

#[derive(Accounts)]
pub struct WithdrawCtx<'info> {
    #[account(mut)]
    pub owner: Signer<'info>,
    
    /// CHECK: Receiver is validated in program logic
    pub receiver: AccountInfo<'info>,
    
    #[account(
        mut,
        seeds = [owner.key().as_ref()],
        bump,
        constraint = vault_info.owner == owner.key() @ VaultError::Unauthorized
    )]
    pub vault_info: Account<'info, VaultInfo>,
}

#[derive(Accounts)]
pub struct FinalizeCtx<'info> {
    #[account(mut)]
    pub owner: Signer<'info>,
    
    /// CHECK: Receiver is validated in program logic and made mutable for lamport transfer
    #[account(mut)]
    pub receiver: AccountInfo<'info>,
    
    #[account(
        mut,
        seeds = [owner.key().as_ref()],
        bump,
        constraint = vault_info.owner == owner.key() @ VaultError::Unauthorized
    )]
    pub vault_info: Account<'info, VaultInfo>,
}

#[derive(Accounts)]
pub struct CancelCtx<'info> {
    #[account(mut)]
    pub recovery: Signer<'info>,
    
    /// CHECK: Owner is validated in program logic
    pub owner: AccountInfo<'info>,
    
    #[account(
        mut,
        seeds = [owner.key().as_ref()],
        bump,
        constraint = vault_info.recovery == recovery.key() @ VaultError::Unauthorized,
        constraint = vault_info.owner == owner.key() @ VaultError::InvalidOwner
    )]
    pub vault_info: Account<'info, VaultInfo>,
}

// Data Structures

#[account]
#[derive(InitSpace)]
pub struct VaultInfo {
    pub owner: Pubkey,        // 32 bytes
    pub recovery: Pubkey,     // 32 bytes  
    pub receiver: Pubkey,     // 32 bytes
    pub wait_time: u64,       // 8 bytes
    pub request_time: u64,    // 8 bytes
    pub amount: u64,          // 8 bytes
    pub state: State,         // 1 byte + padding
}

#[derive(AnchorSerialize, AnchorDeserialize, Clone, PartialEq, Eq, InitSpace)]
pub enum State {
    Idle,  // 0
    Req,   // 1
}

// Error Types

#[error_code]
pub enum VaultError {
    #[msg("Invalid vault state")]
    InvalidState,
    #[msg("Insufficient funds in vault")]
    InsufficientFunds,
    #[msg("Wait time has not elapsed")]
    WaitTimeNotElapsed,
    #[msg("Invalid receiver")]
    InvalidReceiver,
    #[msg("Unauthorized")]
    Unauthorized,
    #[msg("Invalid owner")]
    InvalidOwner,
}