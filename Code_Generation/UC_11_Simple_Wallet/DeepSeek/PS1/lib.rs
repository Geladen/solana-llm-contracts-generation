use anchor_lang::prelude::*;
use anchor_lang::system_program;

declare_id!("8PH6p6pbLzdRrqMpVRSZ2mHzKcpiKB4Xa5bvdnfjpFpe");

#[program]
pub mod simple_wallet {
    use super::*;

    pub fn deposit(ctx: Context<DepositOrWithdrawCtx>, amount_to_deposit: u64) -> Result<()> {
        require!(amount_to_deposit > 0, WalletError::InvalidAmount);
        
        let owner = &ctx.accounts.owner;
        let user_wallet_pda = &ctx.accounts.user_wallet_pda;
        
        // Transfer funds from owner to wallet PDA
        let cpi_context = CpiContext::new(
            ctx.accounts.system_program.to_account_info(),
            system_program::Transfer {
                from: owner.to_account_info(),
                to: user_wallet_pda.to_account_info(),
            },
        );
        system_program::transfer(cpi_context, amount_to_deposit)?;
        
        // Emit deposit event
        emit!(DepositEvent {
            sender: owner.key(),
            amount: amount_to_deposit,
            balance: user_wallet_pda.get_lamports()
        });
        
        Ok(())
    }

    pub fn create_transaction(
        ctx: Context<CreateTransactionCtx>,
        transaction_seed: String,
        transaction_lamports_amount: u64,
    ) -> Result<()> {
        require!(transaction_lamports_amount > 0, WalletError::InvalidAmount);
        
        // REMOVED: Balance check during creation - only check during execution
        let transaction_pda = &mut ctx.accounts.transaction_pda;
        let receiver = &ctx.accounts.receiver;
        
        // Initialize transaction account
        transaction_pda.receiver = receiver.key();
        transaction_pda.amount_in_lamports = transaction_lamports_amount;
        transaction_pda.executed = false;
        
        // Emit transaction creation event
        emit!(SubmitTransactionEvent {
            transaction: transaction_pda.key(),
            receiver: receiver.key(),
            amount: transaction_lamports_amount,
        });
        
        Ok(())
    }

    pub fn execute_transaction(
        ctx: Context<ExecuteTransactionCtx>,
        _transaction_seed: String,
    ) -> Result<()> {
        let user_wallet_pda = &ctx.accounts.user_wallet_pda;
        let transaction_pda = &mut ctx.accounts.transaction_pda;
        let receiver = &ctx.accounts.receiver;
        
        // Validate transaction
        require!(!transaction_pda.executed, WalletError::TransactionAlreadyExecuted);
        require!(
            transaction_pda.receiver == receiver.key(),
            WalletError::InvalidReceiver
        );
        
        // MOVED: Balance check to execution phase (this is what the test expects)
        let wallet_balance = user_wallet_pda.get_lamports();
        require!(
            wallet_balance >= transaction_pda.amount_in_lamports,
            WalletError::InsufficientFunds
        );

        // Transfer funds to receiver with PDA signing
        let bump = ctx.bumps.user_wallet_pda;
        let seeds = &[
            b"wallet".as_ref(),
            ctx.accounts.owner.key.as_ref(),
            &[bump],
        ];
        let signer_seeds = &[&seeds[..]];
        
        let cpi_context = CpiContext::new_with_signer(
            ctx.accounts.system_program.to_account_info(),
            system_program::Transfer {
                from: user_wallet_pda.to_account_info(),
                to: receiver.to_account_info(),
            },
            signer_seeds,
        );
        system_program::transfer(cpi_context, transaction_pda.amount_in_lamports)?;
        
        // Mark transaction as executed
        transaction_pda.executed = true;
        
        // Get the transaction key before moving the account info
        let transaction_key = transaction_pda.key();
        
        // Close the transaction account and send rent to the owner
        let transaction_account_info = transaction_pda.to_account_info();
        let rent_balance = transaction_account_info.get_lamports();
        
        if rent_balance > 0 {
            **transaction_account_info.try_borrow_mut_lamports()? = 0;
            **ctx.accounts.owner.to_account_info().try_borrow_mut_lamports()? += rent_balance;
        }
        
        // Emit execution event
        emit!(ExecuteTransactionEvent {
            transaction: transaction_key,
            receiver: receiver.key(),
            amount: transaction_pda.amount_in_lamports,
        });
        
        Ok(())
    }

