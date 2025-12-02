use anchor_lang::prelude::*;
use anchor_lang::solana_program::program::invoke;
use anchor_lang::solana_program::system_instruction;

declare_id!("HMx1gKCEsFN2L5Nrwct9vF4XSJxFXGGep3PGXWjqrWDY");

#[program]
pub mod vault {
    use super::*;

    pub fn initialize(
        ctx: Context<InitializeCtx>,
        wait_time: u64,
        initial_amount: u64,
    ) -> Result<()> {
        let (_pda, bump) =
            Pubkey::find_program_address(&[ctx.accounts.owner.key().as_ref()], ctx.program_id);

        let vault = &mut ctx.accounts.vault_info;
        vault.owner = ctx.accounts.owner.key();
        vault.recovery = ctx.accounts.recovery.key();
        vault.receiver = Pubkey::default();
        vault.wait_time = wait_time;
        vault.request_time = 0;
        vault.amount = 0;
        vault.state = State::Idle;
        vault.bump = bump;

        if initial_amount > 0 {
            let ix = system_instruction::transfer(
                &ctx.accounts.owner.key(),
                &ctx.accounts.vault_info.key(),
                initial_amount,
            );
            invoke(
                &ix,
                &[
                    ctx.accounts.owner.to_account_info(),
                    ctx.accounts.vault_info.to_account_info(),
                    ctx.accounts.system_program.to_account_info(),
                ],
            )?;
        }

        Ok(())
    }

    pub fn withdraw(ctx: Context<WithdrawCtx>, amount: u64) -> Result<()> {
        let vault_ai = ctx.accounts.vault_info.to_account_info();
        let vault_lamports = vault_ai.lamports();

        let vault = &mut ctx.accounts.vault_info;

        require!(vault.owner == ctx.accounts.owner.key(), ErrorCode::OwnerMismatch);
        require!(vault.state == State::Idle, ErrorCode::InvalidState);
        require!(amount > 0, ErrorCode::InvalidAmount);

        let rent = Rent::get()?;
        let min_balance = rent.minimum_balance(VaultInfo::LEN);
        let available = vault_lamports.checked_sub(min_balance).ok_or(ErrorCode::InsufficientFunds)?;
        require!(amount <= available, ErrorCode::InsufficientFunds);

        vault.receiver = ctx.accounts.receiver.key();
        vault.request_time = Clock::get()?.slot;
        vault.amount = amount;
        vault.state = State::Req;

        Ok(())
    }

    pub fn finalize(ctx: Context<FinalizeCtx>) -> Result<()> {
        let vault_ai = ctx.accounts.vault_info.to_account_info();
        let receiver_ai = ctx.accounts.receiver.to_account_info();
        let vault_key = ctx.accounts.vault_info.key();
        let receiver_key = ctx.accounts.receiver.key();
        let owner_key = ctx.accounts.owner.key();

        let vault = &mut ctx.accounts.vault_info;

        require!(vault.owner == owner_key, ErrorCode::OwnerMismatch);
        require!(vault.state == State::Req, ErrorCode::InvalidState);
        require!(vault.amount > 0, ErrorCode::InvalidAmount);
        require!(vault.receiver == receiver_key, ErrorCode::ReceiverMismatch);

        let current_slot = Clock::get()?.slot;
        let deadline = vault
            .request_time
            .checked_add(vault.wait_time)
            .ok_or(ErrorCode::MathOverflow)?;
        require!(current_slot >= deadline, ErrorCode::WaitTimeNotElapsed);

        let rent = Rent::get()?;
        let min_balance = rent.minimum_balance(VaultInfo::LEN);
        let vault_lamports = vault_ai.lamports();
        let available = vault_lamports.checked_sub(min_balance).ok_or(ErrorCode::InsufficientFunds)?;
        require!(vault.amount <= available, ErrorCode::InsufficientFunds);

        {
            let mut from_lamports = vault_ai.try_borrow_mut_lamports()?;
            let mut to_lamports = receiver_ai.try_borrow_mut_lamports()?;

            **from_lamports = from_lamports
                .checked_sub(vault.amount)
                .ok_or(ErrorCode::InsufficientFunds)?;
            **to_lamports = to_lamports
                .checked_add(vault.amount)
                .ok_or(ErrorCode::MathOverflow)?;
        }

        vault.receiver = Pubkey::default();
        vault.request_time = 0;
        vault.amount = 0;
        vault.state = State::Idle;

        Ok(())
    }

