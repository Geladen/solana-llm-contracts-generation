use anchor_lang::prelude::*;
use anchor_lang::solana_program::{system_instruction, program as sol_program};

declare_id!("ELKFoTm5eCjHTynanj1a4dYBPbi9hmzk1zfWQVEm9PW7");

#[program]
pub mod simple_wallet {
    use super::*;

    pub fn deposit(ctx: Context<DepositOrWithdrawCtx>, amount_to_deposit: u64) -> Result<()> {
        let ix = system_instruction::transfer(
            &ctx.accounts.owner.key(),
            &ctx.accounts.user_wallet_pda.key(),
            amount_to_deposit,
        );
        sol_program::invoke(
            &ix,
            &[
                ctx.accounts.owner.to_account_info(),
                ctx.accounts.user_wallet_pda.to_account_info(),
            ],
        )?;

        emit!(Deposit {
            sender: ctx.accounts.owner.key(),
            amount: amount_to_deposit,
            balance: ctx.accounts.user_wallet_pda.lamports(),
        });

        Ok(())
    }

    pub fn withdraw(ctx: Context<DepositOrWithdrawCtx>, amount_to_withdraw: u64) -> Result<()> {
        require!(
            ctx.accounts.user_wallet_pda.lamports() >= amount_to_withdraw,
            ErrorCode::InsufficientFunds
        );

        let seeds = &[
            b"wallet".as_ref(),
            ctx.accounts.owner.key.as_ref(),
            &[ctx.bumps.user_wallet_pda],
        ];

        let ix = system_instruction::transfer(
            &ctx.accounts.user_wallet_pda.key(),
            &ctx.accounts.owner.key(),
            amount_to_withdraw,
        );

        sol_program::invoke_signed(
            &ix,
            &[
                ctx.accounts.user_wallet_pda.to_account_info(),
                ctx.accounts.owner.to_account_info(),
                ctx.accounts.system_program.to_account_info(),
            ],
            &[seeds],
        )?;

        emit!(Withdraw {
            sender: ctx.accounts.owner.key(),
            amount: amount_to_withdraw,
            balance: ctx.accounts.user_wallet_pda.lamports(),
        });

        Ok(())
    }

    pub fn create_transaction(
        ctx: Context<CreateTransactionCtx>,
        transaction_seed: String,
        transaction_lamports_amount: u64,
    ) -> Result<()> {
        // Copy key first
        let tx_key = ctx.accounts.transaction_pda.key();
        let receiver_key = ctx.accounts.receiver.key();

        // Now mutate transaction PDA
        let tx = &mut ctx.accounts.transaction_pda;
        tx.receiver = ctx.accounts.receiver.key();
        tx.amount_in_lamports = transaction_lamports_amount;
        tx.executed = false;

        emit!(SubmitTransaction {
            transaction_pda: tx_key,
            receiver: receiver_key,
            amount: tx.amount_in_lamports,
        });

        Ok(())
    }

    pub fn execute_transaction(
        ctx: Context<ExecuteTransactionCtx>,
        _transaction_seed: String,
    ) -> Result<()> {
        // Copy keys first
        let tx_key = ctx.accounts.transaction_pda.key();
        let receiver_key = ctx.accounts.transaction_pda.receiver;

        // Mutable borrow
        let tx = &mut ctx.accounts.transaction_pda;
        require!(!tx.executed, ErrorCode::TransactionAlreadyExecuted);
        require!(
            ctx.accounts.user_wallet_pda.lamports() >= tx.amount_in_lamports,
            ErrorCode::InsufficientFunds
        );

        let seeds = &[
            b"wallet".as_ref(),
            ctx.accounts.owner.key.as_ref(),
            &[ctx.bumps.user_wallet_pda],
        ];

        let ix = system_instruction::transfer(
            &ctx.accounts.user_wallet_pda.key(),
            &tx.receiver,
            tx.amount_in_lamports,
        );

        sol_program::invoke_signed(
            &ix,
            &[
                ctx.accounts.user_wallet_pda.to_account_info(),
                ctx.accounts.receiver.to_account_info(),
                ctx.accounts.system_program.to_account_info(),
            ],
            &[seeds],
        )?;

        tx.executed = true;

        emit!(ExecuteTransaction {
            transaction_pda: tx_key,
            receiver: receiver_key,
            amount: tx.amount_in_lamports,
        });

        Ok(())
    }
}

#[derive(Accounts)]
pub struct DepositOrWithdrawCtx<'info> {
    #[account(mut)]
    pub owner: Signer<'info>,

    /// CHECK: PDA holds lamports only; no account data
    #[account(
        mut,
        seeds = ["wallet".as_ref(), owner.key().as_ref()],
        bump
    )]
    pub user_wallet_pda: AccountInfo<'info>,

    pub system_program: Program<'info, System>,
}

#[derive(Accounts)]
#[instruction(transaction_seed: String)]
pub struct CreateTransactionCtx<'info> {
    #[account(mut)]
    pub owner: Signer<'info>,

    /// CHECK: PDA holds lamports only
    #[account(mut, seeds = ["wallet".as_ref(), owner.key().as_ref()], bump)]
    pub user_wallet_pda: AccountInfo<'info>,

    #[account(
        init,
        payer = owner,
        space = 8 + 32 + 8 + 1,
        seeds = [transaction_seed.as_bytes(), user_wallet_pda.key.as_ref()],
        bump
    )]
    pub transaction_pda: Account<'info, UserTransaction>,

    /// CHECK
    pub receiver: AccountInfo<'info>,

    pub system_program: Program<'info, System>,
}

#[derive(Accounts)]
#[instruction(transaction_seed: String)]
pub struct ExecuteTransactionCtx<'info> {
    #[account(mut)]
    pub owner: Signer<'info>,

    /// CHECK: PDA holds lamports only
    #[account(mut, seeds = ["wallet".as_ref(), owner.key().as_ref()], bump)]
    pub user_wallet_pda: AccountInfo<'info>,

    #[account(
        mut,
        close = owner,
        seeds = [transaction_seed.as_bytes(), user_wallet_pda.key.as_ref()],
        bump
    )]
    pub transaction_pda: Account<'info, UserTransaction>,

    /// CHECK
    #[account(mut)]
    pub receiver: AccountInfo<'info>,

    pub system_program: Program<'info, System>,
}

#[account]
pub struct UserTransaction {
    pub receiver: Pubkey,
    pub amount_in_lamports: u64,
    pub executed: bool,
}

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
    pub transaction_pda: Pubkey,
    pub receiver: Pubkey,
    pub amount: u64,
}

#[event]
pub struct ExecuteTransaction {
    pub transaction_pda: Pubkey,
    pub receiver: Pubkey,
    pub amount: u64,
}

#[error_code]
pub enum ErrorCode {
    #[msg("Insufficient funds in wallet.")]
    InsufficientFunds,
    #[msg("Transaction has already been executed.")]
    TransactionAlreadyExecuted,
}
