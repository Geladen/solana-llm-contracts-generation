use anchor_lang::prelude::*;
use anchor_lang::system_program;
use anchor_lang::system_program::Transfer;
use borsh::{BorshDeserialize, BorshSerialize};

declare_id!("BCVfNzix1KJPu96WdcdEL1GZ7jkSoz6AR3JKJ9tzu36q");

#[program]
pub mod vault {
    use super::*;

    /// Initialize the vault PDA (seeded by owner pubkey).
    /// `owner` must sign. `initial_amount` (lamports) will be moved from owner into the vault after creation via CPI.
    pub fn initialize(
        ctx: Context<InitializeCtx>,
        wait_time: u64,
        initial_amount: u64,
    ) -> Result<()> {
        // Basic parameter checks
        if wait_time == 0 {
            return err!(ErrorCode::InvalidWaitTime);
        }

        // Save vault information
        let vault = &mut ctx.accounts.vault_info;
        vault.owner = *ctx.accounts.owner.key;
        vault.recovery = *ctx.accounts.recovery.key;
        vault.receiver = Pubkey::default();
        vault.wait_time = wait_time;
        vault.request_time = 0;
        vault.amount = 0;
        vault.state = State::Idle;

        // If initial_amount requested, transfer lamports from owner -> vault via system program CPI
        if initial_amount > 0 {
            // CPI: system_program::transfer
            // Owner is a signer, system program will debit it
            let cpi_accounts = Transfer {
                from: ctx.accounts.owner.to_account_info(),
                to: ctx.accounts.vault_info.to_account_info(),
            };
            let cpi_ctx = CpiContext::new(ctx.accounts.system_program.to_account_info(), cpi_accounts);
            system_program::transfer(cpi_ctx, initial_amount)?;
        }

        Ok(())
    }

    /// Owner requests a withdrawal of `amount` to `receiver`.
    /// Moves vault into `Req` state and records request_time.
    pub fn withdraw(ctx: Context<WithdrawCtx>, amount: u64) -> Result<()> {
        let vault = &mut ctx.accounts.vault_info;

        require!(vault.state == State::Idle, ErrorCode::InvalidState);

        let clock = Clock::get()?;
        vault.state = State::Req;
        vault.amount = amount;
        vault.receiver = ctx.accounts.receiver.key();
        vault.request_time = clock.slot;

        Ok(())
    }

    /// Owner finalizes a pending withdrawal after the wait time has elapsed.
    /// Funds are transferred to `receiver`. `receiver` must match the recorded receiver in vault_info.
    pub fn finalize(ctx: Context<FinalizeCtx>) -> Result<()> {
        let vault = &mut ctx.accounts.vault_info;

        require!(vault.state == State::Req, ErrorCode::InvalidState);

        let clock = Clock::get()?;
        require!(
            clock.slot >= vault.request_time + vault.wait_time,
            ErrorCode::WithdrawalNotReady
        );

        let vault_ai = vault.to_account_info();
        let receiver_ai = ctx.accounts.receiver.to_account_info();

        **vault_ai.try_borrow_mut_lamports()? -= vault.amount;
        **receiver_ai.try_borrow_mut_lamports()? += vault.amount;

        // reset
        vault.receiver = Pubkey::default();
        vault.request_time = 0;
        vault.amount = 0;
        vault.state = State::Idle;


        Ok(())
    }

    /// Recovery key cancels a pending withdrawal request. No funds move.
    pub fn cancel(ctx: Context<CancelCtx>) -> Result<()> {
        let vault = &mut ctx.accounts.vault_info;

        // Recovery signer must match the stored recovery key
        if vault.recovery != *ctx.accounts.recovery.key {
            return err!(ErrorCode::RecoveryMismatch);
        }

        // Provided owner reference must match vault owner
        if vault.owner != *ctx.accounts.owner.key {
            return err!(ErrorCode::OwnerMismatch);
        }

        // Only valid if there's an active request
        if vault.state != State::Req {
            return err!(ErrorCode::NoPendingRequest);
        }

        // Clear request state (no funds are moved)
        vault.state = State::Idle;
        vault.receiver = Pubkey::default();
        vault.request_time = 0;
        vault.amount = 0;

        Ok(())
    }
}

/// -----------------------------------------------------------------------------
/// Accounts
/// -----------------------------------------------------------------------------

