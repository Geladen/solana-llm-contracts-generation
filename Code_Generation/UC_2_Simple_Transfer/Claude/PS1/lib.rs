#![allow(unexpected_cfgs)]
use anchor_lang::prelude::*;
use anchor_lang::system_program::{transfer, Transfer};

declare_id!("FcKTHyfuGjwjztkgQginLKsZGUsDachGbMNZFG9JXQLo");


#[program]
pub mod transfer_contract {
    use super::*;

    /// Deposits funds into the contract for a specific recipient
    /// Only the sender can call this function
    pub fn deposit(ctx: Context<DepositCtx>, amount_to_deposit: u64) -> Result<()> {
        // Validate amount is not zero
        require!(amount_to_deposit > 0, TransferError::InvalidAmount);

        let sender = &ctx.accounts.sender;
        let recipient_key = ctx.accounts.recipient.key();
        let balance_holder_pda = &mut ctx.accounts.balance_holder_pda;

        // Initialize PDA state (this is a new PDA since we're using init)
        balance_holder_pda.sender = sender.key();
        balance_holder_pda.recipient = recipient_key;
        balance_holder_pda.amount = amount_to_deposit;

        // Transfer lamports from sender to PDA
        let transfer_instruction = Transfer {
            from: sender.to_account_info(),
            to: balance_holder_pda.to_account_info(),
        };

        let cpi_ctx = CpiContext::new(
            ctx.accounts.system_program.to_account_info(),
            transfer_instruction,
        );

        transfer(cpi_ctx, amount_to_deposit)?;

        msg!("Deposited {} lamports for recipient {}", amount_to_deposit, recipient_key);
        
        Ok(())
    }

    /// Adds more funds to an existing PDA
    /// Only the sender can call this function
    pub fn add_funds(ctx: Context<AddFundsCtx>, amount_to_add: u64) -> Result<()> {
        // Validate amount is not zero
        require!(amount_to_add > 0, TransferError::InvalidAmount);

        let sender = &ctx.accounts.sender;
        let balance_holder_pda = &mut ctx.accounts.balance_holder_pda;

        // Validate that the sender matches the PDA's stored sender
        require!(
            balance_holder_pda.sender == sender.key(),
            TransferError::InvalidSender
        );

        // Update amount
        balance_holder_pda.amount += amount_to_add;

        // Transfer lamports from sender to PDA
        let transfer_instruction = Transfer {
            from: sender.to_account_info(),
            to: balance_holder_pda.to_account_info(),
        };

        let cpi_ctx = CpiContext::new(
            ctx.accounts.system_program.to_account_info(),
            transfer_instruction,
        );

        transfer(cpi_ctx, amount_to_add)?;

        msg!("Added {} lamports. Total balance: {} lamports", amount_to_add, balance_holder_pda.amount);
        
        Ok(())
    }

    /// Withdraws funds from the contract
    /// Only the designated recipient can call this function
    pub fn withdraw(ctx: Context<WithdrawCtx>, amount_to_withdraw: u64) -> Result<()> {
        // Validate amount is not zero
        require!(amount_to_withdraw > 0, TransferError::InvalidAmount);

        let balance_holder_pda = &mut ctx.accounts.balance_holder_pda;
        let recipient = &ctx.accounts.recipient;
        let sender_key = ctx.accounts.sender.key();

        // Validate that the PDA contains the correct sender-recipient relationship
        require!(
            balance_holder_pda.sender == sender_key,
            TransferError::InvalidSender
        );
        require!(
            balance_holder_pda.recipient == recipient.key(),
            TransferError::InvalidRecipient
        );

        // Validate sufficient balance
        require!(
            balance_holder_pda.amount >= amount_to_withdraw,
            TransferError::InsufficientBalance
        );

        // Update balance first
        balance_holder_pda.amount -= amount_to_withdraw;

        // Check if this withdrawal empties the account
        if balance_holder_pda.amount == 0 {
            // Close the account - transfer all remaining lamports
            let pda_account_info = balance_holder_pda.to_account_info();
            let pda_lamports = pda_account_info.lamports();
            
            // Transfer withdrawal amount to recipient
            **pda_account_info.try_borrow_mut_lamports()? -= amount_to_withdraw;
            **recipient.to_account_info().try_borrow_mut_lamports()? += amount_to_withdraw;
            
            // Transfer remaining rent to sender (account closure)
            let remaining_lamports = pda_account_info.lamports();
            if remaining_lamports > 0 {
                **pda_account_info.try_borrow_mut_lamports()? -= remaining_lamports;
                **ctx.accounts.sender.to_account_info().try_borrow_mut_lamports()? += remaining_lamports;
            }
            
            msg!("Account closed. Withdrew {} lamports to recipient, returned {} lamports rent to sender", 
                 amount_to_withdraw, remaining_lamports);
        } else {
            // Normal withdrawal - just transfer the requested amount
            **balance_holder_pda.to_account_info().try_borrow_mut_lamports()? -= amount_to_withdraw;
            **recipient.to_account_info().try_borrow_mut_lamports()? += amount_to_withdraw;

            msg!("Withdrew {} lamports. Remaining balance: {} lamports", 
                 amount_to_withdraw, balance_holder_pda.amount);
        }

        Ok(())
    }

