use anchor_lang::prelude::*;
use anchor_lang::system_program::{transfer, Transfer};

declare_id!("GYhBAfjcnssoPm7KFcPDg4JqjLqmZ4uYxmTDs45xnUE5");

#[program]
pub mod simple_wallet {
    use super::*;

    /// Deposit funds into the wallet
    /// Only the owner can deposit funds
    pub fn deposit(ctx: Context<DepositOrWithdrawCtx>, amount_to_deposit: u64) -> Result<()> {
        require!(amount_to_deposit > 0, ErrorCode::InvalidAmount);
        
        let owner = &ctx.accounts.owner;
        let user_wallet_pda = &ctx.accounts.user_wallet_pda;
        let system_program = &ctx.accounts.system_program;

        // Transfer SOL from owner to wallet PDA
        let transfer_accounts = Transfer {
            from: owner.to_account_info(),
            to: user_wallet_pda.to_account_info(),
        };

        let transfer_ctx = CpiContext::new(system_program.to_account_info(), transfer_accounts);
        transfer(transfer_ctx, amount_to_deposit)?;

        // Get current balance after deposit
        let current_balance = user_wallet_pda.to_account_info().lamports();

        // Emit deposit event
        emit!(Deposit {
            sender: owner.key(),
            amount: amount_to_deposit,
            balance: current_balance,
        });

        Ok(())
    }

    /// Create a new transaction
    /// Only the owner can create transactions
    pub fn create_transaction(
        ctx: Context<CreateTransactionCtx>,
        transaction_seed: String,
        transaction_lamports_amount: u64,
    ) -> Result<()> {
        require!(transaction_lamports_amount > 0, ErrorCode::InvalidAmount);
        require!(!transaction_seed.is_empty(), ErrorCode::InvalidTransactionSeed);
        
        let transaction_pda = &mut ctx.accounts.transaction_pda;
        let receiver = &ctx.accounts.receiver;

        // Initialize transaction data
        transaction_pda.receiver = receiver.key();
        transaction_pda.amount_in_lamports = transaction_lamports_amount;
        transaction_pda.executed = false;

        // Emit submit transaction event
        emit!(SubmitTransaction {
            owner: ctx.accounts.owner.key(),
            receiver: receiver.key(),
            amount: transaction_lamports_amount,
            transaction_seed: transaction_seed.clone(),
        });

        Ok(())
    }

    /// Execute a pending transaction
    /// Only the owner can execute transactions
    pub fn execute_transaction(
        ctx: Context<ExecuteTransactionCtx>,
        transaction_seed: String,
    ) -> Result<()> {
        let user_wallet_pda = &ctx.accounts.user_wallet_pda;
        let transaction_pda = &ctx.accounts.transaction_pda;
        let receiver = &ctx.accounts.receiver;

        // Verify transaction hasn't been executed
        require!(!transaction_pda.executed, ErrorCode::TransactionAlreadyExecuted);

        // Verify receiver matches transaction
        require!(
            transaction_pda.receiver == receiver.key(),
            ErrorCode::InvalidReceiver
        );

        // Check wallet has sufficient balance (account for rent exemption)
        let wallet_balance = user_wallet_pda.to_account_info().lamports();
        let rent = Rent::get()?;
        let min_rent_balance = rent.minimum_balance(user_wallet_pda.to_account_info().data_len());
        let available_balance = wallet_balance.saturating_sub(min_rent_balance);
        
        require!(
            available_balance >= transaction_pda.amount_in_lamports,
            ErrorCode::InsufficientBalance
        );

        // Transfer lamports directly from wallet PDA to receiver
        // We use raw lamport transfer since the wallet PDA contains data
        **user_wallet_pda.to_account_info().try_borrow_mut_lamports()? -= transaction_pda.amount_in_lamports;
        **receiver.to_account_info().try_borrow_mut_lamports()? += transaction_pda.amount_in_lamports;

        // Emit execute transaction event
        emit!(ExecuteTransaction {
            owner: ctx.accounts.owner.key(),
            receiver: receiver.key(),
            amount: transaction_pda.amount_in_lamports,
            transaction_seed: transaction_seed.clone(),
        });

        Ok(())
    }

    /// Withdraw funds from the wallet
    /// Only the owner can withdraw funds
    pub fn withdraw(ctx: Context<DepositOrWithdrawCtx>, amount_to_withdraw: u64) -> Result<()> {
        require!(amount_to_withdraw > 0, ErrorCode::InvalidAmount);

        let owner = &ctx.accounts.owner;
        let user_wallet_pda = &ctx.accounts.user_wallet_pda;

        // Check wallet has sufficient balance (account for rent exemption)
        let wallet_balance = user_wallet_pda.to_account_info().lamports();
        let rent = Rent::get()?;
        let min_rent_balance = rent.minimum_balance(user_wallet_pda.to_account_info().data_len());
        let available_balance = wallet_balance.saturating_sub(min_rent_balance);
        
        require!(available_balance >= amount_to_withdraw, ErrorCode::InsufficientBalance);

        // Transfer lamports directly from wallet PDA to owner
        // We use raw lamport transfer since the wallet PDA contains data
        **user_wallet_pda.to_account_info().try_borrow_mut_lamports()? -= amount_to_withdraw;
        **owner.to_account_info().try_borrow_mut_lamports()? += amount_to_withdraw;

        // Get current balance after withdrawal
        let current_balance = user_wallet_pda.to_account_info().lamports();

        // Emit withdraw event
        emit!(Withdraw {
            sender: owner.key(),
            amount: amount_to_withdraw,
            balance: current_balance,
        });

        Ok(())
    }
}

// Context structs for different operations
#[derive(Accounts)]
pub struct DepositOrWithdrawCtx<'info> {
    #[account(mut)]
    pub owner: Signer<'info>,
    
    #[account(
        init_if_needed,
        payer = owner,
        space = 8 + std::mem::size_of::<UserWallet>(),
        seeds = [b"wallet", owner.key().as_ref()],
        bump
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
        payer = owner,
        space = 8 + std::mem::size_of::<UserTransaction>(),
        seeds = [transaction_seed.as_ref(), user_wallet_pda.key().as_ref()],
        bump
    )]
    pub transaction_pda: Account<'info, UserTransaction>,
    
    /// CHECK: This account is validated in the instruction logic
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
    
    /// CHECK: This account is validated against the transaction data
    #[account(mut)]
    pub receiver: AccountInfo<'info>,
    
    pub system_program: Program<'info, System>,
}

// Account state structures
#[account]
pub struct UserWallet {
    // Empty struct - serves as a space holder for the PDA
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
    pub owner: Pubkey,
    pub receiver: Pubkey,
    pub amount: u64,
    pub transaction_seed: String,
}

#[event]
pub struct ExecuteTransaction {
    pub owner: Pubkey,
    pub receiver: Pubkey,
    pub amount: u64,
    pub transaction_seed: String,
}

#[event]
pub struct Withdraw {
    pub sender: Pubkey,
    pub amount: u64,
    pub balance: u64,
}

// Custom error codes
#[error_code]
pub enum ErrorCode {
    #[msg("Invalid amount: must be greater than 0")]
    InvalidAmount,
    #[msg("Insufficient balance in wallet")]
    InsufficientBalance,
    #[msg("Transaction has already been executed")]
    TransactionAlreadyExecuted,
    #[msg("Invalid receiver for this transaction")]
    InvalidReceiver,
    #[msg("Transaction seed cannot be empty")]
    InvalidTransactionSeed,
}