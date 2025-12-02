use anchor_lang::prelude::*;
use anchor_lang::solana_program::{program::invoke_signed, system_instruction};

declare_id!("E4DVdwuTuRFmnFDX5FzMp1X9kEwrpTgkrzhWT5a94VHm");

#[program]
pub mod vault {
    use super::*;

    pub fn initialize(
        ctx: Context<InitializeCtx>,
        wait_time: u64,
        initial_amount: u64,
    ) -> Result<()> {
        // Derive expected PDAs and bumps
        let (expected_info_pda, info_bump) =
            Pubkey::find_program_address(&[ctx.accounts.owner.key.as_ref()], ctx.program_id);
        require!(
            expected_info_pda == ctx.accounts.vault_info.key(),
            ErrorCode::VaultPdaMismatch
        );

        let (expected_wallet_pda, _wallet_bump) = Pubkey::find_program_address(
            &[ctx.accounts.owner.key.as_ref(), b"wallet"],
            ctx.program_id,
        );
        require!(
            expected_wallet_pda == ctx.accounts.vault_wallet.key(),
            ErrorCode::VaultPdaMismatch
        );

        // Initialize VaultInfo (no extra fields added)
        let vault = &mut ctx.accounts.vault_info;
        vault.owner = ctx.accounts.owner.key();
        vault.recovery = ctx.accounts.recovery.key();
        vault.receiver = Pubkey::default();
        vault.wait_time = wait_time;
        vault.request_time = 0;
        vault.amount = initial_amount;
        vault.state = State::Idle;
        vault.bump = info_bump;

        // Transfer initial_amount lamports from owner -> vault_wallet (owner-signed transfer)
        if initial_amount > 0 {
            let cpi_accounts = anchor_lang::system_program::Transfer {
                from: ctx.accounts.owner.to_account_info(),
                to: ctx.accounts.vault_wallet.to_account_info(),
            };
            let cpi_program = ctx.accounts.system_program.to_account_info();
            anchor_lang::system_program::transfer(
                CpiContext::new(cpi_program, cpi_accounts),
                initial_amount,
            )?;
        }

        Ok(())
    }

    pub fn withdraw(ctx: Context<WithdrawCtx>, amount: u64) -> Result<()> {
        // Validate owner and state, check funds in vault_wallet
        let vault = &mut ctx.accounts.vault_info;
        require!(
            ctx.accounts.owner.key() == vault.owner,
            ErrorCode::UnauthorizedOwner
        );
        require!(vault.state == State::Idle, ErrorCode::InvalidStateForWithdraw);

        let vault_wallet_lamports = **ctx.accounts.vault_wallet.to_account_info().lamports.borrow();
        require!(vault_wallet_lamports >= amount, ErrorCode::InsufficientVaultFunds);

        vault.receiver = ctx.accounts.receiver.key();
        vault.amount = amount;
        vault.request_time = Clock::get()?.slot;
        vault.state = State::Req;

        Ok(())
    }

    pub fn finalize(ctx: Context<FinalizeCtx>) -> Result<()> {
        let clock = Clock::get()?;

        // Scoped mutable borrow: validate and copy needed fields, then drop borrow
        let amount = {
            let vault = &mut ctx.accounts.vault_info;

            require!(
                ctx.accounts.owner.key() == vault.owner,
                ErrorCode::UnauthorizedOwner
            );
            require!(vault.state == State::Req, ErrorCode::NoPendingRequest);
            require!(
                ctx.accounts.receiver.key() == vault.receiver,
                ErrorCode::ReceiverMismatch
            );

            let ready_slot = vault
                .request_time
                .checked_add(vault.wait_time)
                .ok_or(ErrorCode::Overflow)?;
            require!(clock.slot >= ready_slot, ErrorCode::WaitTimeNotElapsed);

            let amount = vault.amount;
            require!(amount > 0, ErrorCode::ZeroAmount);

            amount
        }; // mutable borrow ends here

        // Ensure vault_wallet has enough lamports
        let vault_wallet_lamports = **ctx.accounts.vault_wallet.to_account_info().lamports.borrow();
        require!(vault_wallet_lamports >= amount, ErrorCode::InsufficientVaultFunds);

        // Derive wallet bump for signer seeds
        let (_wallet_pda, wallet_bump) = Pubkey::find_program_address(
            &[ctx.accounts.owner.key.as_ref(), b"wallet"],
            ctx.program_id,
        );

        // Build transfer instruction: vault_wallet -> receiver
        let from_ai = ctx.accounts.vault_wallet.to_account_info();
        let to_ai = ctx.accounts.receiver.to_account_info();
        let ix = system_instruction::transfer(from_ai.key, to_ai.key, amount);

        // Proper signer seeds: [owner.key().as_ref(), b"wallet", &[wallet_bump]]
        let seed_owner: &[u8] = ctx.accounts.owner.key.as_ref();
        let seed_wallet: &[u8] = b"wallet";
        let seed_bump: &[u8] = &[wallet_bump];

        let signer_seeds: &[&[u8]] = &[seed_owner, seed_wallet, seed_bump];
        let signer_seeds_arr: &[&[&[u8]]] = &[signer_seeds];

        invoke_signed(
            &ix,
            &[
                from_ai.clone(), // vault_wallet (PDA)
                to_ai.clone(),   // receiver
                ctx.accounts.system_program.to_account_info().clone(),
            ],
            signer_seeds_arr,
        )?;

        // Re-borrow vault_info mutably to reset state
        let vault = &mut ctx.accounts.vault_info;
        vault.receiver = Pubkey::default();
        vault.amount = 0;
        vault.request_time = 0;
        vault.state = State::Idle;

        Ok(())
    }

