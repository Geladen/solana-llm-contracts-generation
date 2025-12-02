use anchor_lang::prelude::*;
use anchor_lang::system_program;
use anchor_lang::solana_program::system_instruction;

declare_id!("B3JB8a3YZ3ZqeVVMutFX1MjeBSqyFSMZUjseKom81W79");

#[program]
pub mod simple_wallet {
    use super::*;

    pub fn deposit(ctx: Context<DepositOrWithdrawCtx>, amount_to_deposit: u64) -> Result<()> {
        require!(amount_to_deposit > 0, WalletError::InvalidAmount);

        let ix = system_instruction::transfer(
            ctx.accounts.owner.to_account_info().key,
            ctx.accounts.user_wallet_pda.to_account_info().key,
            amount_to_deposit,
        );
        anchor_lang::solana_program::program::invoke(
            &ix,
            &[
                ctx.accounts.owner.to_account_info(),
                ctx.accounts.user_wallet_pda.to_account_info(),
                ctx.accounts.system_program.to_account_info(),
            ],
        )?;

        let balance = ctx.accounts.user_wallet_pda.to_account_info().lamports();
        emit!(Deposit {
            sender: ctx.accounts.owner.key(),
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
        require!(transaction_lamports_amount > 0, WalletError::InvalidAmount);

        let tx_account = &mut ctx.accounts.transaction_pda;
        tx_account.receiver = ctx.accounts.receiver.key();
        tx_account.amount_in_lamports = transaction_lamports_amount;
        tx_account.executed = false;

        emit!(SubmitTransaction {
            tx_seed: transaction_seed,
            sender: ctx.accounts.owner.key(),
            receiver: tx_account.receiver,
            amount: transaction_lamports_amount,
        });

        Ok(())
    }

    pub fn execute_transaction(
        ctx: Context<ExecuteTransactionCtx>,
        _transaction_seed: String,
    ) -> Result<()> {
        let tx_account = &mut ctx.accounts.transaction_pda;

        require!(!tx_account.executed, WalletError::AlreadyExecuted);

        let amount = tx_account.amount_in_lamports;
        require!(amount > 0, WalletError::InvalidAmount);

        let wallet_lamports = ctx.accounts.user_wallet_pda.to_account_info().lamports();
        require!(wallet_lamports >= amount, WalletError::InsufficientFunds);

        // Mark executed before transfer to avoid re-execution on success/retry
        tx_account.executed = true;

        // Derive bump for wallet PDA
        let (_pda, wallet_bump) =
            Pubkey::find_program_address(&[b"wallet", ctx.accounts.owner.key.as_ref()], &ID);

        let signer_seeds: &[&[u8]] = &[
            b"wallet".as_ref(),
            ctx.accounts.owner.key.as_ref(),
            &[wallet_bump],
        ];

        let ix = system_instruction::transfer(
            ctx.accounts.user_wallet_pda.to_account_info().key,
            ctx.accounts.receiver.to_account_info().key,
            amount,
        );
        anchor_lang::solana_program::program::invoke_signed(
            &ix,
            &[
                ctx.accounts.user_wallet_pda.to_account_info(),
                ctx.accounts.receiver.to_account_info(),
                ctx.accounts.system_program.to_account_info(),
            ],
            &[signer_seeds],
        )?;

        emit!(ExecuteTransaction {
            sender: ctx.accounts.owner.key(),
            receiver: ctx.accounts.receiver.key(),
            amount,
        });

        Ok(())
    }

    pub fn withdraw(ctx: Context<DepositOrWithdrawCtx>, amount_to_withdraw: u64) -> Result<()> {
        require!(amount_to_withdraw > 0, WalletError::InvalidAmount);

        // ensure wallet has funds
        let wallet_lamports = ctx.accounts.user_wallet_pda.to_account_info().lamports();
        require!(wallet_lamports >= amount_to_withdraw, WalletError::InsufficientFunds);

        // Derive bump for wallet PDA
        let (_pda, wallet_bump) =
            Pubkey::find_program_address(&[b"wallet", ctx.accounts.owner.key.as_ref()], &ID);

        let signer_seeds: &[&[u8]] = &[
            b"wallet".as_ref(),
            ctx.accounts.owner.key.as_ref(),
            &[wallet_bump],
        ];

        let ix = system_instruction::transfer(
            ctx.accounts.user_wallet_pda.to_account_info().key,
            ctx.accounts.owner.to_account_info().key,
            amount_to_withdraw,
        );
        anchor_lang::solana_program::program::invoke_signed(
            &ix,
            &[
                ctx.accounts.user_wallet_pda.to_account_info(),
                ctx.accounts.owner.to_account_info(),
                ctx.accounts.system_program.to_account_info(),
            ],
            &[signer_seeds],
        )?;

        let balance = ctx.accounts.user_wallet_pda.to_account_info().lamports();
        emit!(Withdraw {
            sender: ctx.accounts.owner.key(),
            amount: amount_to_withdraw,
            balance,
        });

        Ok(())
    }
}

//
// State
//

#[account]
pub struct UserTransaction {
    pub receiver: Pubkey,
    pub amount_in_lamports: u64,
    pub executed: bool,
}

//
// Accounts contexts
//

#[derive(Accounts)]
pub struct DepositOrWithdrawCtx<'info> {
    #[account(mut)]
    pub owner: Signer<'info>,

    /// CHECK: Wallet PDA is a system-owned account with zero data so it can be used as `from` in system transfers.
    /// Created on demand with init_if_needed; payer = owner.
    #[account(
        init_if_needed,
        payer = owner,
        space = 0,
        owner = system_program::ID,
        seeds = [b"wallet".as_ref(), owner.key().as_ref()],
        bump
    )]
    pub user_wallet_pda: UncheckedAccount<'info>,

    pub system_program: Program<'info, System>,
}

