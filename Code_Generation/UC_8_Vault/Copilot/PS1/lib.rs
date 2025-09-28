use anchor_lang::prelude::*;
use anchor_lang::solana_program::{program::invoke_signed, system_instruction};
use anchor_lang::solana_program::rent::Rent;
use anchor_lang::solana_program::clock::Clock;

declare_id!("CUWvn3jvxPMtM4MQth962wGGEsJcPYEkHLoBVWEgt1YV");

#[program]
pub mod vault {
    use super::*;

    pub fn initialize(ctx: Context<InitializeCtx>, wait_time: u64, initial_amount: u64) -> Result<()> {
        require!(ctx.accounts.owner.is_signer, VaultError::OwnerMustSign);

        // Validate PDA and capture bump
        let (expected_pda, bump) = Pubkey::find_program_address(&[ctx.accounts.owner.key.as_ref()], ctx.program_id);
        require!(expected_pda == ctx.accounts.vault_info.to_account_info().key(), VaultError::InvalidPDA);

        // Clone AccountInfo needed before mutable borrow
        let vault_ai = ctx.accounts.vault_info.to_account_info().clone();
        let data_len = vault_ai.data_len();
        let rent = Rent::get()?;
        let rent_exempt = rent.minimum_balance(data_len);

        // Optional initial transfer owner -> vault PDA
        if initial_amount > 0 {
            let owner_lamports = ctx.accounts.owner.to_account_info().lamports();
            require!(owner_lamports >= initial_amount, VaultError::InsufficientOwnerFunds);

            let ix = system_instruction::transfer(
                &ctx.accounts.owner.key(),
                &ctx.accounts.vault_info.key(),
                initial_amount,
            );

            invoke_signed(
                &ix,
                &[
                    ctx.accounts.owner.to_account_info(),
                    vault_ai.clone(),
                    ctx.accounts.system_program.to_account_info(),
                ],
                &[], // owner signed
            )?;
        }

        // Initialize VaultInfo
        let vault = &mut ctx.accounts.vault_info;
        vault.owner = ctx.accounts.owner.key();
        vault.recovery = ctx.accounts.recovery.key();
        vault.receiver = Pubkey::default();
        vault.wait_time = wait_time; // interpreted as slots
        vault.request_time = 0;
        vault.amount = 0;
        vault.state = State::Idle;
        vault.bump = bump;

        // Ensure rent-exemptness
        let vault_lamports = ctx.accounts.vault_info.to_account_info().lamports();
        require!(vault_lamports >= rent_exempt, VaultError::VaultMustRemainRentExempt);

        Ok(())
    }

