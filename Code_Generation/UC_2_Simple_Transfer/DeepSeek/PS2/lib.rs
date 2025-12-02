use anchor_lang::prelude::*;
use anchor_lang::system_program;

declare_id!("Ac2UYENaE44h8j8F8YZnfw22rpqXTSdfHkHjrVy7KmZi");

#[program]
pub mod simple_transfer {
    use super::*;

    pub fn deposit(ctx: Context<DepositCtx>, amount_to_deposit: u64) -> Result<()> {
        // Validate amount is positive
        require!(amount_to_deposit > 0, TransferError::InvalidAmount);

        let balance_holder = &mut ctx.accounts.balance_holder_pda;
        
        // If account is being initialized, set sender and recipient
        if balance_holder.amount == 0 {
            balance_holder.sender = ctx.accounts.sender.key();
            balance_holder.recipient = ctx.accounts.recipient.key();
        } else {
            // Verify the sender and recipient match the stored ones
            require!(
                balance_holder.sender == ctx.accounts.sender.key(),
                TransferError::SenderMismatch
            );
            require!(
                balance_holder.recipient == ctx.accounts.recipient.key(),
                TransferError::RecipientMismatch
            );
        }

        // Update the balance
        balance_holder.amount = balance_holder
            .amount
            .checked_add(amount_to_deposit)
            .ok_or(TransferError::Overflow)?;

        // Transfer funds from sender to PDA
        let cpi_context = CpiContext::new(
            ctx.accounts.system_program.to_account_info(),
            system_program::Transfer {
                from: ctx.accounts.sender.to_account_info(),
                to: ctx.accounts.balance_holder_pda.to_account_info(),
            },
        );
        system_program::transfer(cpi_context, amount_to_deposit)?;

        Ok(())
    }

    pub fn withdraw(ctx: Context<WithdrawCtx>, amount_to_withdraw: u64) -> Result<()> {
        // Validate amount is positive
        require!(amount_to_withdraw > 0, TransferError::InvalidAmount);

        let balance_holder = &mut ctx.accounts.balance_holder_pda;
        
        // Verify PDA matches the expected sender and recipient
        require!(
            balance_holder.sender == ctx.accounts.sender.key(),
            TransferError::SenderMismatch
        );
        require!(
            balance_holder.recipient == ctx.accounts.recipient.key(),
            TransferError::RecipientMismatch
        );
        
        // Check sufficient balance
        require!(
            balance_holder.amount >= amount_to_withdraw,
            TransferError::InsufficientFunds
        );

        // Store the amount before updating for closure check
        let new_amount = balance_holder
            .amount
            .checked_sub(amount_to_withdraw)
            .ok_or(TransferError::Underflow)?;

        // Update balance
        balance_holder.amount = new_amount;

        // Get account info references before doing lamports manipulation
        let balance_holder_info = ctx.accounts.balance_holder_pda.to_account_info();
        let recipient_info = ctx.accounts.recipient.to_account_info();

        // Get current lamports in PDA
        let current_lamports = balance_holder_info.lamports();
        let transfer_lamports = amount_to_withdraw.min(current_lamports);

        // Transfer lamports from PDA to recipient
        **balance_holder_info.try_borrow_mut_lamports()? = current_lamports
            .checked_sub(transfer_lamports)
            .ok_or(TransferError::InsufficientFunds)?;
        **recipient_info.try_borrow_mut_lamports()? = recipient_info
            .lamports()
            .checked_add(transfer_lamports)
            .ok_or(TransferError::Overflow)?;

        // Close PDA if balance reaches zero
        if new_amount == 0 {
            let sender_info = ctx.accounts.sender.to_account_info();
            let remaining_lamports = balance_holder_info.lamports();
            
            // Transfer remaining lamports (rent exemption) back to sender
            **balance_holder_info.try_borrow_mut_lamports()? = 0;
            **sender_info.try_borrow_mut_lamports()? = sender_info
                .lamports()
                .checked_add(remaining_lamports)
                .ok_or(TransferError::Overflow)?;
        }

        Ok(())
    }
}

#[derive(Accounts)]
pub struct DepositCtx<'info> {
    /// CHECK: Verified through signer and PDA derivation
    #[account(mut)]
    pub sender: Signer<'info>,
    
    /// CHECK: Used only for PDA derivation
    pub recipient: AccountInfo<'info>,
    
    #[account(
        init_if_needed,
        payer = sender,
        space = BalanceHolderPDA::SIZE,
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
pub struct WithdrawCtx<'info> {
    /// CHECK: Verified through signer and PDA validation
    #[account(mut)]
    pub recipient: Signer<'info>,
    
    /// CHECK: Used for PDA derivation and validation, made mutable for rent refund
    #[account(mut)]
    pub sender: AccountInfo<'info>,
    
    #[account(
        mut,
        seeds = [
            recipient.key().as_ref(),
            sender.key().as_ref()
        ],
        bump,
        constraint = balance_holder_pda.recipient == recipient.key() @ TransferError::RecipientMismatch,
        constraint = balance_holder_pda.sender == sender.key() @ TransferError::SenderMismatch
    )]
    pub balance_holder_pda: Account<'info, BalanceHolderPDA>,
    
    /// CHECK: Required for potential account closure
    pub rent: Sysvar<'info, Rent>,
}

#[account]
pub struct BalanceHolderPDA {
    pub sender: Pubkey,
    pub recipient: Pubkey,
    pub amount: u64,
}

impl BalanceHolderPDA {
    pub const SIZE: usize = 8 + // discriminator
        32 + // sender
        32 + // recipient
        8;   // amount
}

#[error_code]
pub enum TransferError {
    #[msg("Amount must be greater than zero")]
    InvalidAmount,
    #[msg("Insufficient funds in contract")]
    InsufficientFunds,
    #[msg("Sender does not match PDA record")]
    SenderMismatch,
    #[msg("Recipient does not match PDA record")]
    RecipientMismatch,
    #[msg("Arithmetic overflow")]
    Overflow,
    #[msg("Arithmetic underflow")]
    Underflow,
}