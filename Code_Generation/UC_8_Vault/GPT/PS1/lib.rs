use anchor_lang::prelude::*;
use anchor_lang::system_program;

declare_id!("DKF1R2BKYCmhsKfvng7jgtQWL789kFaiPXMWHeuS7oDS");

#[program]
pub mod vault {
    use super::*;

    /// Initialize a vault PDA for the owner and deposit `initial_amount` lamports into it.
    /// - signer: owner
    /// - accounts: owner (payer, signer), recovery (Pubkey ref), vault_info (PDA init), system_program
    pub fn initialize(
        ctx: Context<InitializeCtx>,
        wait_time: u64,
        initial_amount: u64,
    ) -> Result<()> {
        // ensure only owner can call (Anchor enforces signer)
        let vault = &mut ctx.accounts.vault_info;

        // initialize fields
        vault.owner = ctx.accounts.owner.key();
        vault.recovery = ctx.accounts.recovery.key();
        vault.receiver = Pubkey::default();
        vault.wait_time = wait_time;
        vault.request_time = 0u64;
        vault.amount = 0u64;
        vault.state = State::Idle;

        // If initial_amount > 0, transfer from owner -> vault_info by manipulating lamports directly.
        if initial_amount > 0 {
            let cpi_ctx = CpiContext::new(
                ctx.accounts.system_program.to_account_info(),
                system_program::Transfer {
                    from: ctx.accounts.owner.to_account_info(),
                    to: ctx.accounts.vault_info.to_account_info(),
                },
            );
            system_program::transfer(cpi_ctx, initial_amount)?;
        }

        Ok(())
    }

    /// Owner creates a withdrawal request.
    /// - signer: owner
    /// - accounts: owner (signer), receiver (reference), vault_info (PDA)
    pub fn withdraw(ctx: Context<WithdrawCtx>, amount: u64) -> Result<()> {
        // immutable borrow first
        let vault_balance = ctx.accounts.vault_info.to_account_info().lamports();

        // now mutable borrow
        let vault = &mut ctx.accounts.vault_info;

        // checks
        require!(ctx.accounts.owner.key() == vault.owner, VaultError::InvalidOwner);
        require!(vault.state == State::Idle, VaultError::VaultNotIdle);
        require!(amount > 0, VaultError::InvalidAmount);
        require!((vault_balance as u64) >= amount, VaultError::InsufficientVaultFunds);

        // mutate
        vault.receiver = ctx.accounts.receiver.key();
        vault.request_time = Clock::get()?.slot;
    ;
        vault.amount = amount;
        vault.state = State::Req;

        Ok(())
    }

    /// Owner finalizes a pending withdrawal after wait_time has elapsed.
    /// - signer: owner
    /// - accounts: owner (signer), receiver (mutable), vault_info (PDA)
    pub fn finalize(ctx: Context<FinalizeCtx>) -> Result<()> {
        // grab account infos first
        let vault_ai = ctx.accounts.vault_info.to_account_info();
        let receiver_ai = ctx.accounts.receiver.to_account_info();

        // now borrow mutably once
        let vault = &mut ctx.accounts.vault_info;

        // state check
        require!(vault.state == State::Req, VaultError::InvalidState);

        // slot timing check
        let now_slot = Clock::get()?.slot;
        let allowed_slot = vault
            .request_time
            .checked_add(vault.wait_time)
            .ok_or(VaultError::TimeOverflow)?;
        require!(now_slot >= allowed_slot, VaultError::WaitTimeNotElapsed);

        // lamports transfer
        **vault_ai.try_borrow_mut_lamports()? -= vault.amount;
        **receiver_ai.try_borrow_mut_lamports()? += vault.amount;

        // reset vault state
        vault.amount = 0;
        vault.receiver = Pubkey::default();
        vault.state = State::Idle;

        Ok(())
    }

    /// Recovery key cancels a pending withdrawal, no funds moved.
    /// - signer: recovery
    /// - accounts: recovery (signer), owner (reference), vault_info (PDA)
    pub fn cancel(ctx: Context<CancelCtx>) -> Result<()> {
        let vault = &mut ctx.accounts.vault_info;

        // Ensure recovery key matches
        require!(
            ctx.accounts.recovery.key() == vault.recovery,
            VaultError::InvalidRecoveryKey
        );

        // The "owner" reference must match vault.owner (explicitly required)
        require!(
            ctx.accounts.owner.key() == vault.owner,
            VaultError::InvalidOwner
        );

        // Only cancel when in Req state
        require!(
            vault.state == State::Req,
            VaultError::NoPendingRequest
        );

        // Reset request fields without moving funds
        vault.receiver = Pubkey::default();
        vault.request_time = 0u64;
        vault.amount = 0u64;
        vault.state = State::Idle;

        Ok(())
    }
}

