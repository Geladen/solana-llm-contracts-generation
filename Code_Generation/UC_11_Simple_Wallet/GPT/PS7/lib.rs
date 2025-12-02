use anchor_lang::prelude::*;

declare_id!("8ZwghmeVVwmiQVsVcAb1xPEK6cLANzdVGz2dGaixtqog");

#[program]
pub mod simple_wallet {
    use super::*;

    // Deposit funds into the wallet PDA
    pub fn deposit(ctx: Context<DepositOrWithdrawCtx>, amount_to_deposit: u64) -> Result<()> {
        let owner = &ctx.accounts.owner;
        let wallet_pda = &ctx.accounts.user_wallet_pda;

        // Transfer lamports from owner to wallet PDA
        let cpi_ctx = CpiContext::new(
            ctx.accounts.system_program.to_account_info(),
            anchor_lang::system_program::Transfer {
                from: owner.to_account_info(),
                to: wallet_pda.to_account_info(),
            },
        );
        anchor_lang::system_program::transfer(cpi_ctx, amount_to_deposit)?;

        emit!(Deposit {
            sender: owner.key(),
            amount: amount_to_deposit,
            balance: wallet_pda.to_account_info().lamports(),
        });

        Ok(())
    }

    // Create a pending transaction PDA
    pub fn create_transaction(
        ctx: Context<CreateTransactionCtx>,
        transaction_seed: String,
        transaction_lamports_amount: u64,
    ) -> Result<()> {
        let transaction = &mut ctx.accounts.transaction_pda;

        transaction.receiver = ctx.accounts.receiver.key();
        transaction.amount_in_lamports = transaction_lamports_amount;
        transaction.executed = false;

        emit!(SubmitTransaction {
            sender: ctx.accounts.owner.key(),
            receiver: transaction.receiver,
            amount: transaction.amount_in_lamports,
        });

        Ok(())
    }

    // Execute a transaction: transfer from wallet PDA to receiver
    pub fn execute_transaction(
        ctx: Context<ExecuteTransactionCtx>,
        _transaction_seed: String,
    ) -> Result<()> {
        let transaction = &mut ctx.accounts.transaction_pda;
        let wallet_pda = &ctx.accounts.user_wallet_pda;

        require!(!transaction.executed, WalletError::TransactionAlreadyExecuted);
        require!(
            **wallet_pda.to_account_info().lamports.borrow() >= transaction.amount_in_lamports,
            WalletError::InsufficientFunds
        );

        // Transfer lamports to receiver
        **wallet_pda.to_account_info().try_borrow_mut_lamports()? -= transaction.amount_in_lamports;
        **ctx.accounts.receiver.to_account_info().try_borrow_mut_lamports()? += transaction.amount_in_lamports;

        transaction.executed = true;

        emit!(ExecuteTransaction {
            sender: ctx.accounts.owner.key(),
            receiver: transaction.receiver,
            amount: transaction.amount_in_lamports,
        });

        Ok(())
    }

    // Withdraw funds from wallet PDA to owner
    pub fn withdraw(ctx: Context<DepositOrWithdrawCtx>, amount_to_withdraw: u64) -> Result<()> {
        let owner = &ctx.accounts.owner;
        let wallet_pda = &ctx.accounts.user_wallet_pda;

        require!(
            **wallet_pda.to_account_info().lamports.borrow() >= amount_to_withdraw,
            WalletError::InsufficientFunds
        );

        **wallet_pda.to_account_info().try_borrow_mut_lamports()? -= amount_to_withdraw;
        **owner.to_account_info().try_borrow_mut_lamports()? += amount_to_withdraw;

        emit!(Withdraw {
            sender: owner.key(),
            amount: amount_to_withdraw,
            balance: wallet_pda.to_account_info().lamports(),
        });

        Ok(())
    }
}

#[derive(Accounts)]
pub struct DepositOrWithdrawCtx<'info> {
    #[account(mut, signer)]
    /// CHECK: owner signs the transaction
    pub owner: AccountInfo<'info>,

    #[account(
        seeds = [b"wallet".as_ref(), owner.key().as_ref()],
        bump,
        init_if_needed,
        payer = owner,
        space = 8 + std::mem::size_of::<UserWallet>(),
    )]
    pub user_wallet_pda: Account<'info, UserWallet>,

    pub system_program: Program<'info, System>,
}

#[derive(Accounts)]
#[instruction(transaction_seed: String)]
pub struct CreateTransactionCtx<'info> {
    #[account(mut, signer)]
    /// CHECK: owner signs the transaction
    pub owner: AccountInfo<'info>,

    #[account(
        seeds = [b"wallet".as_ref(), owner.key().as_ref()],
        bump,
        init_if_needed,
        payer = owner,
        space = 8 + std::mem::size_of::<UserWallet>(),
    )]
    pub user_wallet_pda: Account<'info, UserWallet>,

    #[account(
        init,
        payer = owner,
        seeds = [transaction_seed.as_bytes(), user_wallet_pda.key().as_ref()],
        bump,
        space = 8 + std::mem::size_of::<UserTransaction>(),
    )]
    pub transaction_pda: Account<'info, UserTransaction>,

    /// CHECK: receiver can be any account
    #[account(mut)]
    pub receiver: AccountInfo<'info>,

    pub system_program: Program<'info, System>,
}


#[derive(Accounts)]
#[instruction(transaction_seed: String)]
pub struct ExecuteTransactionCtx<'info> {
    #[account(mut, signer)]
    /// CHECK: owner signs the transaction
    pub owner: AccountInfo<'info>,

    #[account(
        seeds = [b"wallet".as_ref(), owner.key().as_ref()],
        bump,
        init_if_needed,
        payer = owner,
        space = 8 + std::mem::size_of::<UserWallet>(),
    )]
    pub user_wallet_pda: Account<'info, UserWallet>,

    #[account(
        mut,
        seeds = [transaction_seed.as_bytes(), user_wallet_pda.key().as_ref()],
        bump,
        close = owner
    )]
    pub transaction_pda: Account<'info, UserTransaction>,

    /// CHECK: receiver can be any account
    #[account(mut)]
    pub receiver: AccountInfo<'info>,

    pub system_program: Program<'info, System>,
}

// PDA State
#[account]
pub struct UserWallet {} // Empty struct placeholder

#[account]
pub struct UserTransaction {
    pub receiver: Pubkey,
    pub amount_in_lamports: u64,
    pub executed: bool,
}

// Events
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

// Errors
#[error_code]
pub enum WalletError {
    #[msg("Insufficient funds in wallet.")]
    InsufficientFunds,
    #[msg("Transaction has already been executed.")]
    TransactionAlreadyExecuted,
}
