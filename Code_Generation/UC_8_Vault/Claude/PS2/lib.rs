use anchor_lang::prelude::*;

declare_id!("A2ejAosLbiSsZ1gLVwLdarzhkvnMS7qZ3Mfyjud6cePt");

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
        
        // Validate wait_time (must be at least 1 second)
        require!(wait_time > 0, VaultError::InvalidWaitTime);
        
        // Initialize vault state
        vault_info.owner = ctx.accounts.owner.key();
        vault_info.recovery = ctx.accounts.recovery.key();
        vault_info.receiver = Pubkey::default(); // Will be set during withdraw
        vault_info.wait_time = wait_time;
        vault_info.request_time = 0;
        vault_info.amount = 0;
        vault_info.state = State::Idle;

        // Transfer initial amount to vault if specified
        if initial_amount > 0 {
            let transfer_ix = anchor_lang::solana_program::system_instruction::transfer(
                &ctx.accounts.owner.key(),
                &vault_info.key(),
                initial_amount,
            );

            anchor_lang::solana_program::program::invoke(
                &transfer_ix,
                &[
                    ctx.accounts.owner.to_account_info(),
                    vault_info.to_account_info(),
                    ctx.accounts.system_program.to_account_info(),
                ],
            )?;
        }

        msg!("Vault initialized with wait_time: {} seconds", wait_time);
        Ok(())
    }

    /// Initiate a withdrawal request
    /// Only the owner can call this function
    pub fn withdraw(ctx: Context<WithdrawCtx>, amount: u64) -> Result<()> {
        let vault_info = &mut ctx.accounts.vault_info;
        let clock = Clock::get()?;

        // Verify vault is in idle state
        require!(vault_info.state == State::Idle, VaultError::VaultNotIdle);
        
        // Validate amount
        require!(amount > 0, VaultError::InvalidAmount);
        
        // Check if vault has sufficient balance
        let vault_balance = vault_info.to_account_info().lamports();
        let rent_exempt_minimum = Rent::get()?.minimum_balance(vault_info.to_account_info().data_len());
        let available_balance = vault_balance.saturating_sub(rent_exempt_minimum);
        
        require!(amount <= available_balance, VaultError::InsufficientBalance);

        // Set withdrawal request using slot number for timing
        vault_info.receiver = ctx.accounts.receiver.key();
        vault_info.amount = amount;
        vault_info.request_time = clock.slot;
        vault_info.state = State::Req;

        msg!("Withdrawal request initiated for {} lamports to {} at slot {}", amount, ctx.accounts.receiver.key(), clock.slot);
        Ok(())
    }

    /// Finalize the withdrawal after wait time has elapsed
    /// Only the owner can call this function
    pub fn finalize(ctx: Context<FinalizeCtx>) -> Result<()> {
        let vault_info = &mut ctx.accounts.vault_info;
        let clock = Clock::get()?;

        // Verify vault is in request state
        require!(vault_info.state == State::Req, VaultError::NoWithdrawalRequest);
        
        // Verify receiver matches the one from withdrawal request
        require!(
            ctx.accounts.receiver.key() == vault_info.receiver,
            VaultError::ReceiverMismatch
        );

        // Check if wait time has elapsed
        let current_time = clock.unix_timestamp as u64;
        let elapsed_time = current_time.saturating_sub(vault_info.request_time);
        
        require!(elapsed_time >= vault_info.wait_time, VaultError::WaitTimeNotElapsed);

        // Perform the transfer
        let amount = vault_info.amount;
        let owner_key = vault_info.owner;
        
        // Generate signer seeds for PDA
        let owner_key = vault_info.owner;
        let (_, bump) = Pubkey::find_program_address(&[owner_key.as_ref()], ctx.program_id);
        let signer_seeds: &[&[&[u8]]] = &[&[owner_key.as_ref(), &[bump]]];

        // Transfer lamports from vault to receiver
        **vault_info.to_account_info().try_borrow_mut_lamports()? -= amount;
        **ctx.accounts.receiver.to_account_info().try_borrow_mut_lamports()? += amount;

        // Reset vault to idle state
        vault_info.receiver = Pubkey::default();
        vault_info.amount = 0;
        vault_info.request_time = 0;
        vault_info.state = State::Idle;

        msg!("Withdrawal finalized: {} lamports transferred to {}", amount, ctx.accounts.receiver.key());
        Ok(())
    }

    /// Cancel a pending withdrawal request
    /// Only the recovery key can call this function
    pub fn cancel(ctx: Context<CancelCtx>) -> Result<()> {
        let vault_info = &mut ctx.accounts.vault_info;

        // Verify vault is in request state
        require!(vault_info.state == State::Req, VaultError::NoWithdrawalRequest);

        // Reset vault to idle state without transferring funds
        vault_info.receiver = Pubkey::default();
        vault_info.amount = 0;
        vault_info.request_time = 0;
        vault_info.state = State::Idle;

        msg!("Withdrawal request cancelled by recovery key");
        Ok(())
    }
}

// Context structs for each instruction

#[derive(Accounts)]
pub struct InitializeCtx<'info> {
    #[account(mut)]
    pub owner: Signer<'info>,
    
    /// CHECK: Recovery key is validated but not required to sign for initialization
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
    #[account(
        mut,
        constraint = owner.key() == vault_info.owner @ VaultError::Unauthorized
    )]
    pub owner: Signer<'info>,
    
    /// CHECK: Receiver account is validated during withdrawal setup
    pub receiver: AccountInfo<'info>,
    
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
        constraint = owner.key() == vault_info.owner @ VaultError::Unauthorized
    )]
    pub owner: Signer<'info>,
    
    #[account(mut)]
    /// CHECK: Receiver is validated against vault_info.receiver
    pub receiver: AccountInfo<'info>,
    
    #[account(
        mut,
        seeds = [owner.key().as_ref()],
        bump
    )]
    pub vault_info: Account<'info, VaultInfo>,
}

#[derive(Accounts)]
pub struct CancelCtx<'info> {
    #[account(
        constraint = recovery.key() == vault_info.recovery @ VaultError::Unauthorized
    )]
    pub recovery: Signer<'info>,
    
    /// CHECK: Owner reference for vault validation
    pub owner: AccountInfo<'info>,
    
    #[account(
        mut,
        seeds = [owner.key().as_ref()],
        bump
    )]
    pub vault_info: Account<'info, VaultInfo>,
}

// Account structures

#[account]
#[derive(InitSpace)]
pub struct VaultInfo {
    pub owner: Pubkey,
    pub recovery: Pubkey,
    pub receiver: Pubkey,
    pub wait_time: u64,
    pub request_time: u64,
    pub amount: u64,
    pub state: State,
}

#[derive(AnchorSerialize, AnchorDeserialize, Clone, PartialEq, Eq, InitSpace)]
pub enum State {
    Idle = 0,
    Req = 1,
}

// Error definitions

#[error_code]
pub enum VaultError {
    #[msg("Unauthorized access")]
    Unauthorized,
    #[msg("Invalid wait time - must be greater than 0")]
    InvalidWaitTime,
    #[msg("Invalid amount - must be greater than 0")]
    InvalidAmount,
    #[msg("Insufficient balance in vault")]
    InsufficientBalance,
    #[msg("Vault is not in idle state")]
    VaultNotIdle,
    #[msg("No withdrawal request pending")]
    NoWithdrawalRequest,
    #[msg("Wait time has not elapsed")]
    WaitTimeNotElapsed,
    #[msg("Receiver account mismatch")]
    ReceiverMismatch,
}