use anchor_lang::prelude::*;

declare_id!("DkxQr17qg4wjYwFexk5z2x5sqX3Jvup2uq6J3KjGuXCi");

#[program]
pub mod simple_wallet {
    use super::*;

    pub fn deposit(ctx: Context<DepositOrWithdrawCtx>, amount_to_deposit: u64) -> Result<()> {
        require!(amount_to_deposit > 0, SimpleWalletError::InvalidAmount);
        
        let owner = &ctx.accounts.owner;
        let user_wallet_pda = &ctx.accounts.user_wallet_pda;
        let system_program = &ctx.accounts.system_program;

        // Transfer funds from owner to wallet PDA
        let transfer_instruction = anchor_lang::system_program::Transfer {
            from: owner.to_account_info(),
            to: user_wallet_pda.to_account_info(),
        };

        let cpi_ctx = CpiContext::new(
            system_program.to_account_info(),
            transfer_instruction,
        );

        anchor_lang::system_program::transfer(cpi_ctx, amount_to_deposit)?;

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

    pub fn create_transaction(
        ctx: Context<CreateTransactionCtx>,
        transaction_seed: String,
        transaction_lamports_amount: u64,
    ) -> Result<()> {
        require!(transaction_lamports_amount > 0, SimpleWalletError::InvalidAmount);
        require!(!transaction_seed.is_empty(), SimpleWalletError::InvalidTransactionSeed);
        
        let transaction_pda = &mut ctx.accounts.transaction_pda;
        let receiver = &ctx.accounts.receiver;

        // Initialize transaction PDA (no balance check here - check during execution)
        transaction_pda.receiver = receiver.key();
        transaction_pda.amount_in_lamports = transaction_lamports_amount;
        transaction_pda.executed = false;

        // Emit submit transaction event
        emit!(SubmitTransaction {
            transaction_pda: transaction_pda.key(),
            receiver: receiver.key(),
            amount: transaction_lamports_amount,
        });

        Ok(())
    }

    pub fn execute_transaction(
        ctx: Context<ExecuteTransactionCtx>,
        transaction_seed: String,
    ) -> Result<()> {
        require!(!transaction_seed.is_empty(), SimpleWalletError::InvalidTransactionSeed);
        
        let owner = &ctx.accounts.owner;
        let user_wallet_pda = &ctx.accounts.user_wallet_pda;
        let transaction_pda = &mut ctx.accounts.transaction_pda;
        let receiver = &ctx.accounts.receiver;
        let system_program = &ctx.accounts.system_program;

        // Verify transaction hasn't been executed
        require!(!transaction_pda.executed, SimpleWalletError::TransactionAlreadyExecuted);

        // Verify receiver matches
        require!(
            transaction_pda.receiver == receiver.key(),
            SimpleWalletError::InvalidReceiver
        );

        // Check wallet has sufficient balance
        require!(
            user_wallet_pda.to_account_info().lamports() >= transaction_pda.amount_in_lamports,
            SimpleWalletError::InsufficientFunds
        );

        let amount = transaction_pda.amount_in_lamports;

        // Transfer from wallet PDA to receiver by manually adjusting lamports
        **user_wallet_pda.to_account_info().try_borrow_mut_lamports()? -= amount;
        **receiver.to_account_info().try_borrow_mut_lamports()? += amount;

        // Mark transaction as executed
        transaction_pda.executed = true;

        // Emit execute transaction event
        emit!(ExecuteTransaction {
            transaction_pda: transaction_pda.key(),
            receiver: receiver.key(),
            amount,
        });

        Ok(())
    }

    pub fn withdraw(ctx: Context<DepositOrWithdrawCtx>, amount_to_withdraw: u64) -> Result<()> {
        require!(amount_to_withdraw > 0, SimpleWalletError::InvalidAmount);
        
        let owner = &ctx.accounts.owner;
        let user_wallet_pda = &ctx.accounts.user_wallet_pda;
        let system_program = &ctx.accounts.system_program;

        // Check wallet has sufficient balance
        require!(
            user_wallet_pda.to_account_info().lamports() >= amount_to_withdraw,
            SimpleWalletError::InsufficientFunds
        );

        // Transfer from wallet PDA to owner by manually adjusting lamports
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

// Context structs
#[derive(Accounts)]
pub struct DepositOrWithdrawCtx<'info> {
    #[account(mut)]
    pub owner: Signer<'info>,
    #[account(
        init_if_needed,
        space = 8 + std::mem::size_of::<UserWallet>(),
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
        init_if_needed,
        space = 8 + std::mem::size_of::<UserWallet>(),
        seeds = [b"wallet", owner.key().as_ref()],
        bump,
        payer = owner
    )]
    pub user_wallet_pda: Account<'info, UserWallet>,
    #[account(
        init,
        space = 8 + std::mem::size_of::<UserTransaction>(),
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
    #[account(mut)]
    /// CHECK: This account receives the transferred funds
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

#[event]
pub struct Withdraw {
    pub sender: Pubkey,
    pub amount: u64,
    pub balance: u64,
}

// Error codes
#[error_code]
pub enum SimpleWalletError {
    #[msg("Invalid amount: amount must be greater than 0")]
    InvalidAmount,
    #[msg("Insufficient funds in wallet")]
    InsufficientFunds,
    #[msg("Transaction already executed")]
    TransactionAlreadyExecuted,
    #[msg("Invalid receiver address")]
    InvalidReceiver,
    #[msg("Invalid transaction seed: seed cannot be empty")]
    InvalidTransactionSeed,
}