    pub fn cancel(ctx: Context<CancelCtx>) -> Result<()> {
        let vault = &mut ctx.accounts.vault_info;

        require!(
            ctx.accounts.recovery.key() == vault.recovery,
            ErrorCode::UnauthorizedRecovery
        );
        require!(vault.state == State::Req, ErrorCode::NoPendingRequest);

        vault.receiver = Pubkey::default();
        vault.amount = 0;
        vault.request_time = 0;
        vault.state = State::Idle;

        Ok(())
    }
}

#[derive(Accounts)]
#[instruction(wait_time: u64, initial_amount: u64)]
pub struct InitializeCtx<'info> {
    #[account(mut, signer)]
    pub owner: Signer<'info>,

    /// CHECK: recovery is stored as a Pubkey for off-chain recovery; no on-chain authority checks needed
    pub recovery: UncheckedAccount<'info>,

    /// VaultInfo PDA storing metadata (seeds = [owner.key().as_ref()])
    #[account(
        init,
        payer = owner,
        space = VaultInfo::SPACE,
        seeds = [owner.key().as_ref()],
        bump
    )]
    pub vault_info: Account<'info, VaultInfo>,

    /// CHECK: vault_wallet is a PDA that holds lamports only (space = 0). It is created here
    /// and used as the source of transfers; it must be unchecked because it carries no data.
    #[account(
        init,
        payer = owner,
        space = 0,
        seeds = [owner.key().as_ref(), b"wallet"],
        bump
    )]
    pub vault_wallet: UncheckedAccount<'info>,

    pub system_program: Program<'info, System>,
}

#[derive(Accounts)]
pub struct WithdrawCtx<'info> {
    #[account(signer)]
    pub owner: Signer<'info>,

    /// CHECK: receiver is a destination pubkey for finalized withdrawals
    pub receiver: UncheckedAccount<'info>,

    /// Vault metadata PDA (seeds = [owner.key().as_ref()])
    #[account(mut, seeds = [owner.key().as_ref()], bump = vault_info.bump)]
    pub vault_info: Account<'info, VaultInfo>,

    /// CHECK: vault_wallet is a PDA (space = 0) that holds lamports; it is unchecked because it stores no data
    #[account(mut, seeds = [owner.key().as_ref(), b"wallet"], bump)]
    pub vault_wallet: UncheckedAccount<'info>,
}

#[derive(Accounts)]
pub struct FinalizeCtx<'info> {
    #[account(signer)]
    pub owner: Signer<'info>,

    /// CHECK: receiver will receive lamports
    #[account(mut)]
    pub receiver: UncheckedAccount<'info>,

    /// Vault metadata PDA
    #[account(mut, seeds = [owner.key().as_ref()], bump = vault_info.bump)]
    pub vault_info: Account<'info, VaultInfo>,

    /// CHECK: vault_wallet is a PDA (space = 0) that holds lamports; used as the transfer source
    #[account(mut, seeds = [owner.key().as_ref(), b"wallet"], bump)]
    pub vault_wallet: UncheckedAccount<'info>,

    pub system_program: Program<'info, System>,
}

#[derive(Accounts)]
pub struct CancelCtx<'info> {
    #[account(signer)]
    pub recovery: Signer<'info>,

    /// CHECK: owner used only as PDA seed reference
    pub owner: UncheckedAccount<'info>,

    /// Vault metadata PDA
    #[account(mut, seeds = [owner.key().as_ref()], bump = vault_info.bump)]
    pub vault_info: Account<'info, VaultInfo>,

    /// CHECK: vault_wallet is included for completeness; it is a PDA (space = 0) and carries no data
    #[account(mut, seeds = [owner.key().as_ref(), b"wallet"], bump)]
    pub vault_wallet: UncheckedAccount<'info>,
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
    pub const SPACE: usize = 8   // discriminator
        + 32 // owner
        + 32 // recovery
        + 32 // receiver
        + 8  // wait_time
        + 8  // request_time
        + 8  // amount
        + 1  // state
        + 1; // bump
}

#[derive(AnchorSerialize, AnchorDeserialize, Clone, PartialEq, Eq)]
#[repr(u8)]
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
pub enum ErrorCode {
    #[msg("Unauthorized: caller is not the owner")]
    UnauthorizedOwner,
    #[msg("Unauthorized: caller is not the recovery key")]
    UnauthorizedRecovery,
    #[msg("Vault has insufficient funds")]
    InsufficientVaultFunds,
    #[msg("Invalid state for withdraw")]
    InvalidStateForWithdraw,
    #[msg("No pending withdrawal request")]
    NoPendingRequest,
    #[msg("Wait time has not elapsed")]
    WaitTimeNotElapsed,
    #[msg("Receiver does not match the recorded receiver")]
    ReceiverMismatch,
    #[msg("Requested amount is zero")]
    ZeroAmount,
    #[msg("Integer overflow")]
    Overflow,
    #[msg("Vault PDA mismatch")]
    VaultPdaMismatch,
    #[msg("Bump is missing")]
    MissingBump,
}
