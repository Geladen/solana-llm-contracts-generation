use anchor_lang::prelude::*;
use borsh::{BorshDeserialize, BorshSerialize};

declare_id!("4X2UhEnx9UbCNdRbECsyYavwQk7W6dYoJFHuS7StPyfr");

#[program]
pub mod vault {
    use super::*;

    pub fn initialize(
        ctx: Context<InitializeCtx>, 
        wait_time: u64, 
        initial_amount: u64
    ) -> Result<()> {
        require!(wait_time > 0, VaultError::InvalidWaitTime);
        require!(initial_amount > 0, VaultError::InvalidAmount);

        let vault_info = &mut ctx.accounts.vault_info;
        
        // Initialize vault state
        vault_info.owner = ctx.accounts.owner.key();
        vault_info.recovery = ctx.accounts.recovery.key();
        vault_info.receiver = Pubkey::default(); // Will be set during withdrawal
        vault_info.wait_time = wait_time;
        vault_info.request_time = 0;
        vault_info.amount = 0;
        vault_info.state = State::Idle;

        // Transfer initial funds to vault
        let ix = anchor_lang::system_program::Transfer {
            from: ctx.accounts.owner.to_account_info(),
            to: ctx.accounts.vault_info.to_account_info(),
        };
        
        let cpi_ctx = CpiContext::new(
            ctx.accounts.system_program.to_account_info(),
            ix,
        );
        
        anchor_lang::system_program::transfer(cpi_ctx, initial_amount)?;

        msg!("Vault initialized with {} lamports, wait time: {} slots", 
             initial_amount, wait_time);

        Ok(())
    }

    pub fn withdraw(ctx: Context<WithdrawCtx>, amount: u64) -> Result<()> {
        require!(amount > 0, VaultError::InvalidAmount);
        
        let vault_info = &mut ctx.accounts.vault_info;
        let clock = Clock::get()?;

        // Verify vault is in idle state
        require!(vault_info.state == State::Idle, VaultError::InvalidState);

        // Check sufficient balance
        let vault_balance = vault_info.to_account_info().lamports();
        require!(vault_balance >= amount, VaultError::InsufficientFunds);

        // Set withdrawal request using slot-based timing
        vault_info.receiver = ctx.accounts.receiver.key();
        vault_info.amount = amount;
        vault_info.request_time = clock.slot;
        vault_info.state = State::Req;

        msg!("Withdrawal request initiated for {} lamports to {}, wait until slot: {}", 
             amount, 
             vault_info.receiver,
             vault_info.request_time + vault_info.wait_time);

        Ok(())
    }

    pub fn finalize(ctx: Context<FinalizeCtx>) -> Result<()> {
        let vault_info = &mut ctx.accounts.vault_info;
        let clock = Clock::get()?;

        // Verify vault is in request state
        require!(vault_info.state == State::Req, VaultError::InvalidState);

        // Verify receiver matches the one in request
        require!(
            vault_info.receiver == ctx.accounts.receiver.key(),
            VaultError::InvalidReceiver
        );

        // Check if wait time has elapsed using slot-based timing
        let current_slot = clock.slot;
        let required_slot = vault_info.request_time + vault_info.wait_time;
        require!(current_slot >= required_slot, VaultError::WaitTimeNotElapsed);

        // Transfer funds
        let amount = vault_info.amount;
        let vault_account_info = vault_info.to_account_info();
        let vault_lamports = vault_account_info.lamports();
        require!(vault_lamports >= amount, VaultError::InsufficientFunds);

        **vault_account_info.try_borrow_mut_lamports()? -= amount;
        **ctx.accounts.receiver.try_borrow_mut_lamports()? += amount;

        // Reset vault state
        vault_info.receiver = Pubkey::default();
        vault_info.amount = 0;
        vault_info.request_time = 0;
        vault_info.state = State::Idle;

        msg!("Withdrawal finalized: {} lamports transferred to {}", 
             amount, 
             ctx.accounts.receiver.key());

        Ok(())
    }

    pub fn cancel(ctx: Context<CancelCtx>) -> Result<()> {
        let vault_info = &mut ctx.accounts.vault_info;

        // Verify vault is in request state
        require!(vault_info.state == State::Req, VaultError::InvalidState);

        // Reset vault state without transferring funds
        vault_info.receiver = Pubkey::default();
        vault_info.amount = 0;
        vault_info.request_time = 0;
        vault_info.state = State::Idle;

        msg!("Withdrawal request cancelled by recovery key");

        Ok(())
    }
}

// Context structures
#[derive(Accounts)]
pub struct InitializeCtx<'info> {
    #[account(mut)]
    pub owner: Signer<'info>,
    /// CHECK: Recovery key address validation
    pub recovery: UncheckedAccount<'info>,
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
    #[account(
        constraint = vault_info.owner == owner.key() @ VaultError::Unauthorized
    )]
    pub owner: Signer<'info>,
    /// CHECK: Receiver address validation
    pub receiver: UncheckedAccount<'info>,
    #[account(
        mut,
        seeds = [owner.key().as_ref()],
        bump
    )]
    pub vault_info: Account<'info, VaultInfo>,
}

#[derive(Accounts)]
pub struct FinalizeCtx<'info> {
    #[account(
        constraint = vault_info.owner == owner.key() @ VaultError::Unauthorized
    )]
    pub owner: Signer<'info>,
    #[account(mut)]
    /// CHECK: Receiver validation done in instruction logic
    pub receiver: UncheckedAccount<'info>,
    #[account(
        mut,
        seeds = [vault_info.owner.as_ref()],
        bump
    )]
    pub vault_info: Account<'info, VaultInfo>,
}

#[derive(Accounts)]
pub struct CancelCtx<'info> {
    #[account(
        constraint = vault_info.recovery == recovery.key() @ VaultError::Unauthorized
    )]
    pub recovery: Signer<'info>,
    /// CHECK: Owner reference validation
    pub owner: UncheckedAccount<'info>,
    #[account(
        mut,
        seeds = [owner.key().as_ref()],
        bump
    )]
    pub vault_info: Account<'info, VaultInfo>,
}

// Data structures
#[account]
pub struct VaultInfo {
    pub owner: Pubkey,      // 32 bytes
    pub recovery: Pubkey,   // 32 bytes
    pub receiver: Pubkey,   // 32 bytes
    pub wait_time: u64,     // 8 bytes
    pub request_time: u64,  // 8 bytes
    pub amount: u64,        // 8 bytes
    pub state: State,       // 1 byte
}

impl VaultInfo {
    pub const INIT_SPACE: usize = 32 + 32 + 32 + 8 + 8 + 8 + 1;
}

#[derive(AnchorSerialize, AnchorDeserialize, PartialEq, Eq, Clone, Copy)]
pub enum State {
    Idle = 0,
    Req = 1,
}

// Error definitions
#[error_code]
pub enum VaultError {
    #[msg("Unauthorized access")]
    Unauthorized,
    #[msg("Invalid vault state for this operation")]
    InvalidState,
    #[msg("Invalid wait time - must be greater than 0")]
    InvalidWaitTime,
    #[msg("Invalid amount - must be greater than 0")]
    InvalidAmount,
    #[msg("Insufficient funds in vault")]
    InsufficientFunds,
    #[msg("Wait time has not elapsed yet - need to wait more slots")]
    WaitTimeNotElapsed,
    #[msg("Invalid receiver address")]
    InvalidReceiver,
}