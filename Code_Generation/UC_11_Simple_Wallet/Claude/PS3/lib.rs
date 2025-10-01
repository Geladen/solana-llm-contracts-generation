use anchor_lang::prelude::*;

declare_id!("3vc6hicDBsn8NaLz7rDJmT2gUXjBRwcZxGymkfkg7YCN");

#[program]
pub mod simple_wallet {
    use super::*;

    pub fn deposit(ctx: Context<DepositOrWithdrawCtx>, amount_to_deposit: u64) -> Result<()> {
        require!(amount_to_deposit > 0, WalletError::InvalidAmount);
        
        // Transfer SOL from owner to wallet PDA
        let ix = anchor_lang::solana_program::system_instruction::transfer(
            &ctx.accounts.owner.key(),
            &ctx.accounts.user_wallet_pda.key(),
            amount_to_deposit,
        );
        
        anchor_lang::solana_program::program::invoke(
            &ix,
            &[
                ctx.accounts.owner.to_account_info(),
                ctx.accounts.user_wallet_pda.to_account_info(),
            ],
        )?;

        let wallet_balance = ctx.accounts.user_wallet_pda.to_account_info().lamports();
        
        emit!(Deposit {
            sender: ctx.accounts.owner.key(),
            amount: amount_to_deposit,
            balance: wallet_balance,
        });

        Ok(())
    }

    pub fn create_transaction(
        ctx: Context<CreateTransactionCtx>, 
        transaction_seed: String, 
        transaction_lamports_amount: u64
    ) -> Result<()> {
        require!(transaction_lamports_amount > 0, WalletError::InvalidAmount);

        let transaction = &mut ctx.accounts.transaction_pda;
        transaction.receiver = ctx.accounts.receiver.key();
        transaction.amount_in_lamports = transaction_lamports_amount;
        transaction.executed = false;

        emit!(SubmitTransaction {
            sender: ctx.accounts.owner.key(),
            receiver: ctx.accounts.receiver.key(),
            amount: transaction_lamports_amount,
            transaction_seed: transaction_seed,
        });

        Ok(())
    }

    pub fn execute_transaction(
        ctx: Context<ExecuteTransactionCtx>, 
        _transaction_seed: String
    ) -> Result<()> {
        let transaction = &mut ctx.accounts.transaction_pda;
        
        require!(!transaction.executed, WalletError::TransactionAlreadyExecuted);
        
        let wallet_balance = ctx.accounts.user_wallet_pda.to_account_info().lamports();
        require!(
            wallet_balance >= transaction.amount_in_lamports,
            WalletError::InsufficientFunds
        );

        // Transfer SOL from wallet PDA to receiver
        **ctx.accounts.user_wallet_pda.to_account_info().try_borrow_mut_lamports()? -= transaction.amount_in_lamports;
        **ctx.accounts.receiver.to_account_info().try_borrow_mut_lamports()? += transaction.amount_in_lamports;

        transaction.executed = true;

        emit!(ExecuteTransaction {
            sender: ctx.accounts.owner.key(),
            receiver: ctx.accounts.receiver.key(),
            amount: transaction.amount_in_lamports,
        });

        Ok(())
    }

    pub fn withdraw(ctx: Context<DepositOrWithdrawCtx>, amount_to_withdraw: u64) -> Result<()> {
        require!(amount_to_withdraw > 0, WalletError::InvalidAmount);
        
        let wallet_balance = ctx.accounts.user_wallet_pda.to_account_info().lamports();
        require!(
            wallet_balance >= amount_to_withdraw,
            WalletError::InsufficientFunds
        );

        // Transfer SOL from wallet PDA to owner
        **ctx.accounts.user_wallet_pda.to_account_info().try_borrow_mut_lamports()? -= amount_to_withdraw;
        **ctx.accounts.owner.to_account_info().try_borrow_mut_lamports()? += amount_to_withdraw;

        let updated_balance = ctx.accounts.user_wallet_pda.to_account_info().lamports();

        emit!(Withdraw {
            sender: ctx.accounts.owner.key(),
            amount: amount_to_withdraw,
            balance: updated_balance,
        });

        Ok(())
    }
}

// Context structs
#[derive(Accounts)]
pub struct DepositOrWithdrawCtx<'info> {
    #[account(mut)]
    pub owner: Signer<'info>,
    
    #[account(
        init_if_needed,
        space = 8 + 0, // Discriminator + empty struct
        seeds = [b"wallet", owner.key().as_ref()],
        bump,
        payer = owner
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
        seeds = [b"wallet", owner.key().as_ref()],
        bump
    )]
    pub user_wallet_pda: Account<'info, UserWallet>,
    
    #[account(
        init,
        space = 8 + 32 + 8 + 1, // Discriminator + Pubkey + u64 + bool
        seeds = [transaction_seed.as_ref(), user_wallet_pda.key().as_ref()],
        bump,
        payer = owner
    )]
    pub transaction_pda: Account<'info, UserTransaction>,
    
    /// CHECK: This account is used as a reference for the receiver
    pub receiver: AccountInfo<'info>,
    
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
        bump
    )]
    pub user_wallet_pda: Account<'info, UserWallet>,
    
    #[account(
        mut,
        seeds = [transaction_seed.as_ref(), user_wallet_pda.key().as_ref()],
        bump,
        close = owner
    )]
    pub transaction_pda: Account<'info, UserTransaction>,
    
    /// CHECK: This account receives the transferred funds
    #[account(mut)]
    pub receiver: AccountInfo<'info>,
    
    pub system_program: Program<'info, System>,
}

// Account structs
#[account]
pub struct UserWallet {
    // Empty struct - space holder only
}

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
pub struct SubmitTransaction {
    pub sender: Pubkey,
    pub receiver: Pubkey,
    pub amount: u64,
    pub transaction_seed: String,
}

#[event]
pub struct ExecuteTransaction {
    pub sender: Pubkey,
    pub receiver: Pubkey,
    pub amount: u64,
}

#[event]
pub struct Withdraw {
    pub sender: Pubkey,
    pub amount: u64,
    pub balance: u64,
}

// Error codes
#[error_code]
pub enum WalletError {
    #[msg("Invalid amount: must be greater than 0")]
    InvalidAmount,
    #[msg("Insufficient funds in wallet")]
    InsufficientFunds,
    #[msg("Transaction has already been executed")]
    TransactionAlreadyExecuted,
}