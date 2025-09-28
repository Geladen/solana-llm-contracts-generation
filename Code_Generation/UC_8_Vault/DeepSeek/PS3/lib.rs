// programs/vault-program/src/lib.rs
use anchor_lang::prelude::*;
use anchor_lang::system_program;

declare_id!("8UejKXHZycEBBAwhacZeF56Rbr278ycVKefSi9UoRUh8");

#[program]
pub mod vault {
    use super::*;

    pub fn initialize(
        ctx: Context<Initialize>,
        wait_time: u64,
        initial_amount: u64,
    ) -> Result<()> {
        let vault_info = &mut ctx.accounts.vault_info;
        
        // Initialize vault state
        vault_info.owner = ctx.accounts.owner.key();
        vault_info.recovery = ctx.accounts.recovery.key();
        vault_info.receiver = Pubkey::default();
        vault_info.wait_time = wait_time;
        vault_info.request_time = 0;
        vault_info.amount = 0;
        vault_info.state = State::Idle;

        // Transfer initial funds to vault PDA
        let cpi_context = CpiContext::new(
            ctx.accounts.system_program.to_account_info(),
            system_program::Transfer {
                from: ctx.accounts.owner.to_account_info(),
                to: ctx.accounts.vault_info.to_account_info(),
            },
        );
        system_program::transfer(cpi_context, initial_amount)?;

        msg!("Vault initialized with {} lamports", initial_amount);
        msg!("Wait time: {} slots", wait_time);
        
        Ok(())
    }

    pub fn withdraw(ctx: Context<Withdraw>, amount: u64) -> Result<()> {
        // Get vault balance before mutable borrow
        let vault_balance = ctx.accounts.vault_info.to_account_info().lamports();
        
        let vault_info = &mut ctx.accounts.vault_info;
        
        // Validate state transition
        require!(vault_info.state == State::Idle, VaultError::InvalidState);
        
        // Check vault has sufficient balance
        require!(vault_balance >= amount, VaultError::InsufficientFunds);
        
        // Set withdrawal request
        vault_info.receiver = ctx.accounts.receiver.key();
        vault_info.amount = amount;
        vault_info.request_time = Clock::get()?.slot;
        vault_info.state = State::Req;
        
        msg!("Withdrawal request created for {} lamports", amount);
        msg!("Receiver: {}", vault_info.receiver);
        msg!("Request time: {} slots", vault_info.request_time);
        
        Ok(())
    }

    pub fn finalize(ctx: Context<Finalize>) -> Result<()> {
        // Create a reference to read values before mutable borrow
        let vault_info_ref = &ctx.accounts.vault_info;
        
        // Get values needed for validation
        let vault_state = vault_info_ref.state;
        let vault_request_time = vault_info_ref.request_time;
        let vault_wait_time = vault_info_ref.wait_time;
        let vault_receiver = vault_info_ref.receiver;
        let vault_amount = vault_info_ref.amount;
        let current_slot = Clock::get()?.slot;
        
        // Validate state and timing
        require!(vault_state == State::Req, VaultError::InvalidState);
        
        let elapsed_slots = current_slot.checked_sub(vault_request_time)
            .ok_or(VaultError::InvalidTimeCalculation)?;
        
        require!(elapsed_slots >= vault_wait_time, VaultError::WaitTimeNotElapsed);
        require!(vault_receiver == ctx.accounts.receiver.key(), VaultError::InvalidReceiver);

        // Now do mutable operations
        let vault_info = &mut ctx.accounts.vault_info;
        
        // Transfer funds from vault to receiver
        let vault_account_info = vault_info.to_account_info();
        let receiver_account_info = ctx.accounts.receiver.to_account_info();
        
        **vault_account_info.try_borrow_mut_lamports()? = vault_account_info
            .lamports()
            .checked_sub(vault_amount)
            .ok_or(VaultError::InsufficientFunds)?;
            
        **receiver_account_info.try_borrow_mut_lamports()? = receiver_account_info
            .lamports()
            .checked_add(vault_amount)
            .ok_or(VaultError::TransferFailed)?;

        // Reset vault state
        vault_info.state = State::Idle;
        vault_info.amount = 0;
        vault_info.request_time = 0;
        vault_info.receiver = Pubkey::default();
        
        msg!("Withdrawal finalized: {} lamports transferred", vault_amount);
        
        Ok(())
    }

    pub fn cancel(ctx: Context<Cancel>) -> Result<()> {
        let vault_info = &mut ctx.accounts.vault_info;
        
        // Validate state
        require!(vault_info.state == State::Req, VaultError::InvalidState);
        
        // Reset withdrawal request
        vault_info.state = State::Idle;
        vault_info.amount = 0;
        vault_info.request_time = 0;
        vault_info.receiver = Pubkey::default();
        
        msg!("Withdrawal request cancelled by recovery key");
        
        Ok(())
    }
}

#[derive(Accounts)]
pub struct Initialize<'info> {
    #[account(mut)]
    pub owner: Signer<'info>,
    
    /// CHECK: This is the recovery key pubkey
    pub recovery: AccountInfo<'info>,
    
    #[account(
        init,
        payer = owner,
        space = VaultInfo::SIZE,
        seeds = [owner.key().as_ref()],
        bump
    )]
    pub vault_info: Account<'info, VaultInfo>,
    
    pub system_program: Program<'info, System>,
}

#[derive(Accounts)]
pub struct Withdraw<'info> {
    #[account(mut)]
    pub owner: Signer<'info>,
    
    /// CHECK: Receiver account validated during finalization
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
pub struct Finalize<'info> {
    #[account(mut)]
    pub owner: Signer<'info>,
    
    #[account(mut)]
    /// CHECK: Receiver account validated against vault_info.receiver
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
pub struct Cancel<'info> {
    pub recovery: Signer<'info>,
    
    /// CHECK: Owner account used for PDA derivation
    pub owner: AccountInfo<'info>,
    
    #[account(
        mut,
        seeds = [owner.key().as_ref()],
        bump,
        constraint = vault_info.recovery == recovery.key() @ VaultError::Unauthorized
    )]
    pub vault_info: Account<'info, VaultInfo>,
}

#[account]
pub struct VaultInfo {
    pub owner: Pubkey,
    pub recovery: Pubkey,
    pub receiver: Pubkey,
    pub wait_time: u64,
    pub request_time: u64,
    pub amount: u64,
    pub state: State,
}

#[derive(AnchorSerialize, AnchorDeserialize, Clone, Copy, PartialEq, Eq)]
pub enum State {
    Idle = 0,
    Req = 1,
}

impl VaultInfo {
    pub const SIZE: usize = 8 + // discriminator
        32 + // owner
        32 + // recovery  
        32 + // receiver
        8 +  // wait_time
        8 +  // request_time
        8 +  // amount
        1;   // state
}

#[error_code]
pub enum VaultError {
    #[msg("Invalid state transition")]
    InvalidState,
    #[msg("Insufficient funds in vault")]
    InsufficientFunds,
    #[msg("Wait time has not elapsed")]
    WaitTimeNotElapsed,
    #[msg("Unauthorized access")]
    Unauthorized,
    #[msg("Invalid receiver account")]
    InvalidReceiver,
    #[msg("Invalid time calculation")]
    InvalidTimeCalculation,
    #[msg("Fund transfer failed")]
    TransferFailed,
}