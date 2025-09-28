use anchor_lang::prelude::*;
use anchor_lang::system_program;

declare_id!("42hzGSkezZt7zJabtTcjqzK3TLFpWsQSmBUW6bM7XgXw");

#[program]
pub mod vault {
    use super::*;

    pub fn initialize(
        ctx: Context<Initialize>,
        wait_time: u64,
        initial_amount: u64,
    ) -> Result<()> {
        let vault_info = &mut ctx.accounts.vault_info;
        
        vault_info.owner = ctx.accounts.owner.key();
        vault_info.recovery = ctx.accounts.recovery.key();
        vault_info.receiver = Pubkey::default();
        vault_info.wait_time = wait_time; // This will be waitTime in TypeScript
        vault_info.request_time = 0;      // This will be requestTime in TypeScript
        vault_info.amount = 0;
        vault_info.state = VaultState::Idle;

        if initial_amount > 0 {
            let cpi_context = CpiContext::new(
                ctx.accounts.system_program.to_account_info(),
                system_program::Transfer {
                    from: ctx.accounts.owner.to_account_info(),
                    to: ctx.accounts.vault_info.to_account_info(),
                },
            );
            system_program::transfer(cpi_context, initial_amount)?;
        }

        Ok(())
    }

    pub fn withdraw(ctx: Context<Withdraw>, amount: u64) -> Result<()> {
        let vault_account_info = ctx.accounts.vault_info.to_account_info();
        let vault_balance = vault_account_info.lamports();
        
        let vault_info = &mut ctx.accounts.vault_info;
        
        require!(vault_info.state == VaultState::Idle, VaultError::InvalidState);
        require!(amount > 0, VaultError::InvalidAmount);
        require!(vault_balance >= amount, VaultError::InsufficientFunds);

        vault_info.state = VaultState::Pending;
        vault_info.amount = amount;
        vault_info.receiver = ctx.accounts.receiver.key();
        vault_info.request_time = Clock::get()?.slot; // Store current slot

        Ok(())
    }

    pub fn finalize(ctx: Context<Finalize>) -> Result<()> {
        let amount_to_transfer;
        let receiver_key;
        
        {
            let vault_info = &ctx.accounts.vault_info;
            
            require!(vault_info.state == VaultState::Pending, VaultError::InvalidState);
            
            let current_slot = Clock::get()?.slot;
            let elapsed_slots = current_slot.saturating_sub(vault_info.request_time);
            require!(elapsed_slots >= vault_info.wait_time, VaultError::WaitTimeNotElapsed);
            require!(
                vault_info.receiver == ctx.accounts.receiver.key(),
                VaultError::InvalidReceiver
            );
            
            amount_to_transfer = vault_info.amount;
            receiver_key = vault_info.receiver;
        }

        let vault_account_info = ctx.accounts.vault_info.to_account_info();
        let receiver_account_info = ctx.accounts.receiver.to_account_info();
        
        require!(
            receiver_key == receiver_account_info.key(),
            VaultError::InvalidReceiver
        );

        **vault_account_info.try_borrow_mut_lamports()? = vault_account_info
            .lamports()
            .checked_sub(amount_to_transfer)
            .ok_or(VaultError::InsufficientFunds)?;
            
        **receiver_account_info.try_borrow_mut_lamports()? = receiver_account_info
            .lamports()
            .checked_add(amount_to_transfer)
            .ok_or(VaultError::TransferFailed)?;

        let vault_info = &mut ctx.accounts.vault_info;
        vault_info.state = VaultState::Idle;
        vault_info.amount = 0;
        vault_info.request_time = 0;
        vault_info.receiver = Pubkey::default();

        Ok(())
    }

    pub fn cancel(ctx: Context<Cancel>) -> Result<()> {
        let vault_info = &mut ctx.accounts.vault_info;
        
        require!(vault_info.state == VaultState::Pending, VaultError::InvalidState);

        vault_info.state = VaultState::Idle;
        vault_info.amount = 0;
        vault_info.request_time = 0;
        vault_info.receiver = Pubkey::default();

        Ok(())
    }
}

#[derive(Accounts)]
pub struct Initialize<'info> {
    #[account(mut)]
    pub owner: Signer<'info>,
    
    /// CHECK: This is the recovery key that can cancel withdrawals
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
    
    /// CHECK: The receiver account that will receive funds when finalizing
    pub receiver: AccountInfo<'info>,
    
    #[account(
        mut,
        seeds = [owner.key().as_ref()],
        bump,
        has_one = owner @ VaultError::InvalidOwner
    )]
    pub vault_info: Account<'info, VaultInfo>,
}

#[derive(Accounts)]
pub struct Finalize<'info> {
    #[account(mut)]
    pub owner: Signer<'info>,
    
    #[account(mut)]
    /// CHECK: Receiver account validated in instruction logic
    pub receiver: AccountInfo<'info>,
    
    #[account(
        mut,
        seeds = [owner.key().as_ref()],
        bump,
        has_one = owner @ VaultError::InvalidOwner
    )]
    pub vault_info: Account<'info, VaultInfo>,
}

#[derive(Accounts)]
pub struct Cancel<'info> {
    pub recovery: Signer<'info>,
    
    /// CHECK: Owner account reference for PDA derivation
    pub owner: AccountInfo<'info>,
    
    #[account(
        mut,
        seeds = [owner.key().as_ref()],
        bump,
        has_one = recovery @ VaultError::InvalidRecovery
    )]
    pub vault_info: Account<'info, VaultInfo>,
}

#[account]
pub struct VaultInfo {
    pub owner: Pubkey,           // 32 bytes - becomes 'owner' in TypeScript
    pub recovery: Pubkey,        // 32 bytes - becomes 'recovery' in TypeScript  
    pub receiver: Pubkey,        // 32 bytes - becomes 'receiver' in TypeScript
    pub wait_time: u64,          // 8 bytes - becomes 'waitTime' in TypeScript
    pub request_time: u64,       // 8 bytes - becomes 'requestTime' in TypeScript
    pub amount: u64,             // 8 bytes - becomes 'amount' in TypeScript
    pub state: VaultState,       // 1 byte - becomes 'state' in TypeScript
}

impl VaultInfo {
    pub const SIZE: usize = 8 + 32 + 32 + 32 + 8 + 8 + 8 + 1;
}

#[derive(AnchorSerialize, AnchorDeserialize, Clone, PartialEq, Eq)]
pub enum VaultState {
    Idle = 0,
    Pending = 1,
}

impl Default for VaultState {
    fn default() -> Self {
        VaultState::Idle
    }
}

#[error_code]
pub enum VaultError {
    #[msg("Invalid vault state for this operation")]
    InvalidState,
    #[msg("Invalid amount specified")]
    InvalidAmount,
    #[msg("Insufficient funds in vault")]
    InsufficientFunds,
    #[msg("Wait time has not elapsed")]
    WaitTimeNotElapsed,
    #[msg("Invalid receiver account")]
    InvalidReceiver,
    #[msg("Invalid owner account")]
    InvalidOwner,
    #[msg("Invalid recovery account")]
    InvalidRecovery,
    #[msg("Fund transfer failed")]
    TransferFailed,
}