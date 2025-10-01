use anchor_lang::prelude::*;

declare_id!("56wE4rB5RVVno2sqv7GGzYMsJ7rtbs5QwsfryJUDbdK5");

#[program]
pub mod simple_wallet {
    use super::*;

    // -------------------
    // Deposit lamports into wallet PDA (auto-initialize)
    // -------------------
    pub fn deposit(ctx: Context<DepositOrWithdrawCtx>, amount_to_deposit: u64) -> Result<()> {
        let owner = &ctx.accounts.owner;
        let wallet = &ctx.accounts.user_wallet_pda;

        require!(amount_to_deposit > 0, WalletError::InvalidAmount);

        let ix = anchor_lang::solana_program::system_instruction::transfer(
            &owner.key(),
            &wallet.key(),
            amount_to_deposit,
        );
        anchor_lang::solana_program::program::invoke(
            &ix,
            &[
                owner.to_account_info(),
                wallet.to_account_info(),
                ctx.accounts.system_program.to_account_info(),
            ],
        )?;

        emit!(Deposit {
            sender: owner.key(),
            amount: amount_to_deposit,
            balance: wallet.to_account_info().lamports(),
        });

        Ok(())
    }

    // -------------------
    // Withdraw lamports from wallet PDA
    // -------------------
    pub fn withdraw(ctx: Context<DepositOrWithdrawCtx>, amount_to_withdraw: u64) -> Result<()> {
        let owner = &ctx.accounts.owner;
        let wallet = &ctx.accounts.user_wallet_pda;

        require!(amount_to_withdraw > 0, WalletError::InvalidAmount);
        require!(
            **wallet.to_account_info().lamports.borrow() >= amount_to_withdraw,
            WalletError::InsufficientFunds
        );

        **wallet.to_account_info().try_borrow_mut_lamports()? -= amount_to_withdraw;
        **owner.to_account_info().try_borrow_mut_lamports()? += amount_to_withdraw;

        emit!(Withdraw {
            sender: owner.key(),
            amount: amount_to_withdraw,
            balance: wallet.to_account_info().lamports(),
        });

        Ok(())
    }

    // -------------------
    // Create transaction
    // -------------------
    pub fn create_transaction(
        ctx: Context<CreateTransactionCtx>,
        _transaction_seed: String,
        transaction_lamports_amount: u64,
    ) -> Result<()> {
        let owner = &ctx.accounts.owner;
        let transaction = &mut ctx.accounts.transaction_pda;

        require!(transaction_lamports_amount > 0, WalletError::InvalidAmount);

        transaction.receiver = ctx.accounts.receiver.key();
        transaction.amount_in_lamports = transaction_lamports_amount;
        transaction.executed = false;

        emit!(SubmitTransaction {
            owner: owner.key(),
            receiver: transaction.receiver,
            amount: transaction_lamports_amount,
        });

        Ok(())
    }

    // -------------------
    // Execute transaction
    // -------------------
    pub fn execute_transaction(
        ctx: Context<ExecuteTransactionCtx>,
        _transaction_seed: String,
    ) -> Result<()> {
        let owner = &ctx.accounts.owner;
        let wallet = &ctx.accounts.user_wallet_pda;
        let transaction = &mut ctx.accounts.transaction_pda;
        let receiver = &ctx.accounts.receiver;

        require!(!transaction.executed, WalletError::AlreadyExecuted);
        require!(
            **wallet.to_account_info().lamports.borrow() >= transaction.amount_in_lamports,
            WalletError::InsufficientFunds
        );

        **wallet.to_account_info().try_borrow_mut_lamports()? -= transaction.amount_in_lamports;
        **receiver.to_account_info().try_borrow_mut_lamports()? += transaction.amount_in_lamports;

        transaction.executed = true;

        emit!(ExecuteTransaction {
            owner: owner.key(),
            receiver: receiver.key(),
            amount: transaction.amount_in_lamports,
        });

        Ok(())
    }
}

//
// -------------------
// Account Contexts
// -------------------
//

#[derive(Accounts)]
pub struct DepositOrWithdrawCtx<'info> {
    #[account(mut)]
    pub owner: Signer<'info>,

    // Auto-initialize wallet PDA if needed
    #[account(
        init_if_needed,
        payer = owner,
        space = 8, // empty struct
        seeds = [b"wallet", owner.key().as_ref()],
        bump,
    )]
    pub user_wallet_pda: Account<'info, UserWallet>,

    pub system_program: Program<'info, System>,
}

#[derive(Accounts)]
#[instruction(transaction_seed: String)]
pub struct CreateTransactionCtx<'info> {
    #[account(mut)]
    pub owner: Signer<'info>,

    #[account(
        mut,
        seeds = [b"wallet", owner.key().as_ref()],
        bump,
    )]
    pub user_wallet_pda: Account<'info, UserWallet>,

    #[account(
        init,
        payer = owner,
        space = 8 + UserTransaction::LEN,
        seeds = [transaction_seed.as_bytes(), user_wallet_pda.key().as_ref()],
        bump,
    )]
    pub transaction_pda: Account<'info, UserTransaction>,

    /// CHECK: receiver of lamports
    pub receiver: UncheckedAccount<'info>,

    pub system_program: Program<'info, System>,
}

#[derive(Accounts)]
#[instruction(transaction_seed: String)]
pub struct ExecuteTransactionCtx<'info> {
    #[account(mut)]
    pub owner: Signer<'info>,

    #[account(
        mut,
        seeds = [b"wallet", owner.key().as_ref()],
        bump,
    )]
    pub user_wallet_pda: Account<'info, UserWallet>,

    #[account(
        mut,
        close = owner,
        seeds = [transaction_seed.as_bytes(), user_wallet_pda.key().as_ref()],
        bump,
    )]
    pub transaction_pda: Account<'info, UserTransaction>,

    /// CHECK: receiver of lamports
    #[account(mut)]
    pub receiver: UncheckedAccount<'info>,

    pub system_program: Program<'info, System>,
}

//
// -------------------
// State Accounts
// -------------------
//

#[account]
pub struct UserWallet {}

#[account]
pub struct UserTransaction {
    pub receiver: Pubkey,
    pub amount_in_lamports: u64,
    pub executed: bool,
}

impl UserTransaction {
    pub const LEN: usize = 32 + 8 + 1;
}

//
// -------------------
// Events
// -------------------
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
    pub owner: Pubkey,
    pub receiver: Pubkey,
    pub amount: u64,
}

#[event]
pub struct ExecuteTransaction {
    pub owner: Pubkey,
    pub receiver: Pubkey,
    pub amount: u64,
}

//
// -------------------
// Errors
// -------------------
//

#[error_code]
pub enum WalletError {
    #[msg("Invalid lamports amount")]
    InvalidAmount,
    #[msg("Insufficient funds in wallet PDA")]
    InsufficientFunds,
    #[msg("Transaction already executed")]
    AlreadyExecuted,
}
