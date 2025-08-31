#![allow(unexpected_cfgs)]
use anchor_lang::prelude::*;
use anchor_lang::system_program::{transfer, Transfer};

declare_id!("FnyoFnQKzbfZsfnKEHB8NsF9o1eRC8nUhD3Km3d5fjye");

#[program]
pub mod transfer_contract {
    use super::*;

    pub fn deposit(ctx: Context<Deposit>, amount_to_deposit: u64) -> Result<()> {
        require!(amount_to_deposit > 0, TransferError::InvalidAmount);

        let balance_holder = &mut ctx.accounts.balance_holder_pda;
        
        // Check if this is a new account by examining the amount field
        if balance_holder.amount == 0 {
            // Initialize account with sender and recipient data
            balance_holder.sender = ctx.accounts.sender.key();
            balance_holder.recipient = ctx.accounts.recipient.key();
            balance_holder.amount = amount_to_deposit;
        } else {
            // Update existing account
            balance_holder.amount = balance_holder
                .amount
                .checked_add(amount_to_deposit)
                .ok_or(TransferError::Overflow)?;
        }

        // Transfer lamports
        let cpi_context = CpiContext::new(
            ctx.accounts.system_program.to_account_info(),
            Transfer {
                from: ctx.accounts.sender.to_account_info(),
                to: ctx.accounts.balance_holder_pda.to_account_info(),
            },
        );
        transfer(cpi_context, amount_to_deposit)?;

        Ok(())
    }

    pub fn withdraw(ctx: Context<Withdraw>, amount_to_withdraw: u64) -> Result<()> {
        require!(amount_to_withdraw > 0, TransferError::InvalidAmount);
        require!(
            ctx.accounts.balance_holder_pda.amount >= amount_to_withdraw,
            TransferError::InsufficientFunds
        );

        let balance_holder = &mut ctx.accounts.balance_holder_pda;
        let remaining_balance = balance_holder
            .amount
            .checked_sub(amount_to_withdraw)
            .ok_or(TransferError::Underflow)?;
        
        balance_holder.amount = remaining_balance;

        // Get account info references before using them in lamport operations
        let balance_holder_account_info = balance_holder.to_account_info();
        let recipient_account_info = ctx.accounts.recipient.to_account_info();
        
        // Transfer withdrawal amount to recipient
        **balance_holder_account_info.try_borrow_mut_lamports()? = balance_holder_account_info
            .lamports()
            .checked_sub(amount_to_withdraw)
            .ok_or(TransferError::InsufficientLamports)?;
        
        **recipient_account_info.try_borrow_mut_lamports()? = recipient_account_info
            .lamports()
            .checked_add(amount_to_withdraw)
            .ok_or(TransferError::Overflow)?;

        if remaining_balance == 0 {
            // Close account and return rent to sender
            let rent_lamports = ctx.accounts.rent.minimum_balance(balance_holder_account_info.data_len());
            let remaining_lamports = balance_holder_account_info.lamports();
            
            if remaining_lamports > 0 {
                let sender_account_info = ctx.accounts.sender.to_account_info();
                
                **balance_holder_account_info.try_borrow_mut_lamports()? = 0;
                **sender_account_info.try_borrow_mut_lamports()? = sender_account_info
                    .lamports()
                    .checked_add(remaining_lamports)
                    .ok_or(TransferError::Overflow)?;
            }

            // Mark account as closed
            balance_holder.close(ctx.accounts.sender.to_account_info())?;
        }

        Ok(())
    }
}

#[derive(Accounts)]
pub struct Deposit<'info> {
    #[account(mut)]
    sender: Signer<'info>,
    /// CHECK: This account is used for PDA derivation only
    recipient: AccountInfo<'info>,
    #[account(
        init,
        payer = sender,
        space = 8 + 32 + 32 + 8,
        seeds = [recipient.key().as_ref(), sender.key().as_ref()],
        bump
    )]
    balance_holder_pda: Account<'info, BalanceHolderPDA>,
    system_program: Program<'info, System>,
}

#[derive(Accounts)]
pub struct Withdraw<'info> {
    #[account(mut)]
    recipient: Signer<'info>,
    /// CHECK: This account is used for PDA derivation and validation, and will receive rent refund
    #[account(mut)]
    sender: AccountInfo<'info>,
    #[account(
        mut,
        seeds = [recipient.key().as_ref(), sender.key().as_ref()],
        bump,
        has_one = sender,
        has_one = recipient
    )]
    balance_holder_pda: Account<'info, BalanceHolderPDA>,
    rent: Sysvar<'info, Rent>,
}

#[account]
pub struct BalanceHolderPDA {
    sender: Pubkey,
    recipient: Pubkey,
    amount: u64,
}

#[error_code]
pub enum TransferError {
    #[msg("Amount must be greater than zero")]
    InvalidAmount,
    #[msg("Insufficient funds in PDA")]
    InsufficientFunds,
    #[msg("Lamport arithmetic overflow")]
    Overflow,
    #[msg("Lamport arithmetic underflow")]
    Underflow,
    #[msg("Insufficient lamports for operation")]
    InsufficientLamports,
}