    pub fn withdraw(ctx: Context<WithdrawCtx>, amount: u64) -> Result<()> {
        require!(ctx.accounts.owner.is_signer, VaultError::OwnerMustSign);

        // Validate PDA
        let (expected_pda, _bump) = Pubkey::find_program_address(&[ctx.accounts.owner.key.as_ref()], ctx.program_id);
        require!(expected_pda == ctx.accounts.vault_info.to_account_info().key(), VaultError::InvalidPDA);

        // Read vault account meta before mutable borrow
        let vault_ai = ctx.accounts.vault_info.to_account_info().clone();
        let data_len = vault_ai.data_len();
        let rent = Rent::get()?;
        let rent_exempt = rent.minimum_balance(data_len);
        let vault_lamports = vault_ai.lamports();

        // Mutable borrow to update state
        let vault = &mut ctx.accounts.vault_info;

        // Verifications
        require!(vault.owner == ctx.accounts.owner.key(), VaultError::InvalidOwnerAccount);
        require!(vault.state == State::Idle, VaultError::InvalidState);
        require!(amount > 0, VaultError::InvalidAmount);

        let available = vault_lamports.checked_sub(rent_exempt).unwrap_or(0);
        require!(available >= amount, VaultError::InsufficientVaultFunds);

        // Record request using slot-based timing
        let now_slot = Clock::get()?.slot as u64;
        vault.receiver = ctx.accounts.receiver.key();
        vault.request_time = now_slot;
        vault.amount = amount;
        vault.state = State::Req;

        Ok(())
    }

pub fn finalize(ctx: Context<FinalizeCtx>) -> Result<()> {
    require!(ctx.accounts.owner.is_signer, VaultError::OwnerMustSign);

    // Validate PDA and capture bump
    let (expected_pda, bump) = Pubkey::find_program_address(&[ctx.accounts.owner.key.as_ref()], ctx.program_id);
    require!(expected_pda == ctx.accounts.vault_info.to_account_info().key(), VaultError::InvalidPDA);

    // Clone AccountInfo objects we will use for lamports operations
    let vault_ai = ctx.accounts.vault_info.to_account_info().clone();
    let receiver_ai = ctx.accounts.receiver.to_account_info().clone();

    // Read rent & lamports from cloned AccountInfo (immutable operations)
    let data_len = vault_ai.data_len();
    let rent = Rent::get()?;
    let rent_exempt = rent.minimum_balance(data_len);
    let vault_lamports = vault_ai.lamports();

    // Immutable borrow to read on-chain VaultInfo fields needed for checks
    let vault_read = &ctx.accounts.vault_info;
    require!(vault_read.owner == ctx.accounts.owner.key(), VaultError::InvalidOwnerAccount);
    require!(vault_read.state == State::Req, VaultError::InvalidState);
    require!(vault_read.receiver == *receiver_ai.key, VaultError::ReceiverMismatch);

    // Slot-based wait check
    let now_slot = Clock::get()?.slot as u64;
    let ready_slot = vault_read.request_time.checked_add(vault_read.wait_time).ok_or(VaultError::TimeOverflow)?;
    require!(now_slot >= ready_slot, VaultError::WaitTimeNotMet);

    // Ensure available funds
    let available = vault_lamports.checked_sub(rent_exempt).unwrap_or(0);
    require!(available >= vault_read.amount, VaultError::InsufficientVaultFunds);

    // Read amount into a plain u64
    let amount = vault_read.amount;

    // Mutate lamports using the cloned AccountInfo bindings (no CPI)
    // Do checked arithmetic on plain u64s, then assign into RefMut
    let mut from_account = vault_ai; // owned clone
    let mut to_account = receiver_ai; // owned clone

    let mut from_lamports_ref = from_account.try_borrow_mut_lamports()?;
    let mut to_lamports_ref = to_account.try_borrow_mut_lamports()?;

    let from_before = **from_lamports_ref;
    let to_before = **to_lamports_ref;

    let from_after = from_before.checked_sub(amount).ok_or(VaultError::InsufficientVaultFunds)?;
    let to_after = to_before.checked_add(amount).ok_or(VaultError::Overflow)?;

    **from_lamports_ref = from_after;
    **to_lamports_ref = to_after;

    // Now that lamports have been moved, take a mutable borrow to reset on-chain state
    let vault = &mut ctx.accounts.vault_info;
    vault.receiver = Pubkey::default();
    vault.request_time = 0;
    vault.amount = 0;
    vault.state = State::Idle;

    Ok(())
}


    pub fn cancel(ctx: Context<CancelCtx>) -> Result<()> {
        require!(ctx.accounts.recovery.is_signer, VaultError::RecoveryMustSign);

        // Validate PDA
        let (expected_pda, _bump) = Pubkey::find_program_address(&[ctx.accounts.owner.key.as_ref()], ctx.program_id);
        require!(expected_pda == ctx.accounts.vault_info.to_account_info().key(), VaultError::InvalidPDA);

        let vault = &mut ctx.accounts.vault_info;

        require!(vault.recovery == ctx.accounts.recovery.key(), VaultError::InvalidRecovery);
        require!(vault.owner == ctx.accounts.owner.key(), VaultError::InvalidOwnerAccount);
        require!(vault.state == State::Req, VaultError::InvalidState);

        // Cancel request
        vault.receiver = Pubkey::default();
        vault.request_time = 0;
        vault.amount = 0;
        vault.state = State::Idle;

        Ok(())
    }
}

// ---------- Accounts ----------

#[derive(Accounts)]
#[instruction(wait_time: u64, initial_amount: u64)]
pub struct InitializeCtx<'info> {
    /// CHECK: owner must sign; we only read owner pubkey and use as PDA seed
    #[account(mut, signer)]
    pub owner: AccountInfo<'info>,

    /// CHECK: recovery is a public key reference stored in VaultInfo for cancellation
    pub recovery: AccountInfo<'info>,

    /// Vault PDA holds state and lamports
    /// Seeds: [owner.key().as_ref()]
    #[account(
        init,
        payer = owner,
        space = VaultInfo::SPACE,
        seeds = [owner.key().as_ref()],
        bump
    )]
    pub vault_info: Account<'info, VaultInfo>,

    pub system_program: Program<'info, System>,
}

