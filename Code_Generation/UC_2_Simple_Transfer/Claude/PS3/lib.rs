#![allow(unexpected_cfgs)]
use anchor_lang::prelude::*;
use anchor_lang::system_program;

declare_id!("BtPavqaHQh4tgb7hkS5jBuwQ4EeJ5reKW1KgzyLG7thf");

#[program]
pub mod transfer_contract {
    use super::*;

    /// Deposits funds from sender to a PDA account designated for a specific recipient
    pub fn deposit(ctx: Context<DepositCtx>, amount_to_deposit: u64) -> Result<()> {
        // Validate amount is not zero
        require!(amount_to_deposit > 0, TransferError::InvalidAmount);

        let balance_holder_pda = &mut ctx.accounts.balance_holder_pda;
        let sender = &mut ctx.accounts.sender;
        let recipient = &ctx.accounts.recipient;

        // Initialize PDA state (this is a new account due to init constraint)
        balance_holder_pda.sender = sender.key();
        balance_holder_pda.recipient = recipient.key();
        balance_holder_pda.amount = amount_to_deposit;

        // Transfer lamports from sender to PDA
        let cpi_context = CpiContext::new(
            ctx.accounts.system_program.to_account_info(),
            system_program::Transfer {
                from: sender.to_account_info(),
                to: balance_holder_pda.to_account_info(),
            },
        );
        
        system_program::transfer(cpi_context, amount_to_deposit)?;

        Ok(())
    }

    /// Adds additional funds to an existing PDA account
    pub fn add_deposit(ctx: Context<AddDepositCtx>, amount_to_deposit: u64) -> Result<()> {
        // Validate amount is not zero
        require!(amount_to_deposit > 0, TransferError::InvalidAmount);

        let balance_holder_pda = &mut ctx.accounts.balance_holder_pda;
        let sender = &mut ctx.accounts.sender;

        // Validate sender matches PDA state
        require!(
            balance_holder_pda.sender == sender.key(),
            TransferError::InvalidSender
        );

        // Update PDA balance
        balance_holder_pda.amount = balance_holder_pda.amount.checked_add(amount_to_deposit)
            .ok_or(TransferError::ArithmeticOverflow)?;

        // Transfer lamports from sender to PDA
        let cpi_context = CpiContext::new(
            ctx.accounts.system_program.to_account_info(),
            system_program::Transfer {
                from: sender.to_account_info(),
                to: balance_holder_pda.to_account_info(),
            },
        );
        
        system_program::transfer(cpi_context, amount_to_deposit)?;

        Ok(())
    }

    /// Withdraws funds from PDA to the designated recipient
    pub fn withdraw(ctx: Context<WithdrawCtx>, amount_to_withdraw: u64) -> Result<()> {
        // Validate amount is not zero
        require!(amount_to_withdraw > 0, TransferError::InvalidAmount);

        let balance_holder_pda = &mut ctx.accounts.balance_holder_pda;
        let recipient = &mut ctx.accounts.recipient;

        // Validate recipient matches PDA state
        require!(
            balance_holder_pda.recipient == recipient.key(),
            TransferError::InvalidRecipient
        );

        // Validate sufficient balance
        require!(
            balance_holder_pda.amount >= amount_to_withdraw,
            TransferError::InsufficientFunds
        );

        // Calculate new balance
        let new_balance = balance_holder_pda.amount.checked_sub(amount_to_withdraw)
            .ok_or(TransferError::ArithmeticUnderflow)?;

        // Transfer lamports from PDA to recipient
        **balance_holder_pda.to_account_info().try_borrow_mut_lamports()? -= amount_to_withdraw;
        **recipient.to_account_info().try_borrow_mut_lamports()? += amount_to_withdraw;

        if new_balance == 0 {
            // Close account manually and return remaining lamports to sender
            let sender = &ctx.accounts.sender;
            let remaining_lamports = balance_holder_pda.to_account_info().lamports();
            
            **balance_holder_pda.to_account_info().try_borrow_mut_lamports()? = 0;
            **sender.to_account_info().try_borrow_mut_lamports()? += remaining_lamports;
        }

        // Update PDA balance
        balance_holder_pda.amount = new_balance;

        Ok(())
    }
}

#[derive(Accounts)]
pub struct DepositCtx<'info> {
    #[account(mut)]
    pub sender: Signer<'info>,
    
    /// CHECK: Used only for PDA derivation, no validation needed on the account itself
    pub recipient: UncheckedAccount<'info>,
    
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
pub struct AddDepositCtx<'info> {
    #[account(mut)]
    pub sender: Signer<'info>,
    
    /// CHECK: Used only for PDA derivation, no validation needed on the account itself
    pub recipient: UncheckedAccount<'info>,
    
    #[account(
        mut,
        seeds = [recipient.key().as_ref(), sender.key().as_ref()],
        bump,
        constraint = balance_holder_pda.sender == sender.key() @ TransferError::InvalidSender
    )]
    pub balance_holder_pda: Account<'info, BalanceHolderPDA>,
    
    pub system_program: Program<'info, System>,
}

#[derive(Accounts)]
pub struct WithdrawCtx<'info> {
    #[account(mut)]
    pub recipient: Signer<'info>,
    
    /// CHECK: Used for PDA derivation and receiving rent refund
    #[account(mut)]
    pub sender: UncheckedAccount<'info>,
    
    #[account(
        mut,
        seeds = [recipient.key().as_ref(), sender.key().as_ref()],
        bump,
        constraint = balance_holder_pda.sender == sender.key() @ TransferError::InvalidSender,
        constraint = balance_holder_pda.recipient == recipient.key() @ TransferError::InvalidRecipient
    )]
    pub balance_holder_pda: Account<'info, BalanceHolderPDA>,
}

#[account]
pub struct BalanceHolderPDA {
    pub sender: Pubkey,     // 32 bytes
    pub recipient: Pubkey,  // 32 bytes
    pub amount: u64,        // 8 bytes
}

#[error_code]
pub enum TransferError {
    #[msg("Amount must be greater than zero")]
    InvalidAmount,
    #[msg("Invalid recipient - not authorized to withdraw from this PDA")]
    InvalidRecipient,
    #[msg("Invalid sender - PDA sender mismatch")]
    InvalidSender,
    #[msg("Insufficient funds in PDA account")]
    InsufficientFunds,
    #[msg("Arithmetic overflow occurred")]
    ArithmeticOverflow,
    #[msg("Arithmetic underflow occurred")]
    ArithmeticUnderflow,
}