    /// Closes the PDA account when balance reaches zero
    /// Returns remaining lamports to sender
    pub fn close_account(ctx: Context<CloseAccountCtx>) -> Result<()> {
        let balance_holder_pda = &ctx.accounts.balance_holder_pda;
        
        // Only allow closing when balance is zero
        require!(
            balance_holder_pda.amount == 0,
            TransferError::BalanceNotZero
        );

        // Validate that the PDA contains the correct sender-recipient relationship
        require!(
            balance_holder_pda.sender == ctx.accounts.sender.key(),
            TransferError::InvalidSender
        );
        require!(
            balance_holder_pda.recipient == ctx.accounts.recipient.key(),
            TransferError::InvalidRecipient
        );

        msg!("Closing PDA account and returning remaining lamports to sender");
        
        Ok(())
    }
}

#[derive(Accounts)]
pub struct DepositCtx<'info> {
    #[account(mut)]
    pub sender: Signer<'info>,
    
    /// CHECK: Used only for PDA derivation, not modified
    pub recipient: AccountInfo<'info>,
    
    #[account(
        init,
        payer = sender,
        space = 8 + 32 + 32 + 8, // discriminator + sender + recipient + amount
        seeds = [recipient.key().as_ref(), sender.key().as_ref()],
        bump
    )]
    pub balance_holder_pda: Account<'info, BalanceHolderPDA>,
    
    pub system_program: Program<'info, System>,
}

#[derive(Accounts)]
pub struct AddFundsCtx<'info> {
    #[account(mut)]
    pub sender: Signer<'info>,
    
    /// CHECK: Used only for PDA derivation, not modified
    pub recipient: AccountInfo<'info>,
    
    #[account(
        mut,
        seeds = [recipient.key().as_ref(), sender.key().as_ref()],
        bump
    )]
    pub balance_holder_pda: Account<'info, BalanceHolderPDA>,
    
    pub system_program: Program<'info, System>,
}

#[derive(Accounts)]
pub struct WithdrawCtx<'info> {
    #[account(mut)]
    pub recipient: Signer<'info>,
    
    /// CHECK: Used for PDA derivation and validation, receives rent refund on closure
    #[account(mut)]
    pub sender: AccountInfo<'info>,
    
    #[account(
        mut,
        seeds = [recipient.key().as_ref(), sender.key().as_ref()],
        bump
    )]
    pub balance_holder_pda: Account<'info, BalanceHolderPDA>,
    
    pub rent: Sysvar<'info, Rent>,
}

#[derive(Accounts)]
pub struct CloseAccountCtx<'info> {
    /// CHECK: Used for validation and receives remaining lamports
    #[account(mut)]
    pub sender: AccountInfo<'info>,
    
    /// CHECK: Used for PDA derivation and validation only
    pub recipient: AccountInfo<'info>,
    
    #[account(
        mut,
        seeds = [recipient.key().as_ref(), sender.key().as_ref()],
        bump,
        close = sender
    )]
    pub balance_holder_pda: Account<'info, BalanceHolderPDA>,
}

#[account]
pub struct BalanceHolderPDA {
    pub sender: Pubkey,      // 32 bytes
    pub recipient: Pubkey,   // 32 bytes
    pub amount: u64,         // 8 bytes
}

#[error_code]
pub enum TransferError {
    #[msg("Amount must be greater than zero")]
    InvalidAmount,
    #[msg("Invalid sender for this PDA")]
    InvalidSender,
    #[msg("Invalid recipient for this PDA")]
    InvalidRecipient,
    #[msg("Insufficient balance for withdrawal")]
    InsufficientBalance,
    #[msg("Balance must be zero to close account")]
    BalanceNotZero,
}