#[derive(Accounts)]
pub struct WithdrawCtx<'info> {
    /// CHECK: owner must sign; using AccountInfo so we can use key as PDA seed
    #[account(signer)]
    pub owner: AccountInfo<'info>,

    /// CHECK: receiver is simply stored and later used as transfer destination
    pub receiver: AccountInfo<'info>,

    #[account(
        mut,
        seeds = [owner.key().as_ref()],
        bump = vault_info.bump,
        has_one = owner @ VaultError::InvalidOwnerAccount
    )]
    pub vault_info: Account<'info, VaultInfo>,
}

#[derive(Accounts)]
pub struct FinalizeCtx<'info> {
    /// CHECK: owner must sign; only used for PDA derivation and owner validation
    #[account(signer)]
    pub owner: AccountInfo<'info>,

    /// CHECK: receiver must be mutable to accept lamports
    #[account(mut)]
    pub receiver: AccountInfo<'info>,

    #[account(
        mut,
        seeds = [owner.key().as_ref()],
        bump = vault_info.bump,
        has_one = owner @ VaultError::InvalidOwnerAccount
    )]
    pub vault_info: Account<'info, VaultInfo>,

    pub system_program: Program<'info, System>,
}

#[derive(Accounts)]
pub struct CancelCtx<'info> {
    /// CHECK: recovery must sign; we verify its pubkey matches vault.recovery
    #[account(signer)]
    pub recovery: AccountInfo<'info>,

    /// CHECK: owner is provided as a reference and must match vault.owner
    pub owner: AccountInfo<'info>,

    #[account(
        mut,
        seeds = [owner.key().as_ref()],
        bump = vault_info.bump,
        has_one = recovery @ VaultError::InvalidRecovery,
        has_one = owner @ VaultError::InvalidOwnerAccount
    )]
    pub vault_info: Account<'info, VaultInfo>,
}

// ---------- State & Errors ----------

#[account]
pub struct VaultInfo {
    pub owner: Pubkey,
    pub recovery: Pubkey,
    pub receiver: Pubkey,
    /// number of slots to wait before finalize allowed
    pub wait_time: u64,        // JS -> waitTime
    /// slot when withdraw was requested
    pub request_time: u64,     // JS -> requestTime
    pub amount: u64,
    pub state: State,          // Anchor enum, JS -> { idle: {} } or { req: {} }
    pub bump: u8,
    pub _padding: [u8; 7],
}

impl VaultInfo {
    // sizes: discriminator 8 + 32+32+32 + 8+8+8 + 1 + 1 +7
    pub const SPACE: usize = 8 + 32 + 32 + 32 + 8 + 8 + 8 + 1 + 1 + 7;
}

#[derive(AnchorSerialize, AnchorDeserialize, Clone, Copy, PartialEq, Eq)]
#[repr(u8)]
pub enum State {
    Idle = 0,
    Req = 1,
}

#[error_code]
pub enum VaultError {
    #[msg("Owner must sign this transaction")]
    OwnerMustSign,
    #[msg("Recovery key must sign this transaction")]
    RecoveryMustSign,
    #[msg("Invalid owner account")]
    InvalidOwnerAccount,
    #[msg("Invalid recovery account")]
    InvalidRecovery,
    #[msg("Invalid state for this operation")]
    InvalidState,
    #[msg("Invalid amount")]
    InvalidAmount,
    #[msg("Vault does not have sufficient funds")]
    InsufficientVaultFunds,
    #[msg("Owner does not have sufficient funds")]
    InsufficientOwnerFunds,
    #[msg("Vault must remain rent-exempt")]
    VaultMustRemainRentExempt,
    #[msg("Wait time has not been met")]
    WaitTimeNotMet,
    #[msg("Receiver mismatch")]
    ReceiverMismatch,
    #[msg("Time overflow")]
    TimeOverflow,
    #[msg("Missing bump")]
    MissingBump,
    #[msg("Invalid PDA")]
    InvalidPDA,
    #[msg("Integer overflow")]
    Overflow,

}