/// Helper to get current unix timestamp as u64 with safety checks.
fn unix_timestamp_now() -> Result<u64> {
    let ts_i64 = Clock::get()?.unix_timestamp;
    if ts_i64 < 0 {
        return err!(VaultError::InvalidClockTime);
    }
    Ok(ts_i64 as u64)
}

#[derive(AnchorSerialize, AnchorDeserialize, Clone, PartialEq, Eq)]
pub enum State {
    Idle = 0,
    Req = 1,
}



/// Vault account stored at PDA [owner.key().as_ref()] with program id bump
#[account]
pub struct VaultInfo {
    pub owner: Pubkey,
    /// CHECK: recovery is only used for signature validation
    pub recovery: Pubkey,
    pub receiver: Pubkey,
    pub wait_time: u64,
    pub request_time: u64,
    pub amount: u64,
    pub state: State, // ✅ use State directly
}


// ---------- Accounts Contexts -----------

#[derive(Accounts)]
pub struct InitializeCtx<'info> {
    /// Owner (payer) who signs
    #[account(mut)]
    pub owner: Signer<'info>,

    /// CHECK: This is only used to record the recovery pubkey in the vault state.
    /// Recovery key (a reference, not signer)
    /// This is only stored in vault_info; it does NOT have to be mutable or signer.
    /// It can be any system account; we accept it as AccountInfo (unchecked).
    /// Anchor doesn't require explicit type; the simplest is to accept as AccountInfo.
    /// But keep type as `AccountInfo` to match "reference" requirement.
    pub recovery: UncheckedAccount<'info>,

    /// Vault PDA - created here. Seeds must be [owner.key().as_ref()].
    /// We init the account, payer = owner.
    #[account(
        init,
        payer = owner,
        seeds = [owner.key().as_ref()],
        bump,
        space = 136  // discriminator (8) + fields (calculated + padding)
    )]
    pub vault_info: Account<'info, VaultInfo>,

    /// System program required by `init`
    pub system_program: Program<'info, System>,
}

#[derive(Accounts)]
pub struct WithdrawCtx<'info> {
    /// Owner must sign
    pub owner: Signer<'info>,

    /// CHECK: Receiver account can be any system account. Only lamports balance is mutated.
    /// Receiver reference (not mutable)
    /// We accept as `UncheckedAccount` to allow any pubkey; not used to debit here.
    /// Will be checked against stored receiver on finalize.
    pub receiver: UncheckedAccount<'info>,

    /// Vault PDA (derived with [owner.key().as_ref()])
    /// This account is read/write because we modify its state.
    #[account(
        mut,
        seeds = [owner.key().as_ref()],
        bump
    )]
    pub vault_info: Account<'info, VaultInfo>,
}

#[derive(Accounts)]
pub struct FinalizeCtx<'info> {
    /// Owner signer
    pub owner: Signer<'info>,

    /// CHECK: Receiver account can be any system account. Only lamports balance is mutated.
    /// Receiver must be mutable to accept lamports
    /// Note: we check its Pubkey equals vault_info.receiver in handler.
    #[account(mut)]
    pub receiver: UncheckedAccount<'info>,

    /// Vault PDA (mut because we update its state)
    #[account(
        mut,
        seeds = [owner.key().as_ref()],
        bump
    )]
    pub vault_info: Account<'info, VaultInfo>,
}

#[derive(Accounts)]
pub struct CancelCtx<'info> {
    /// Recovery key signer
    pub recovery: Signer<'info>,

    /// CHECK: Owner reference is only used for pubkey comparison with vault.owner
    /// Owner reference (not signer for this instruction)
    /// but must match vault_info.owner
    pub owner: UncheckedAccount<'info>,

    /// Vault PDA (mut because we will modify its state)
    #[account(
        mut,
        seeds = [owner.key().as_ref()],
        bump
    )]
    pub vault_info: Account<'info, VaultInfo>,
}

// ---------- Errors -----------
#[error_code]
pub enum VaultError {
    #[msg("Provided owner does not match vault owner")]
    InvalidOwner,
    #[msg("Provided recovery key does not match vault recovery")]
    InvalidRecoveryKey,
    #[msg("Vault must be in Idle state for this operation")]
    VaultNotIdle,
    #[msg("Vault has no pending request")]
    NoPendingRequest,
    #[msg("Receiver does not match the pending request receiver")]
    ReceiverMismatch,
    #[msg("Wait time has not yet elapsed")]
    WaitTimeNotElapsed,
    #[msg("Vault has insufficient funds to satisfy this request")]
    InsufficientVaultFunds,
    #[msg("Invalid amount provided")]
    InvalidAmount,
    #[msg("Clock returned invalid (negative) unix timestamp")]
    InvalidClockTime,
    #[msg("Time arithmetic overflow")]
    TimeOverflow,
    #[msg("Invalid state for this operation")]
    InvalidState,
}
