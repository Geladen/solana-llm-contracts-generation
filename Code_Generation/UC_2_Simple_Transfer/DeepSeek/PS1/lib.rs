use anchor_lang::prelude::*;
use anchor_lang::system_program::{transfer, Transfer};

declare_id!("FnyoFnQKzbfZsfnKEHB8NsF9o1eRC8nUhD3Km3d5fjye");

#[program]
pub mod transfer_contract {
    use super::*;

    pub fn deposit(ctx: Context<Deposit>, amount_to_deposit: u64) -> Result<()> {
        require!(amount_to_deposit > 0, TransferError::ZeroAmount);

        let balance_holder = &mut ctx.accounts.balance_holder_pda;
        
        // Check if we need to initialize the account
        if balance_holder.amount == 0 {
            // Initialize the account
            balance_holder.sender = ctx.accounts.sender.key();
            balance_holder.recipient = ctx.accounts.recipient.key();
            balance_holder.amount = amount_to_deposit;
        } else {
            // Verify the stored sender and recipient match the expected ones
            require!(
                balance_holder.sender == ctx.accounts.sender.key() &&
                balance_holder.recipient == ctx.accounts.recipient.key(),
                TransferError::AccountMismatch
            );
            
            // Update the balance
            balance_holder.amount = balance_holder
                .amount
                .checked_add(amount_to_deposit)
                .ok_or(TransferError::Overflow)?;
        }

        // Transfer funds from sender to PDA
        let cpi_ctx = CpiContext::new(
            ctx.accounts.system_program.to_account_info(),
            Transfer {
                from: ctx.accounts.sender.to_account_info(),
                to: ctx.accounts.balance_holder_pda.to_account_info(),
            },
        );

        transfer(cpi_ctx, amount_to_deposit)
    }

    pub fn withdraw(ctx: Context<Withdraw>, amount_to_withdraw: u64) -> Result<()> {
        require!(amount_to_withdraw > 0, TransferError::ZeroAmount);
        require!(
            ctx.accounts.balance_holder_pda.amount >= amount_to_withdraw,
            TransferError::InsufficientFunds
        );

        let balance_holder = &mut ctx.accounts.balance_holder_pda;
        balance_holder.amount = balance_holder
            .amount
            .checked_sub(amount_to_withdraw)
            .ok_or(TransferError::Underflow)?;

        let should_close = balance_holder.amount == 0;

        // Transfer funds from PDA to recipient using direct lamport manipulation
        let balance_holder_info = ctx.accounts.balance_holder_pda.to_account_info();
        let recipient_info = ctx.accounts.recipient.to_account_info();
        
        // Withdraw the requested amount
        **balance_holder_info.try_borrow_mut_lamports()? = balance_holder_info
            .lamports()
            .checked_sub(amount_to_withdraw)
            .ok_or(TransferError::Underflow)?;

        **recipient_info.try_borrow_mut_lamports()? = recipient_info
            .lamports()
            .checked_add(amount_to_withdraw)
            .ok_or(TransferError::Overflow)?;

        // Close the account if balance is zero and transfer remaining lamports to sender
        if should_close {
            let sender_info = ctx.accounts.sender.to_account_info();
            let remaining_lamports = balance_holder_info.lamports();
            
            // Transfer remaining lamports back to sender
            **sender_info.try_borrow_mut_lamports()? = sender_info
                .lamports()
                .checked_add(remaining_lamports)
                .ok_or(TransferError::Overflow)?;
                
            **balance_holder_info.try_borrow_mut_lamports()? = 0;
        }

        Ok(())
    }
}

#[derive(Accounts)]
pub struct Deposit<'info> {
    #[account(mut)]
    pub sender: Signer<'info>,
    /// CHECK: This account is only used for PDA derivation
    pub recipient: AccountInfo<'info>,
    #[account(
        init,
        payer = sender,
        space = 8 + 32 + 32 + 8,
        seeds = [
            recipient.key().as_ref(), 
            sender.key().as_ref()
        ],
        bump
    )]
    pub balance_holder_pda: Account<'info, BalanceHolderPDA>,
    pub system_program: Program<'info, System>,
}

#[derive(Accounts)]
pub struct Withdraw<'info> {
    #[account(mut)]
    pub recipient: Signer<'info>,
    #[account(mut)]
    /// CHECK: This account is used for PDA derivation and validation, and we refund rent to it
    pub sender: AccountInfo<'info>,
    #[account(
        mut,
        seeds = [
            recipient.key().as_ref(), 
            sender.key().as_ref()
        ],
        bump,
        has_one = sender,
        has_one = recipient
    )]
    pub balance_holder_pda: Account<'info, BalanceHolderPDA>,
    pub system_program: Program<'info, System>,
}

#[account]
pub struct BalanceHolderPDA {
    pub sender: Pubkey,
    pub recipient: Pubkey,
    pub amount: u64,
}

#[error_code]
pub enum TransferError {
    #[msg("Amount must be greater than zero")]
    ZeroAmount,
    #[msg("Insufficient funds in PDA")]
    InsufficientFunds,
    #[msg("Account validation mismatch")]
    AccountMismatch,
    #[msg("Arithmetic overflow")]
    Overflow,
    #[msg("Arithmetic underflow")]
    Underflow,
}