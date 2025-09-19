use anchor_lang::prelude::*;
use anchor_lang::system_program;

declare_id!("Fm6LQ6qtDtFBY9gverXixkugzJPjD3uxocDESyDrVJw8");


#[program]
pub mod transfer_contract {
    use super::*;

    /// Deposit funds into the contract
    /// Only the sender/owner can call this function
    pub fn deposit(ctx: Context<DepositCtx>, amount_to_deposit: u64) -> Result<()> {
        // Validate amount is not zero
        require!(amount_to_deposit > 0, TransferError::ZeroAmountNotAllowed);

        let sender = &ctx.accounts.sender;
        let recipient = &ctx.accounts.recipient;
        let balance_holder_pda = &mut ctx.accounts.balance_holder_pda;

        // Initialize PDA state
        balance_holder_pda.sender = sender.key();
        balance_holder_pda.recipient = recipient.key();
        balance_holder_pda.amount = amount_to_deposit;

        // Transfer lamports from sender to PDA
        let transfer_instruction = system_program::Transfer {
            from: sender.to_account_info(),
            to: balance_holder_pda.to_account_info(),
        };

        let cpi_ctx = CpiContext::new(
            ctx.accounts.system_program.to_account_info(),
            transfer_instruction,
        );

        system_program::transfer(cpi_ctx, amount_to_deposit)?;

        msg!(
            "Deposited {} lamports from {} to PDA for recipient {}",
            amount_to_deposit,
            sender.key(),
            recipient.key()
        );

        Ok(())
    }

    /// Add more funds to an existing contract
    /// Only the sender/owner can call this function
    pub fn add_funds(ctx: Context<AddFundsCtx>, amount_to_add: u64) -> Result<()> {
        // Validate amount is not zero
        require!(amount_to_add > 0, TransferError::ZeroAmountNotAllowed);

        let sender = &ctx.accounts.sender;
        let balance_holder_pda = &mut ctx.accounts.balance_holder_pda;

        // Validate that the sender matches the one stored in PDA
        require_eq!(
            balance_holder_pda.sender,
            sender.key(),
            TransferError::InvalidSender
        );

        // Add to existing amount
        balance_holder_pda.amount = balance_holder_pda.amount
            .checked_add(amount_to_add)
            .ok_or(TransferError::Overflow)?;

        // Transfer lamports from sender to PDA
        let transfer_instruction = system_program::Transfer {
            from: sender.to_account_info(),
            to: balance_holder_pda.to_account_info(),
        };

        let cpi_ctx = CpiContext::new(
            ctx.accounts.system_program.to_account_info(),
            transfer_instruction,
        );

        system_program::transfer(cpi_ctx, amount_to_add)?;

        msg!(
            "Added {} lamports from {}. New balance: {}",
            amount_to_add,
            sender.key(),
            balance_holder_pda.amount
        );

        Ok(())
    }

    /// Withdraw funds from the contract
    /// Only the designated recipient can call this function
    pub fn withdraw(ctx: Context<WithdrawCtx>, amount_to_withdraw: u64) -> Result<()> {
        // Validate amount is not zero
        require!(amount_to_withdraw > 0, TransferError::ZeroAmountNotAllowed);

        let recipient = &ctx.accounts.recipient;
        let sender = &ctx.accounts.sender;
        let balance_holder_pda = &mut ctx.accounts.balance_holder_pda;

        msg!("Withdraw called - PDA key: {}, amount: {}", 
             balance_holder_pda.key(), amount_to_withdraw);

        // Validate that the recipient matches the one stored in PDA
        require_eq!(
            balance_holder_pda.recipient,
            recipient.key(),
            TransferError::UnauthorizedRecipient
        );

        // Validate that the sender matches the one stored in PDA
        require_eq!(
            balance_holder_pda.sender,
            sender.key(),
            TransferError::InvalidSender
        );

        // Validate sufficient balance
        require!(
            balance_holder_pda.amount >= amount_to_withdraw,
            TransferError::InsufficientBalance
        );

        // Calculate new balance
        let new_balance = balance_holder_pda.amount
            .checked_sub(amount_to_withdraw)
            .ok_or(TransferError::Underflow)?;

        // Update PDA balance
        balance_holder_pda.amount = new_balance;

        msg!(
            "Withdraw successful - Recipient: {}, Amount: {}, Remaining balance: {}",
            recipient.key(),
            amount_to_withdraw,
            new_balance
        );

        Ok(())
    }
}

#[derive(Accounts)]
pub struct DepositCtx<'info> {
    #[account(mut)]
    pub sender: Signer<'info>,
    
    /// CHECK: This account is used only for PDA derivation
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
pub struct AddFundsCtx<'info> {
    #[account(mut)]
    pub sender: Signer<'info>,
    
    /// CHECK: This account is used only for PDA derivation
    pub recipient: UncheckedAccount<'info>,
    
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
    
    /// CHECK: This account is used for PDA derivation and validation
    #[account(mut)]
    pub sender: UncheckedAccount<'info>,
    
    #[account(
        mut,
        seeds = [recipient.key().as_ref(), sender.key().as_ref()],
        bump
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
    ZeroAmountNotAllowed,
    
    #[msg("Insufficient balance for withdrawal")]
    InsufficientBalance,
    
    #[msg("Unauthorized recipient")]
    UnauthorizedRecipient,
    
    #[msg("Invalid sender")]
    InvalidSender,
    
    #[msg("Arithmetic overflow")]
    Overflow,
    
    #[msg("Arithmetic underflow")]
    Underflow,
}