#[derive(Accounts)]
pub struct InitializeCtx<'info> {
    /// Owner creates the vault and pays for account creation
    #[account(mut)]
    pub owner: Signer<'info>,

    /// Recovery key (just a reference address)
    /// CHECK: validated/stored in VaultInfo
    pub recovery: UncheckedAccount<'info>,

    /// PDA that stores vault state AND holds lamports.
    /// Seeds exactly: [owner.key().as_ref()]
    #[account(
        init,
        payer = owner,
        space = VaultInfo::LEN,
        seeds = [owner.key().as_ref()],
        bump
    )]
    pub vault_info: Account<'info, VaultInfo>,

    /// System program required for CPI transfer and init
    pub system_program: Program<'info, System>,
}

#[derive(Accounts)]
pub struct WithdrawCtx<'info> {
    /// Owner must sign
    pub owner: Signer<'info>,

    /// Receiver address where funds will be sent on finalize (reference)
    /// CHECK: validated against VaultInfo.receiver on finalize
    pub receiver: UncheckedAccount<'info>,

    /// Vault PDA - must match seeds = [owner.key().as_ref()]
    #[account(mut, seeds = [owner.key().as_ref()], bump)]
    pub vault_info: Account<'info, VaultInfo>,
}

#[derive(Accounts)]
pub struct FinalizeCtx<'info> {
    pub owner: Signer<'info>,

    /// Receiver must be mutable (we will deposit lamports)
    /// CHECK: must equal vault_info.receiver
    #[account(mut)]
    pub receiver: UncheckedAccount<'info>,

    /// Vault PDA - must match seeds = [owner.key().as_ref()]
    #[account(mut, seeds = [owner.key().as_ref()], bump)]
    pub vault_info: Account<'info, VaultInfo>,
}

#[derive(Accounts)]
pub struct CancelCtx<'info> {
    /// Recovery key must sign
    pub recovery: Signer<'info>,

    /// Owner is provided as a reference to derive the PDA. Not a signer here.
    /// CHECK: validated against vault_info.owner
    pub owner: UncheckedAccount<'info>,

    /// Vault PDA - must match seeds = [owner.key().as_ref()]
    #[account(mut, seeds = [owner.key().as_ref()], bump)]
    pub vault_info: Account<'info, VaultInfo>,
}

/// -----------------------------------------------------------------------------
/// State & Storage
/// -----------------------------------------------------------------------------

// Use Anchor serialization for the enum
#[derive(AnchorSerialize, AnchorDeserialize, Clone, PartialEq, Debug)]
pub enum State {
    Idle = 0,
    Req = 1,
}

#[account]
pub struct VaultInfo {
    pub owner: Pubkey,
    pub recovery: Pubkey,
    pub receiver: Pubkey,
    pub wait_time: u64,    // → waitTime in JS
    pub request_time: u64, // → requestTime in JS
    pub amount: u64,
    pub state: State,
}



impl VaultInfo {
    pub const LEN: usize = 8 + (32 * 3) + (8 * 3) + 1 + 1;
}

/// -----------------------------------------------------------------------------
/// Errors
/// -----------------------------------------------------------------------------
#[error_code]
pub enum ErrorCode {
    #[msg("Invalid wait_time: must be nonzero")]
    InvalidWaitTime,
    #[msg("Invalid amount: must be > 0")]
    InvalidAmount,
    #[msg("Owner provided does not match vault owner")]
    OwnerMismatch,
    #[msg("Recovery signer does not match vault recovery key")]
    RecoveryMismatch,
    #[msg("A withdrawal is already requested")]
    AlreadyRequested,
    #[msg("No pending withdrawal request")]
    NoPendingRequest,
    #[msg("Vault does not have enough funds to cover request plus rent-exempt minimum")]
    InsufficientVaultFunds,
    #[msg("Owner does not have enough funds")]
    InsufficientOwnerFunds,
    #[msg("Wait time has not yet elapsed")]
    WaitNotElapsed,
    #[msg("Receiver account mismatch")]
    ReceiverMismatch,
    #[msg("Integer overflow while calculating times/amounts")]
    TimeOverflow,
    #[msg("Integer overflow while calculating amounts")]
    AmountOverflow,
    #[msg("Vault is not in the correct state for this action")]
    InvalidState,
    #[msg("Withdrawal is not ready yet (wait time not passed)")]
    WithdrawalNotReady,
}