#[derive(Accounts)]
#[instruction(transaction_seed: String)]
pub struct CreateTransactionCtx<'info> {
    #[account(mut)]
    pub owner: Signer<'info>,

    /// CHECK: Wallet PDA is a system-owned account with zero data; create if needed.
    #[account(
        init_if_needed,
        payer = owner,
        space = 0,
        owner = system_program::ID,
        seeds = [b"wallet".as_ref(), owner.key().as_ref()],
        bump
    )]
    pub user_wallet_pda: UncheckedAccount<'info>,

    /// Transaction PDA initialized here; payer = owner
    #[account(
        init,
        payer = owner,
        space = 8 + 32 + 8 + 1,
        seeds = [transaction_seed.as_bytes(), user_wallet_pda.to_account_info().key.as_ref()],
        bump
    )]
    pub transaction_pda: Account<'info, UserTransaction>,

    /// CHECK: Receiver of the eventual transfer. Only the Pubkey is stored in the transaction account; no on-chain validation required here.
    #[account(mut)]
    pub receiver: UncheckedAccount<'info>,

    pub system_program: Program<'info, System>,
}

#[derive(Accounts)]
#[instruction(transaction_seed: String)]
pub struct ExecuteTransactionCtx<'info> {
    #[account(mut)]
    pub owner: Signer<'info>,

    /// CHECK: Wallet PDA must be the same system-owned PDA (zero-data) derived from seeds.
    /// It must be mutable because lamports will be debited.
    #[account(
        mut,
        seeds = [b"wallet".as_ref(), owner.key().as_ref()],
        bump
    )]
    pub user_wallet_pda: UncheckedAccount<'info>,

    /// Transaction PDA, closed to owner after execution
    #[account(
        mut,
        seeds = [transaction_seed.as_bytes(), user_wallet_pda.to_account_info().key.as_ref()],
        bump,
        close = owner
    )]
    pub transaction_pda: Account<'info, UserTransaction>,

    /// CHECK: Receiver of funds
    #[account(mut)]
    pub receiver: UncheckedAccount<'info>,

    pub system_program: Program<'info, System>,
}

//
// Events
//

#[event]
pub struct Deposit {
    pub sender: Pubkey,
    pub amount: u64,
    pub balance: u64,
}

#[event]
pub struct Withdraw {
    pub sender: Pubkey,
    pub amount: u64,
    pub balance: u64,
}

#[event]
pub struct SubmitTransaction {
    pub tx_seed: String,
    pub sender: Pubkey,
    pub receiver: Pubkey,
    pub amount: u64,
}

#[event]
pub struct ExecuteTransaction {
    pub sender: Pubkey,
    pub receiver: Pubkey,
    pub amount: u64,
}

//
// Errors
//

#[error_code]
pub enum WalletError {
    #[msg("Invalid amount; must be > 0")]
    InvalidAmount,
    #[msg("Insufficient funds in wallet")]
    InsufficientFunds,
    #[msg("Transaction already executed")]
    AlreadyExecuted,
}
