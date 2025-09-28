use anchor_lang::prelude::*;

declare_id!("94rj2WXoKrPeimQkywxg1GDsVMHhun9b2vDYvpx9C1xW");

#[program]
pub mod vault {
    use super::*;

    pub fn initialize(
        ctx: Context<InitializeCtx>,
        wait_time: u64,
        initial_amount: u64,
    ) -> Result<()> {
        let vault_info = &mut ctx.accounts.vault_info;
        
        vault_info.owner = ctx.accounts.owner.key();
        vault_info.recovery = ctx.accounts.recovery.key();
        vault_info.receiver = Pubkey::default();
        vault_info.wait_time = wait_time;
        vault_info.request_time = 0;
        vault_info.amount = 0;
        vault_info.state = VaultState::Idle;

        // Transfer initial funds to vault PDA
        let cpi_context = CpiContext::new(
            ctx.accounts.system_program.to_account_info(),
            anchor_lang::system_program::Transfer {
                from: ctx.accounts.owner.to_account_info(),
                to: ctx.accounts.vault_info.to_account_info(),
            },
        );
        anchor_lang::system_program::transfer(cpi_context, initial_amount)?;

        Ok(())
    }

    pub fn withdraw(ctx: Context<WithdrawCtx>, amount: u64) -> Result<()> {
        let vault_balance = ctx.accounts.vault_info.to_account_info().lamports();
        let vault_info = &mut ctx.accounts.vault_info;
        
        require!(vault_info.state == VaultState::Idle, ErrorCode::InvalidState);
        require!(amount > 0, ErrorCode::InvalidAmount);
        require!(vault_balance >= amount, ErrorCode::InsufficientFunds);

        vault_info.state = VaultState::Req;
        vault_info.amount = amount;
        vault_info.request_time = Clock::get()?.slot;
        vault_info.receiver = ctx.accounts.receiver.key();

        Ok(())
    }

    pub fn finalize(ctx: Context<FinalizeCtx>) -> Result<()> {
        let vault_info_ref = &ctx.accounts.vault_info;
        
        require!(vault_info_ref.state == VaultState::Req, ErrorCode::InvalidState);
        
        let current_slot = Clock::get()?.slot;
        let elapsed_slots = current_slot.saturating_sub(vault_info_ref.request_time);
        require!(elapsed_slots >= vault_info_ref.wait_time, ErrorCode::WaitTimeNotElapsed);
        require!(vault_info_ref.receiver == ctx.accounts.receiver.key(), ErrorCode::InvalidReceiver);

        let amount = vault_info_ref.amount;

        // Transfer funds by manually adjusting lamports
        let vault_account_info = &ctx.accounts.vault_info.to_account_info();
        let receiver_account_info = &ctx.accounts.receiver;
        
        let vault_balance = vault_account_info.lamports();
        require!(vault_balance >= amount, ErrorCode::InsufficientFunds);

        **vault_account_info.try_borrow_mut_lamports()? = vault_balance
            .checked_sub(amount)
            .ok_or(ErrorCode::TransferFailed)?;
            
        **receiver_account_info.try_borrow_mut_lamports()? = receiver_account_info
            .lamports()
            .checked_add(amount)
            .ok_or(ErrorCode::TransferFailed)?;

        let vault_info = &mut ctx.accounts.vault_info;
        vault_info.state = VaultState::Idle;
        vault_info.amount = 0;
        vault_info.request_time = 0;
        vault_info.receiver = Pubkey::default();

        Ok(())
    }

    pub fn cancel(ctx: Context<CancelCtx>) -> Result<()> {
        let vault_info = &mut ctx.accounts.vault_info;
        
        require!(vault_info.state == VaultState::Req, ErrorCode::InvalidState);

        vault_info.state = VaultState::Idle;
        vault_info.amount = 0;
        vault_info.request_time = 0;
        vault_info.receiver = Pubkey::default();

        Ok(())
    }
}

#[derive(Accounts)]
pub struct InitializeCtx<'info> {
    #[account(mut)]
    pub owner: Signer<'info>,
    
    /// CHECK: This is the recovery public key
    pub recovery: AccountInfo<'info>,
    
    #[account(
        init,
        payer = owner,
        space = 8 + VaultInfo::INITIAL_SPACE,
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
    
    /// CHECK: Receiver account
    pub receiver: AccountInfo<'info>,
    
    #[account(
        mut,
        seeds = [owner.key().as_ref()],
        bump,
        has_one = owner @ ErrorCode::InvalidOwner
    )]
    pub vault_info: Account<'info, VaultInfo>,
}

#[derive(Accounts)]
pub struct FinalizeCtx<'info> {
    #[account(mut)]
    pub owner: Signer<'info>,
    
    #[account(mut)]
    /// CHECK: Receiver account
    pub receiver: AccountInfo<'info>,
    
    #[account(
        mut,
        seeds = [owner.key().as_ref()],
        bump,
        has_one = owner @ ErrorCode::InvalidOwner
    )]
    pub vault_info: Account<'info, VaultInfo>,
}

#[derive(Accounts)]
pub struct CancelCtx<'info> {
    pub recovery: Signer<'info>,
    
    /// CHECK: Owner account
    pub owner: AccountInfo<'info>,
    
    #[account(
        mut,
        seeds = [owner.key().as_ref()],
        bump,
        has_one = recovery @ ErrorCode::InvalidRecovery
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
    pub state: VaultState,
}

impl VaultInfo {
    pub const INITIAL_SPACE: usize = 32 + 32 + 32 + 8 + 8 + 8 + 1;
}

// Proper enum definition that should serialize to { idle: {} } format
#[derive(AnchorSerialize, AnchorDeserialize, Clone, PartialEq, Debug)]
pub enum VaultState {
    Idle,
    Req,
}

#[error_code]
pub enum ErrorCode {
    #[msg("Invalid account owner")]
    InvalidOwner,
    #[msg("Invalid recovery key")]
    InvalidRecovery,
    #[msg("Invalid receiver account")]
    InvalidReceiver,
    #[msg("Invalid vault state for this operation")]
    InvalidState,
    #[msg("Wait time has not elapsed")]
    WaitTimeNotElapsed,
    #[msg("Invalid amount specified")]
    InvalidAmount,
    #[msg("Insufficient funds in vault")]
    InsufficientFunds,
    #[msg("Fund transfer failed")]
    TransferFailed,
}