    pub fn withdraw(ctx: Context<DepositOrWithdrawCtx>, amount_to_withdraw: u64) -> Result<()> {
        require!(amount_to_withdraw > 0, WalletError::InvalidAmount);
        
        let owner = &ctx.accounts.owner;
        let user_wallet_pda = &ctx.accounts.user_wallet_pda;
        
        let wallet_balance = user_wallet_pda.get_lamports();
        require!(wallet_balance >= amount_to_withdraw, WalletError::InsufficientFunds);
        
        // Transfer funds from wallet PDA to owner with PDA signing
        let bump = ctx.bumps.user_wallet_pda;
        let seeds = &[
            b"wallet".as_ref(),
            owner.key.as_ref(),
            &[bump],
        ];
        let signer_seeds = &[&seeds[..]];
        
        let cpi_context = CpiContext::new_with_signer(
            ctx.accounts.system_program.to_account_info(),
            system_program::Transfer {
                from: user_wallet_pda.to_account_info(),
                to: owner.to_account_info(),
            },
            signer_seeds,
        );
        system_program::transfer(cpi_context, amount_to_withdraw)?;
        
        // Emit withdraw event
        emit!(WithdrawEvent {
            sender: owner.key(),
            amount: amount_to_withdraw,
            balance: wallet_balance - amount_to_withdraw
        });
        
        Ok(())
    }
}

#[derive(Accounts)]
pub struct DepositOrWithdrawCtx<'info> {
    #[account(mut)]
    pub owner: Signer<'info>,
    
    #[account(
        mut,
        seeds = [b"wallet".as_ref(), owner.key().as_ref()],
        bump,
    )]
    /// CHECK: This is a PDA that holds lamports, no data initialization needed
    pub user_wallet_pda: AccountInfo<'info>,
    
    pub system_program: Program<'info, System>,
}

#[derive(Accounts)]
#[instruction(transaction_seed: String)]
pub struct CreateTransactionCtx<'info> {
    #[account(mut)]
    pub owner: Signer<'info>,
    
    #[account(
        mut,
        seeds = [b"wallet".as_ref(), owner.key().as_ref()],
        bump,
    )]
    /// CHECK: This is a PDA that holds lamports, no data initialization needed
    pub user_wallet_pda: AccountInfo<'info>,
    
    #[account(
        init,
        payer = owner,
        space = 8 + UserTransaction::INIT_SPACE,
        seeds = [transaction_seed.as_bytes(), user_wallet_pda.key().as_ref()],
        bump,
    )]
    pub transaction_pda: Account<'info, UserTransaction>,
    
    /// CHECK: Receiver account for the transaction
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
        seeds = [b"wallet".as_ref(), owner.key().as_ref()],
        bump,
    )]
    /// CHECK: This is a PDA that holds lamports, no data initialization needed
    pub user_wallet_pda: AccountInfo<'info>,
    
    #[account(
        mut,
        seeds = [transaction_seed.as_bytes(), user_wallet_pda.key().as_ref()],
        bump,
    )]
    pub transaction_pda: Account<'info, UserTransaction>,
    
    /// CHECK: Receiver account for the transaction
    #[account(mut)]
    pub receiver: AccountInfo<'info>,
    
    pub system_program: Program<'info, System>,
}

#[account]
#[derive(InitSpace)]
pub struct UserTransaction {
    pub receiver: Pubkey,
    pub amount_in_lamports: u64,
    pub executed: bool,
}

#[event]
pub struct DepositEvent {
    pub sender: Pubkey,
    pub amount: u64,
    pub balance: u64,
}

#[event]
pub struct WithdrawEvent {
    pub sender: Pubkey,
    pub amount: u64,
    pub balance: u64,
}

#[event]
pub struct SubmitTransactionEvent {
    pub transaction: Pubkey,
    pub receiver: Pubkey,
    pub amount: u64,
}

#[event]
pub struct ExecuteTransactionEvent {
    pub transaction: Pubkey,
    pub receiver: Pubkey,
    pub amount: u64,
}

#[error_code]
pub enum WalletError {
    #[msg("Invalid amount: must be greater than zero")]
    InvalidAmount,
    #[msg("Insufficient funds in wallet")]
    InsufficientFunds,
    #[msg("Transaction has already been executed")]
    TransactionAlreadyExecuted,
    #[msg("Invalid receiver account")]
    InvalidReceiver,
}