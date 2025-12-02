use anchor_lang::prelude::*;
use anchor_lang::solana_program::system_instruction;

declare_id!("9YhCuQR569WZofNaLcEg136m5Vm1pZMy9SATVqYp5Fus");

#[program]
pub mod simple_wallet {
    use super::*;

    // deposit: init wallet PDA if missing (zero-data PDA), then transfer lamports
    pub fn deposit(ctx: Context<DepositOrWithdrawCtx>, amount_to_deposit: u64) -> Result<()> {
        require!(amount_to_deposit > 0, WalletError::InvalidAmount);

        let owner = &ctx.accounts.owner;
        let wallet_pda = &ctx.accounts.user_wallet_pda;

        // ensure supplied PDA matches derivation and obtain bump
        let (derived, bump) =
            Pubkey::find_program_address(&[b"wallet", owner.key.as_ref()], ctx.program_id);
        require_eq!(derived, wallet_pda.key(), WalletError::InvalidPda);

        // Transfer lamports from owner to wallet PDA
        let ix = system_instruction::transfer(&owner.key(), &wallet_pda.key(), amount_to_deposit);
        anchor_lang::solana_program::program::invoke(
            &ix,
            &[
                owner.to_account_info(),
                wallet_pda.to_account_info(),
                ctx.accounts.system_program.to_account_info(),
            ],
        )?;

        let balance = **wallet_pda.to_account_info().lamports.borrow();

        emit!(Deposit {
            sender: owner.key(),
            amount: amount_to_deposit,
            balance,
        });

        Ok(())
    }

    // create_transaction: initializes a transaction PDA and stores receiver & amount
    pub fn create_transaction(
        ctx: Context<CreateTransactionCtx>,
        transaction_seed: String,
        transaction_lamports_amount: u64,
    ) -> Result<()> {
        require!(transaction_lamports_amount > 0, WalletError::InvalidAmount);

        let owner = &ctx.accounts.owner;
        let wallet_pda = &ctx.accounts.user_wallet_pda;
        let tx = &mut ctx.accounts.transaction_pda;

        // validate wallet PDA derivation
        let (derived, _bump) =
            Pubkey::find_program_address(&[b"wallet", owner.key.as_ref()], ctx.program_id);
        require_eq!(derived, wallet_pda.key(), WalletError::InvalidPda);

        // check wallet balance covers the requested transaction amount
        let wallet_balance = **wallet_pda.to_account_info().lamports.borrow();
        require!(
            wallet_balance >= transaction_lamports_amount,
            WalletError::InsufficientFundsInWallet
        );

        tx.receiver = ctx.accounts.receiver.key();
        tx.amount_in_lamports = transaction_lamports_amount;
        tx.executed = false;

        emit!(SubmitTransaction {
            owner: owner.key(),
            transaction: tx.key(),
            receiver: tx.receiver,
            amount: tx.amount_in_lamports,
        });

        Ok(())
    }

    // execute_transaction: transfer lamports from wallet PDA to receiver; close tx PDA to owner
    pub fn execute_transaction(ctx: Context<ExecuteTransactionCtx>, _transaction_seed: String) -> Result<()> {
        let owner = &ctx.accounts.owner;
        let wallet_pda = &ctx.accounts.user_wallet_pda;
        let tx = &mut ctx.accounts.transaction_pda;
        let receiver = &ctx.accounts.receiver;

        require!(!tx.executed, WalletError::TransactionAlreadyExecuted);
        require!(tx.amount_in_lamports > 0, WalletError::InvalidAmount);

        let wallet_balance = **wallet_pda.to_account_info().lamports.borrow();
        require!(
            wallet_balance >= tx.amount_in_lamports,
            WalletError::InsufficientFundsInWallet
        );

        // derive bump to sign as PDA
        let (_derived, bump) = Pubkey::find_program_address(&[b"wallet", owner.key.as_ref()], ctx.program_id);
        let seeds: &[&[u8]] = &[b"wallet", owner.key.as_ref(), &[bump]];

        let ix = system_instruction::transfer(&wallet_pda.key(), &receiver.key(), tx.amount_in_lamports);
        anchor_lang::solana_program::program::invoke_signed(
            &ix,
            &[
                wallet_pda.to_account_info(),
                receiver.to_account_info(),
                ctx.accounts.system_program.to_account_info(),
            ],
            &[seeds],
        )?;

        tx.executed = true;

        emit!(ExecuteTransaction {
            owner: owner.key(),
            transaction: tx.key(),
            receiver: receiver.key(),
            amount: tx.amount_in_lamports,
        });

        Ok(())
    }