    pub fn cancel(ctx: Context<CancelCtx>) -> Result<()> {
        let vault = &mut ctx.accounts.vault_info;
        require!(vault.recovery == ctx.accounts.recovery.key(), ErrorCode::RecoveryMismatch);
        require!(vault.state == State::Req, ErrorCode::InvalidState);

        vault.receiver = Pubkey::default();
        vault.request_time = 0;
        vault.amount = 0;
        vault.state = State::Idle;

        Ok(())
    }
}

#[derive(Accounts)]
#[instruction(wait_time: u64, initial_amount: u64)]
pub struct InitializeCtx<'info> {
    #[account(mut)]
    pub owner: Signer<'info>,

    /// CHECK: recovery is stored in VaultInfo for off-chain recovery procedures
    pub recovery: UncheckedAccount<'info>,

    /// Vault PDA: seeds = [owner.key().as_ref()]
    #[account(
        init,
        payer = owner,
        space = VaultInfo::LEN,
        seeds = [owner.key().as_ref()],
        bump
    )]
    pub vault_info: Account<'info, VaultInfo>,

    pub system_program: Program<'info, System>,
}

#[derive(Accounts)]
pub struct WithdrawCtx<'info> {
    pub owner: Signer<'info>,

    /// CHECK: receiver is a destination pubkey for later finalize; validated on finalize
    pub receiver: UncheckedAccount<'info>,

    /// Vault PDA: seeds = [owner.key().as_ref()]
    #[account(
        mut,
        seeds = [owner.key().as_ref()],
        bump = vault_info.bump,
        has_one = owner
    )]
    pub vault_info: Account<'info, VaultInfo>,
}

#[derive(Accounts)]
pub struct FinalizeCtx<'info> {
    pub owner: Signer<'info>,

    /// CHECK: receiver will receive lamports; validated against stored receiver in VaultInfo
    #[account(mut)]
    pub receiver: UncheckedAccount<'info>,

    /// Vault PDA: seeds = [owner.key().as_ref()]
    #[account(
        mut,
        seeds = [owner.key().as_ref()],
        bump = vault_info.bump,
        has_one = owner
    )]
    pub vault_info: Account<'info, VaultInfo>,

    pub system_program: Program<'info, System>,
}

#[derive(Accounts)]
pub struct CancelCtx<'info> {
    pub recovery: Signer<'info>,

    /// CHECK: owner is used for PDA derivation and validated by has_one on vault_info
    pub owner: UncheckedAccount<'info>,

    /// Vault PDA: seeds = [owner.key().as_ref()]
    #[account(
        mut,
        seeds = [owner.key().as_ref()],
        bump = vault_info.bump,
        has_one = owner
    )]
    pub vault_info: Account<'info, VaultInfo>,
}

#[derive(AnchorSerialize, AnchorDeserialize, Clone, Copy, PartialEq, Eq, Debug)]
pub enum State {
    Idle = 0,
    Req = 1,
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
    pub bump: u8,
}

impl VaultInfo {
    pub const LEN: usize = 8
        + 32
        + 32
        + 32
        + 8
        + 8
        + 8
        + 1
        + 1;
}

#[error_code]
pub enum ErrorCode {
    #[msg("Owner mismatch")]
    OwnerMismatch,
    #[msg("Recovery key mismatch")]
    RecoveryMismatch,
    #[msg("Invalid program state for this operation")]
    InvalidState,
    #[msg("Insufficient funds in vault")]
    InsufficientFunds,
    #[msg("Amount must be > 0")]
    InvalidAmount,
    #[msg("Wait time has not elapsed yet")]
    WaitTimeNotElapsed,
    #[msg("Receiver does not match request")]
    ReceiverMismatch,
    #[msg("Mathematical overflow")]
    MathOverflow,
    #[msg("Missing PDA bump")]
    MissingBump,
    #[msg("Time conversion failed")]
    TimeConversion,
    #[msg("Stored bump does not match derived bump")]
    BumpMismatch,
}
