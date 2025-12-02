
use anchor_lang::prelude::*;
use anchor_lang::solana_program::{program::invoke_signed, system_instruction};

declare_id!("Fvr76ebtUkmsb29D9ym9Pi93e9BuMqQ6a84auxRYrGPg");

#[program]
pub mod simple_wallet {
    use super::*;

    /// Explicit initializer to create the system-owned wallet PDA with space = 0.
    pub fn initialize_wallet(ctx: Context<InitializeWalletCtx>) -> Result<()> {
        // Nothing to store; this instruction only ensures the PDA exists and is owned by the system program.
        Ok(())
    }

    pub fn deposit(ctx: Context<DepositOrWithdrawCtx>, amount_to_deposit: u64) -> Result<()> {
        require!(amount_to_deposit > 0, ErrorCode::InvalidAmount);

        let owner_info = ctx.accounts.owner.to_account_info();
        let wallet_info = ctx.accounts.user_wallet_pda.to_account_info();

        // transfer lamports from owner to wallet PDA (system account)
        let ix = system_instruction::transfer(owner_info.key, wallet_info.key, amount_to_deposit);
        anchor_lang::solana_program::program::invoke(
            &ix,
            &[
                owner_info.clone(),
                wallet_info.clone(),
                ctx.accounts.system_program.to_account_info(),
            ],
        )?;

        let balance = wallet_info.lamports();
        emit!(Deposit {
            sender: *ctx.accounts.owner.key,
            amount: amount_to_deposit,
            balance,
        });

        Ok(())
    }

    pub fn create_transaction(
        ctx: Context<CreateTransactionCtx>,
        transaction_seed: String,
        transaction_lamports_amount: u64,
    ) -> Result<()> {
        require!(transaction_lamports_amount > 0, ErrorCode::InvalidAmount);

        let tx_account = &mut ctx.accounts.transaction_pda;
        tx_account.receiver = ctx.accounts.receiver.key();
        tx_account.amount_in_lamports = transaction_lamports_amount;
        tx_account.executed = false;

        emit!(SubmitTransaction {
            submitter: *ctx.accounts.owner.key,
            transaction_seed,
            receiver: tx_account.receiver,
            amount: tx_account.amount_in_lamports,
        });

        Ok(())
    }

    pub fn execute_transaction(ctx: Context<ExecuteTransactionCtx>, _transaction_seed: String) -> Result<()> {
        let tx_account = &mut ctx.accounts.transaction_pda;
        require!(!tx_account.executed, ErrorCode::AlreadyExecuted);
        let amount = tx_account.amount_in_lamports;
        require!(amount > 0, ErrorCode::InvalidAmount);
        require!(tx_account.receiver == ctx.accounts.receiver.key(), ErrorCode::ReceiverMismatch);

        let wallet_info = ctx.accounts.user_wallet_pda.to_account_info();
        require!(wallet_info.lamports() >= amount, ErrorCode::InsufficientFunds);

        // recompute bump and verify PDA is correct
        let (pda, bump) = Pubkey::find_program_address(&[b"wallet", ctx.accounts.owner.key.as_ref()], ctx.program_id);
        require!(pda == ctx.accounts.user_wallet_pda.key(), ErrorCode::PdaMismatch);

        let bump_arr = [bump];
        let seeds: &[&[u8]] = &[b"wallet", ctx.accounts.owner.key.as_ref(), &bump_arr];

        let ix = system_instruction::transfer(wallet_info.key, ctx.accounts.receiver.key, amount);
        invoke_signed(
            &ix,
            &[
                wallet_info.clone(),
                ctx.accounts.receiver.to_account_info().clone(),
                ctx.accounts.system_program.to_account_info(),
            ],
            &[seeds],
        )?;

        tx_account.executed = true;

        emit!(ExecuteTransaction {
            executor: *ctx.accounts.owner.key,
            receiver: ctx.accounts.receiver.key(),
            amount,
        });

        Ok(())
    }

    pub fn withdraw(ctx: Context<DepositOrWithdrawCtx>, amount_to_withdraw: u64) -> Result<()> {
        require!(amount_to_withdraw > 0, ErrorCode::InvalidAmount);

        let wallet_info = ctx.accounts.user_wallet_pda.to_account_info();
        let owner_info = ctx.accounts.owner.to_account_info();

        require!(wallet_info.lamports() >= amount_to_withdraw, ErrorCode::InsufficientFunds);

        let (pda, bump) = Pubkey::find_program_address(&[b"wallet", ctx.accounts.owner.key.as_ref()], ctx.program_id);
        require!(pda == ctx.accounts.user_wallet_pda.key(), ErrorCode::PdaMismatch);

        let bump_arr = [bump];
        let seeds: &[&[u8]] = &[b"wallet", ctx.accounts.owner.key.as_ref(), &bump_arr];

        let ix = system_instruction::transfer(wallet_info.key, owner_info.key, amount_to_withdraw);
        invoke_signed(
            &ix,
            &[
                wallet_info.clone(),
                owner_info.clone(),
                ctx.accounts.system_program.to_account_info(),
            ],
            &[seeds],
        )?;

        let balance = wallet_info.lamports();
        emit!(Withdraw {
            sender: *ctx.accounts.owner.key,
            amount: amount_to_withdraw,
            balance,
        });

        Ok(())
    }
}

