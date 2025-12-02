use anchor_lang::prelude::*;
use anchor_lang::system_program;

declare_id!("AHHrBJPCm9UvX2RErSt241LMRWf3YnwraEzXDBqEfW3u");

#[program]
pub mod simple_wallet {
    use super::*;

    // Deposit funds into the wallet PDA
    pub fn deposit(ctx: Context<DepositOrWithdrawCtx>, amount_to_deposit: u64) -> Result<()> {
        require!(amount_to_deposit > 0, SimpleWalletError::InvalidAmount);
        
        let owner = &ctx.accounts.owner;
        let wallet_pda = &ctx.accounts.user_wallet_pda;
        
        // Get current balance using direct lamports access
        let current_balance = **wallet_pda.to_account_info().lamports.borrow();
        
        // Transfer funds from owner to wallet PDA
        let transfer_instruction = system_program::Transfer {
            from: owner.to_account_info(),
            to: wallet_pda.to_account_info(),
        };
        
        system_program::transfer(
            CpiContext::new(
                ctx.accounts.system_program.to_account_info(),
                transfer_instruction,
            ),
            amount_to_deposit,
        )?;

        // Emit deposit event
        emit!(DepositEvent {
            sender: owner.key(),
            amount: amount_to_deposit,
            balance: current_balance + amount_to_deposit,
        });

        Ok(())
    }

    // Create a new transaction
    pub fn create_transaction(
        ctx: Context<CreateTransactionCtx>,
        transaction_seed: String,
        transaction_lamports_amount: u64,
    ) -> Result<()> {
        require!(transaction_lamports_amount > 0, SimpleWalletError::InvalidAmount);
        require!(!transaction_seed.is_empty(), SimpleWalletError::InvalidSeed);
        
        let wallet_pda = &ctx.accounts.user_wallet_pda;
        
        // Note: We DON'T check balance here because funds are only reserved, not transferred yet
        // The actual balance check happens in execute_transaction

        let transaction_pda = &mut ctx.accounts.transaction_pda;
        let receiver = ctx.accounts.receiver.key();
        
        // Initialize transaction data
        transaction_pda.receiver = receiver;
        transaction_pda.amount_in_lamports = transaction_lamports_amount;
        transaction_pda.executed = false;

        // Emit transaction creation event
        emit!(SubmitTransactionEvent {
            transaction_pda: transaction_pda.key(),
            receiver,
            amount: transaction_lamports_amount,
        });

        Ok(())
    }

    // Execute a pending transaction
    pub fn execute_transaction(
        ctx: Context<ExecuteTransactionCtx>,
        _transaction_seed: String,
    ) -> Result<()> {
        let wallet_pda = &ctx.accounts.user_wallet_pda;
        let transaction_pda = &mut ctx.accounts.transaction_pda;
        
        // Validate transaction state
        require!(!transaction_pda.executed, SimpleWalletError::TransactionAlreadyExecuted);
        require!(
            transaction_pda.amount_in_lamports > 0,
            SimpleWalletError::InvalidAmount
        );

        let wallet_balance = **wallet_pda.to_account_info().lamports.borrow();
        require!(
            wallet_balance >= transaction_pda.amount_in_lamports,
            SimpleWalletError::InsufficientFunds
        );

        // Validate receiver matches transaction data
        require!(
            transaction_pda.receiver == ctx.accounts.receiver.key(),
            SimpleWalletError::InvalidReceiver
        );

        // Transfer funds from wallet PDA to receiver
        let bump = ctx.bumps.user_wallet_pda;
        let seeds = &[
            b"wallet",
            ctx.accounts.owner.key.as_ref(),
            &[bump],
        ];
        let signer_seeds = &[&seeds[..]];
        
        let transfer_instruction = system_program::Transfer {
            from: wallet_pda.to_account_info(),
            to: ctx.accounts.receiver.to_account_info(),
        };
        
        system_program::transfer(
            CpiContext::new_with_signer(
                ctx.accounts.system_program.to_account_info(),
                transfer_instruction,
                signer_seeds,
            ),
            transaction_pda.amount_in_lamports,
        )?;

        // Mark transaction as executed
        transaction_pda.executed = true;

        // Emit execution event
        emit!(ExecuteTransactionEvent {
            transaction_pda: transaction_pda.key(),
            receiver: transaction_pda.receiver,
            amount: transaction_pda.amount_in_lamports,
        });

        Ok(())
    }