    // withdraw: transfer lamports from wallet PDA back to owner
    pub fn withdraw(ctx: Context<DepositOrWithdrawCtx>, amount_to_withdraw: u64) -> Result<()> {
        require!(amount_to_withdraw > 0, WalletError::InvalidAmount);

        let owner = &ctx.accounts.owner;
        let wallet_pda = &ctx.accounts.user_wallet_pda;

        // derive bump and verify
        let (_derived, bump) = Pubkey::find_program_address(&[b"wallet", owner.key.as_ref()], ctx.program_id);
        let seeds: &[&[u8]] = &[b"wallet", owner.key.as_ref(), &[bump]];

        let wallet_balance = **wallet_pda.to_account_info().lamports.borrow();
        require!(
            wallet_balance >= amount_to_withdraw,
            WalletError::InsufficientFundsInWallet
        );

        let ix = system_instruction::transfer(&wallet_pda.key(), &owner.key(), amount_to_withdraw);
        anchor_lang::solana_program::program::invoke_signed(
            &ix,
            &[
                wallet_pda.to_account_info(),
                owner.to_account_info(),
                ctx.accounts.system_program.to_account_info(),
            ],
            &[seeds],
        )?;

        let balance = **wallet_pda.to_account_info().lamports.borrow();

        emit!(Withdraw {
            sender: owner.key(),
            amount: amount_to_withdraw,
            balance,
        });

        Ok(())
    }
}

// -------------------- Accounts / Contexts --------------------

#[derive(Accounts)]
pub struct DepositOrWithdrawCtx<'info> {
    /// Owner must sign and be mutable because lamports move from/to owner
    #[account(mut)]
    pub owner: Signer<'info>,

    /// CHECK: Wallet PDA is a zero-data PDA (space = 0) owned by this program and used only to hold lamports.
    /// It is declared UncheckedAccount to ensure it carries no Anchor-managed data.
    /// Seeds: ["wallet", owner.key().as_ref()]
    /// init_if_needed will create it as a program-owned account with space = 0 when missing.
    #[account(
        init_if_needed,
        payer = owner,
        seeds = [b"wallet", owner.key().as_ref()],
        bump,
        space = 0
    )]
    pub user_wallet_pda: UncheckedAccount<'info>,

    pub system_program: Program<'info, System>,
}

#[derive(Accounts)]
#[instruction(transaction_seed: String)]
pub struct CreateTransactionCtx<'info> {
    /// owner must sign
    #[account(mut)]
    pub owner: Signer<'info>,

    /// CHECK: Wallet PDA is a zero-data PDA used only to hold lamports; validated by PDA derivation at runtime.
    #[account(mut, seeds = [b"wallet", owner.key().as_ref()], bump)]
    pub user_wallet_pda: UncheckedAccount<'info>,

    /// Transaction PDA: unique per transaction_seed + wallet
    #[account(
        init,
        payer = owner,
        seeds = [transaction_seed.as_bytes(), user_wallet_pda.key().as_ref()],
        bump,
        space = 8 + 32 + 8 + 1, // discriminator + receiver(pubkey) + amount(u64) + executed(bool)
    )]
    pub transaction_pda: Box<Account<'info, UserTransactionAccount>>,

    /// CHECK: Receiver is an unchecked account used only as the destination of lamport transfers
    pub receiver: UncheckedAccount<'info>,

    pub system_program: Program<'info, System>,
}

#[derive(Accounts)]
#[instruction(transaction_seed: String)]
pub struct ExecuteTransactionCtx<'info> {
    /// owner must sign
    #[account(mut)]
    pub owner: Signer<'info>,

    /// CHECK: Wallet PDA is a zero-data PDA used only to hold lamports; validated by PDA derivation at runtime.
    #[account(mut, seeds = [b"wallet", owner.key().as_ref()], bump)]
    pub user_wallet_pda: UncheckedAccount<'info>,

    /// Transaction PDA that will be closed after execution (rent returned to owner)
    #[account(mut, seeds = [transaction_seed.as_bytes(), user_wallet_pda.key().as_ref()], bump, close = owner)]
    pub transaction_pda: Box<Account<'info, UserTransactionAccount>>,

    /// CHECK: Receiver is an unchecked account used only as the destination of lamport transfers
    #[account(mut)]
    pub receiver: UncheckedAccount<'info>,

    pub system_program: Program<'info, System>,
}


// -------------------- State Structs --------------------

// Transaction account stores receiver, amount, executed flag
#[account]
pub struct UserTransactionAccount {
    pub receiver: Pubkey,
    pub amount_in_lamports: u64,
    pub executed: bool,
}

// -------------------- Events --------------------

#[event]
pub struct Deposit {
    pub sender: Pubkey,
    pub amount: u64,
    pub balance: u64,
}

#[event]
pub struct SubmitTransaction {
    pub owner: Pubkey,
    pub transaction: Pubkey,
    pub receiver: Pubkey,
    pub amount: u64,
}

#[event]
pub struct ExecuteTransaction {
    pub owner: Pubkey,
    pub transaction: Pubkey,
    pub receiver: Pubkey,
    pub amount: u64,
}

#[event]
pub struct Withdraw {
    pub sender: Pubkey,
    pub amount: u64,
    pub balance: u64,
}

// -------------------- Errors --------------------

#[error_code]
pub enum WalletError {
    #[msg("Invalid amount, must be > 0")]
    InvalidAmount,
    #[msg("Insufficient funds in wallet")]
    InsufficientFundsInWallet,
    #[msg("Transaction already executed")]
    TransactionAlreadyExecuted,
    #[msg("Derived PDA does not match provided account")]
    InvalidPda,
}