/***** Accounts *****/

#[derive(Accounts)]
pub struct InitializeWalletCtx<'info> {
    #[account(mut)]
    pub owner: Signer<'info>,

    /// Create the system-owned PDA (space = 0) — payer = owner
    /// CHECK: This PDA is created as a system account (owner = System) and holds no data; no further checks.
    #[account(
        init,
        payer = owner,
        space = 0,
        seeds = [b"wallet", owner.key().as_ref()],
        bump,
        owner = System::id(),
    )]
    pub user_wallet_pda: UncheckedAccount<'info>,

    pub system_program: Program<'info, System>,
    pub rent: Sysvar<'info, Rent>,
}

#[derive(Accounts)]
pub struct DepositOrWithdrawCtx<'info> {
    #[account(mut)]
    pub owner: Signer<'info>,

    /// Wallet PDA must already exist and is mutable since lamports are moved
    /// CHECK: This is a system-owned PDA (owner = System), zero-data account used purely as lamports holder.
    #[account(
        mut,
        seeds = [b"wallet", owner.key().as_ref()],
        bump,
        owner = System::id(),
    )]
    pub user_wallet_pda: UncheckedAccount<'info>,

    pub system_program: Program<'info, System>,
}

#[derive(Accounts)]
#[instruction(transaction_seed: String)]
pub struct CreateTransactionCtx<'info> {
    #[account(mut)]
    pub owner: Signer<'info>,

    /// Wallet PDA must exist (system-owned)
    /// CHECK: system-owned PDA (zero-data) used as lamports holder.
    #[account(
        seeds = [b"wallet", owner.key().as_ref()],
        bump,
        owner = System::id(),
    )]
    pub user_wallet_pda: UncheckedAccount<'info>,

    #[account(
        init,
        payer = owner,
        space = 8 + UserTransaction::LEN,
        seeds = [transaction_seed.as_bytes(), user_wallet_pda.key().as_ref()],
        bump
    )]
    pub transaction_pda: Account<'info, UserTransaction>,

    /// Receiver (any account)
    /// CHECK: receiver is stored in the transaction account and used as the destination for lamports; it is not validated here.
    #[account(mut)]
    pub receiver: UncheckedAccount<'info>,

    pub system_program: Program<'info, System>,
}

#[derive(Accounts)]
#[instruction(transaction_seed: String)]
pub struct ExecuteTransactionCtx<'info> {
    #[account(mut)]
    pub owner: Signer<'info>,

    /// Wallet PDA must be mutable here
    /// CHECK: system-owned PDA (zero-data) used as lamports holder.
    #[account(
        mut,
        seeds = [b"wallet", owner.key().as_ref()],
        bump,
        owner = System::id(),
    )]
    pub user_wallet_pda: UncheckedAccount<'info>,

    /// Transaction PDA; closed to owner after execution
    #[account(
        mut,
        seeds = [transaction_seed.as_bytes(), user_wallet_pda.key().as_ref()],
        bump,
        close = owner
    )]
    pub transaction_pda: Account<'info, UserTransaction>,

    /// Receiver must be mutable to receive lamports
    /// CHECK: will receive lamports; no additional checks performed here.
    #[account(mut)]
    pub receiver: UncheckedAccount<'info>,

    pub system_program: Program<'info, System>,
}

/***** State, events, errors *****/

#[account]
pub struct UserTransaction {
    pub receiver: Pubkey,
    pub amount_in_lamports: u64,
    pub executed: bool,
}

impl UserTransaction {
    pub const LEN: usize = 32 + 8 + 1;
}

#[event]
pub struct Deposit {
    pub sender: Pubkey,
    pub amount: u64,
    pub balance: u64,
}

#[event]
pub struct SubmitTransaction {
    pub submitter: Pubkey,
    pub transaction_seed: String,
    pub receiver: Pubkey,
    pub amount: u64,
}

#[event]
pub struct ExecuteTransaction {
    pub executor: Pubkey,
    pub receiver: Pubkey,
    pub amount: u64,
}

#[event]
pub struct Withdraw {
    pub sender: Pubkey,
    pub amount: u64,
    pub balance: u64,
}

#[error_code]
pub enum ErrorCode {
    #[msg("Insufficient funds in wallet")]
    InsufficientFunds,
    #[msg("Transaction already executed")]
    AlreadyExecuted,
    #[msg("Invalid amount")]
    InvalidAmount,
    #[msg("Receiver mismatch")]
    ReceiverMismatch,
    #[msg("PDA mismatch")]
    PdaMismatch,
}
