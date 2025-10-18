use anchor_lang::prelude::*;
use anchor_lang::solana_program::{clock::Clock, pubkey::Pubkey, rent::Rent};

declare_id!("5Wcyc2y3X8Ziu4hcfJFbM1jmGPZyML5dEDqC2KRuQCxB");

#[program]
pub mod vault {
    use super::*;

    pub fn initialize(
        ctx: Context<InitializeCtx>,
        wait_time: u64,
        initial_amount: u64,
    ) -> Result<()> {
        let vault = &mut ctx.accounts.vault_info;

        vault.owner = *ctx.accounts.owner.key;
        vault.recovery = *ctx.accounts.recovery.key;
        vault.receiver = Pubkey::default();
        vault.wait_time = wait_time;
        vault.request_time = 0;
        vault.amount = 0;
        vault.state = State::Idle;

        if initial_amount > 0 {
            let ix = anchor_lang::solana_program::system_instruction::transfer(
                ctx.accounts.owner.key,
                ctx.accounts.vault_info.to_account_info().key,
                initial_amount,
            );
            anchor_lang::solana_program::program::invoke(
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
        let vault = &mut ctx.accounts.vault_info;

        let (pda, _bump) =
            Pubkey::find_program_address(&[ctx.accounts.owner.key.as_ref()], ctx.program_id);
        require_keys_eq!(pda, vault_ai.key(), VaultError::InvalidVaultPDA);

        require_keys_eq!(vault.owner, *ctx.accounts.owner.key, VaultError::OwnerMismatch);
        require!(vault.state == State::Idle, VaultError::AlreadyRequested);
        require!(amount > 0, VaultError::AmountZero);

        let rent = Rent::get()?;
        let rent_exempt = rent.minimum_balance(VaultInfo::LEN);
        let available = vault_ai.lamports().saturating_sub(rent_exempt);
        require!(amount <= available, VaultError::InsufficientFunds);

        let clock = Clock::get()?;
        vault.receiver = *ctx.accounts.receiver.key;
        vault.request_time = clock.slot;
        vault.amount = amount;
        vault.state = State::Req;

        Ok(())
    }

    pub fn finalize(ctx: Context<FinalizeCtx>) -> Result<()> {
        let vault_ai = ctx.accounts.vault_info.to_account_info();
        let vault = &mut ctx.accounts.vault_info;

        let (pda, _bump) =
            Pubkey::find_program_address(&[ctx.accounts.owner.key.as_ref()], ctx.program_id);
        require_keys_eq!(pda, vault_ai.key(), VaultError::InvalidVaultPDA);

        require_keys_eq!(vault.owner, *ctx.accounts.owner.key, VaultError::OwnerMismatch);
        require!(vault.state == State::Req, VaultError::NoPendingRequest);
        require_keys_eq!(vault.receiver, ctx.accounts.receiver.key(), VaultError::ReceiverMismatch);

        let clock = Clock::get()?;
        require!(
            clock.slot >= vault.request_time.saturating_add(vault.wait_time),
            VaultError::TooEarlyToFinalize
        );

        let amount = vault.amount;
        require!(amount > 0, VaultError::AmountZero);

        let rent = Rent::get()?;
        let rent_exempt = rent.minimum_balance(VaultInfo::LEN);
        let available = vault_ai.lamports().saturating_sub(rent_exempt);
        require!(amount <= available, VaultError::InsufficientFunds);

        {
            let mut vault_lamports_ref = vault_ai.try_borrow_mut_lamports()?;
            **vault_lamports_ref = vault_lamports_ref
                .checked_sub(amount)
                .ok_or(VaultError::InsufficientFunds)?;
            drop(vault_lamports_ref);

            let receiver_ai = ctx.accounts.receiver.to_account_info(); // keep it alive
            let mut receiver_lamports_ref = receiver_ai.try_borrow_mut_lamports()?;
            **receiver_lamports_ref = receiver_lamports_ref
                .checked_add(amount)
                .ok_or(VaultError::ArithmeticOverflow)?;
        }

        vault.state = State::Idle;
        vault.amount = 0;
        vault.request_time = 0;
        vault.receiver = Pubkey::default();

        Ok(())
    }

    pub fn cancel(ctx: Context<CancelCtx>) -> Result<()> {
        let vault_ai = ctx.accounts.vault_info.to_account_info();
        let vault = &mut ctx.accounts.vault_info;

        let (pda, _bump) =
            Pubkey::find_program_address(&[ctx.accounts.owner.key.as_ref()], ctx.program_id);
        require_keys_eq!(pda, vault_ai.key(), VaultError::InvalidVaultPDA);

        require_keys_eq!(vault.recovery, *ctx.accounts.recovery.key, VaultError::NotRecoveryKey);
        require_keys_eq!(vault.owner, *ctx.accounts.owner.key, VaultError::OwnerMismatch);
        require!(vault.state == State::Req, VaultError::NoPendingRequest);

        vault.state = State::Idle;
        vault.amount = 0;
        vault.request_time = 0;
        vault.receiver = Pubkey::default();

        Ok(())
    }
}

#[derive(Accounts)]
pub struct InitializeCtx<'info> {
    #[account(mut)]
    pub owner: Signer<'info>,
    /// CHECK: stored only
    pub recovery: UncheckedAccount<'info>,
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
    /// CHECK: only stored
    pub receiver: UncheckedAccount<'info>,
    #[account(mut)]
    pub vault_info: Account<'info, VaultInfo>,
}

#[derive(Accounts)]
pub struct FinalizeCtx<'info> {
    pub owner: Signer<'info>,
    /// CHECK: lamports only
    #[account(mut)]
    pub receiver: UncheckedAccount<'info>,
    #[account(mut)]
    pub vault_info: Account<'info, VaultInfo>,
}

#[derive(Accounts)]
pub struct CancelCtx<'info> {
    pub recovery: Signer<'info>,
    /// CHECK: only used for PDA seeds
    pub owner: UncheckedAccount<'info>,
    #[account(mut)]
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

impl VaultInfo {
    pub const LEN: usize = 8 + (32 * 3) + (8 * 3) + 1 + 7;
}

#[derive(AnchorSerialize, AnchorDeserialize, Clone, PartialEq, Eq)]
pub enum State {
    Idle = 0,
    Req = 1,
}
impl Default for State {
    fn default() -> Self {
        State::Idle
    }
}

#[error_code]
pub enum VaultError {
    #[msg("Vault PDA derived from owner does not match provided account.")]
    InvalidVaultPDA,
    #[msg("Owner pubkey mismatch.")]
    OwnerMismatch,
    #[msg("Not the configured recovery key.")]
    NotRecoveryKey,
    #[msg("There is already a pending withdrawal request.")]
    AlreadyRequested,
    #[msg("No pending withdrawal request.")]
    NoPendingRequest,
    #[msg("Too early to finalize the withdrawal; wait time not elapsed.")]
    TooEarlyToFinalize,
    #[msg("Insufficient funds in the vault to cover the requested amount.")]
    InsufficientFunds,
    #[msg("Requested amount must be > 0.")]
    AmountZero,
    #[msg("Receiver account does not match the withdrawal request.")]
    ReceiverMismatch,
    #[msg("Arithmetic overflow while moving lamports.")]
    ArithmeticOverflow,
}