    // Withdraw funds from wallet PDA
    pub fn withdraw(ctx: Context<DepositOrWithdrawCtx>, amount_to_withdraw: u64) -> Result<()> {
        require!(amount_to_withdraw > 0, SimpleWalletError::InvalidAmount);
        
        let owner = &ctx.accounts.owner;
        let wallet_pda = &ctx.accounts.user_wallet_pda;
        
        let wallet_balance = **wallet_pda.to_account_info().lamports.borrow();
        require!(
            wallet_balance >= amount_to_withdraw,
            SimpleWalletError::InsufficientFunds
        );

        // Transfer funds from wallet PDA to owner
        let bump = ctx.bumps.user_wallet_pda;
        let seeds = &[
            b"wallet",
            owner.key.as_ref(),
            &[bump],
        ];
        let signer_seeds = &[&seeds[..]];
        
        let transfer_instruction = system_program::Transfer {
            from: wallet_pda.to_account_info(),
            to: owner.to_account_info(),
        };
        
        system_program::transfer(
            CpiContext::new_with_signer(
                ctx.accounts.system_program.to_account_info(),
                transfer_instruction,
                signer_seeds,
            ),
            amount_to_withdraw,
        )?;

        // Emit withdraw event
        emit!(WithdrawEvent {
            sender: owner.key(),
            amount: amount_to_withdraw,
            balance: wallet_balance - amount_to_withdraw,
        });

        Ok(())
    }
}

// Context Structures

#[derive(Accounts)]
pub struct DepositOrWithdrawCtx<'info> {
    /// CHECK: Validated by signer constraint
    #[account(mut, signer)]
    pub owner: AccountInfo<'info>,
    
    /// CHECK: PDA validated by seeds and bump
    #[account(
        mut,
        seeds = [b"wallet", owner.key().as_ref()],
        bump
    )]
    pub user_wallet_pda: AccountInfo<'info>,
    
    pub system_program: Program<'info, System>,
}

#[derive(Accounts)]
#[instruction(transaction_seed: String)]
pub struct CreateTransactionCtx<'info> {
    /// CHECK: Validated by signer constraint
    #[account(mut, signer)]
    pub owner: AccountInfo<'info>,
    
    /// CHECK: PDA validated by seeds and bump
    #[account(
        mut,
        seeds = [b"wallet", owner.key().as_ref()],
        bump
    )]
    pub user_wallet_pda: AccountInfo<'info>,
    
    #[account(
        init,
        payer = owner,
        space = UserTransaction::SIZE,
        seeds = [transaction_seed.as_bytes(), user_wallet_pda.key().as_ref()],
        bump
    )]
    pub transaction_pda: Account<'info, UserTransaction>,
    
    /// CHECK: Receiver account validation is done in the instruction
    pub receiver: AccountInfo<'info>,
    
    pub system_program: Program<'info, System>,
}

#[derive(Accounts)]
#[instruction(transaction_seed: String)]
pub struct ExecuteTransactionCtx<'info> {
    /// CHECK: Validated by signer constraint
    #[account(mut, signer)]
    pub owner: AccountInfo<'info>,
    
    /// CHECK: PDA validated by seeds and bump
    #[account(
        mut,
        seeds = [b"wallet", owner.key().as_ref()],
        bump
    )]
    pub user_wallet_pda: AccountInfo<'info>,
    
    #[account(
        mut,
        seeds = [transaction_seed.as_bytes(), user_wallet_pda.key().as_ref()],
        bump,
        close = owner
    )]
    pub transaction_pda: Account<'info, UserTransaction>,
    
    /// CHECK: Receiver account is validated against transaction data in instruction
    #[account(mut)]
    pub receiver: AccountInfo<'info>,
    
    pub system_program: Program<'info, System>,
}

// State Structures

#[account]
pub struct UserWallet {}

#[account]
pub struct UserTransaction {
    pub receiver: Pubkey,
    pub amount_in_lamports: u64,
    pub executed: bool,
}

impl UserTransaction {
    pub const SIZE: usize = 8 + 32 + 8 + 1; // discriminator + receiver + amount + executed
}

// Events

#[event]
pub struct DepositEvent {
    pub sender: Pubkey,
    pub amount: u64,
    pub balance: u64,
}

#[event]
pub struct SubmitTransactionEvent {
    pub transaction_pda: Pubkey,
    pub receiver: Pubkey,
    pub amount: u64,
}

#[event]
pub struct ExecuteTransactionEvent {
    pub transaction_pda: Pubkey,
    pub receiver: Pubkey,
    pub amount: u64,
}

#[event]
pub struct WithdrawEvent {
    pub sender: Pubkey,
    pub amount: u64,
    pub balance: u64,
}

// Error Codes

#[error_code]
pub enum SimpleWalletError {
    #[msg("Invalid amount: must be greater than zero")]
    InvalidAmount,
    #[msg("Insufficient funds in wallet")]
    InsufficientFunds,
    #[msg("Transaction has already been executed")]
    TransactionAlreadyExecuted,
    #[msg("Invalid transaction seed")]
    InvalidSeed,
    #[msg("Arithmetic overflow occurred")]
    Overflow,
    #[msg("Receiver account does not match transaction data")]
    InvalidReceiver,
}