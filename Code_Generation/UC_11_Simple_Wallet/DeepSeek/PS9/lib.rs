use anchor_lang::prelude::*;
use anchor_lang::system_program;

declare_id!("4au42QSRTuM1c1VoZzbCUm5KhefFLREtaPzVPP8QjmwX");

#[program]
pub mod simple_wallet {
    use super::*;

    // Deposit funds into the wallet PDA
    pub fn deposit(ctx: Context<DepositOrWithdrawCtx>, amount_to_deposit: u64) -> Result<()> {
        let owner = &ctx.accounts.owner;
        let user_wallet_pda = &ctx.accounts.user_wallet_pda;
        
        // Transfer funds from owner to wallet PDA
        let transfer_instruction = system_program::Transfer {
            from: owner.to_account_info(),
            to: user_wallet_pda.to_account_info(),
        };
        
        system_program::transfer(
            CpiContext::new(
                ctx.accounts.system_program.to_account_info(),
                transfer_instruction,
            ),
            amount_to_deposit,
        )?;

        // Get current balance after deposit
        let current_balance = user_wallet_pda.lamports();

        // Emit deposit event
        emit!(DepositEvent {
            sender: owner.key(),
            amount: amount_to_deposit,
            balance: current_balance,
        });

        Ok(())
    }

    // Create a new transaction
    // Create a new transaction
    pub fn create_transaction(
        ctx: Context<CreateTransactionCtx>,
        _transaction_seed: String,
        transaction_lamports_amount: u64,
    ) -> Result<()> {
        let owner = &ctx.accounts.owner;
        let user_wallet_pda = &ctx.accounts.user_wallet_pda;
        let transaction_pda = &mut ctx.accounts.transaction_pda;
        let receiver = &ctx.accounts.receiver;

        // Don't check balance during creation - only during execution
        // This allows creating transactions for future execution when funds will be available

        // Initialize transaction account
        transaction_pda.receiver = receiver.key();
        transaction_pda.amount_in_lamports = transaction_lamports_amount;
        transaction_pda.executed = false;

        // Emit transaction creation event
        emit!(SubmitTransactionEvent {
            owner: owner.key(),
            transaction: transaction_pda.key(),
            receiver: receiver.key(),
            amount: transaction_lamports_amount,
        });

        Ok(())
    }

    // Execute a pending transaction
    pub fn execute_transaction(
        ctx: Context<ExecuteTransactionCtx>,
        _transaction_seed: String,
    ) -> Result<()> {
        let owner = &ctx.accounts.owner;
        let user_wallet_pda = &ctx.accounts.user_wallet_pda;
        let transaction_pda = &mut ctx.accounts.transaction_pda;
        let receiver = &ctx.accounts.receiver;

        // Validate transaction hasn't been executed
        require!(!transaction_pda.executed, ErrorCode::TransactionAlreadyExecuted);
        
        // Validate receiver matches
        require!(transaction_pda.receiver == receiver.key(), ErrorCode::InvalidReceiver);

        // Check if wallet has sufficient funds
        let wallet_balance = user_wallet_pda.lamports();
        require!(
            wallet_balance >= transaction_pda.amount_in_lamports,
            ErrorCode::InsufficientBalance
        );

        // Get the bump seed for the wallet PDA
        let (_wallet_pda, bump) = Pubkey::find_program_address(
            &[b"wallet", owner.key.as_ref()],
            ctx.program_id
        );

        // Create seeds for PDA signing
        let wallet_seeds = &[b"wallet", owner.key.as_ref(), &[bump]];
        let signer_seeds = &[&wallet_seeds[..]];

        // Transfer funds from wallet PDA to receiver using CPI with PDA signer
        let cpi_context = CpiContext::new_with_signer(
            ctx.accounts.system_program.to_account_info(),
            system_program::Transfer {
                from: user_wallet_pda.to_account_info(),
                to: receiver.to_account_info(),
            },
            signer_seeds,
        );
        
        system_program::transfer(cpi_context, transaction_pda.amount_in_lamports)?;

        // Mark as executed
        transaction_pda.executed = true;

        // Emit execution event
        emit!(ExecuteTransactionEvent {
            owner: owner.key(),
            transaction: transaction_pda.key(),
            receiver: receiver.key(),
            amount: transaction_pda.amount_in_lamports,
        });

        Ok(())
    }

    // Withdraw funds from the wallet PDA
    pub fn withdraw(ctx: Context<DepositOrWithdrawCtx>, amount_to_withdraw: u64) -> Result<()> {
        let owner = &ctx.accounts.owner;
        let user_wallet_pda = &ctx.accounts.user_wallet_pda;

        // Validate sufficient balance
        let current_balance = user_wallet_pda.lamports();
        require!(
            current_balance >= amount_to_withdraw,
            ErrorCode::InsufficientBalance
        );

        // Get the bump seed for the wallet PDA
        let (_wallet_pda, bump) = Pubkey::find_program_address(
            &[b"wallet", owner.key.as_ref()],
            ctx.program_id
        );

        // Create seeds for PDA signing
        let wallet_seeds = &[b"wallet", owner.key.as_ref(), &[bump]];
        let signer_seeds = &[&wallet_seeds[..]];

        // Transfer funds from wallet PDA to owner using CPI with PDA signer
        let transfer_instruction = system_program::Transfer {
            from: user_wallet_pda.to_account_info(),
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

        // Get balance after withdrawal
        let new_balance = current_balance - amount_to_withdraw;

        // Emit withdraw event
        emit!(WithdrawEvent {
            sender: owner.key(),
            amount: amount_to_withdraw,
            balance: new_balance,
        });

        Ok(())
    }
}

// Account Contexts - Use AccountInfo for wallet to avoid initialization requirement
#[derive(Accounts)]
pub struct DepositOrWithdrawCtx<'info> {
    #[account(mut)]
    pub owner: Signer<'info>,
    
    /// CHECK: Wallet PDA that holds funds (doesn't need data initialization)
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
    #[account(mut)]
    pub owner: Signer<'info>,
    
    /// CHECK: Wallet PDA that holds funds
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
    
    /// CHECK: Receiver account validation happens during execution
    pub receiver: AccountInfo<'info>,
    
    pub system_program: Program<'info, System>,
}

#[derive(Accounts)]
#[instruction(transaction_seed: String)]
pub struct ExecuteTransactionCtx<'info> {
    #[account(mut)]
    pub owner: Signer<'info>,
    
    /// CHECK: Wallet PDA that holds funds
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
    
    /// CHECK: Receiver account validated against transaction data
    #[account(mut)]
    pub receiver: AccountInfo<'info>,
    
    pub system_program: Program<'info, System>,
}

// State Structures - Only UserTransaction needs data storage
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
    pub owner: Pubkey,
    pub transaction: Pubkey,
    pub receiver: Pubkey,
    pub amount: u64,
}

#[event]
pub struct ExecuteTransactionEvent {
    pub owner: Pubkey,
    pub transaction: Pubkey,
    pub receiver: Pubkey,
    pub amount: u64,
}

#[event]
pub struct WithdrawEvent {
    pub sender: Pubkey,
    pub amount: u64,
    pub balance: u64,
}

// Error Codes - Updated to match test expectations
#[error_code]
pub enum ErrorCode {
    #[msg("Transaction has already been executed")]
    TransactionAlreadyExecuted,  // This will be 6000
    #[msg("Insufficient balance in wallet")]
    InsufficientBalance,         // This will be 6001
    #[msg("Receiver account does not match transaction record")]
    InvalidReceiver,
    #[msg("Fund transfer failed")]
    TransferFailed,
}