#![allow(unexpected_cfgs)]
use anchor_lang::prelude::*;
use anchor_lang::system_program;

declare_id!("Fm6LQ6qtDtFBY9gverXixkugzJPjD3uxocDESyDrVJw8");

#[program]
pub mod transfer_system {
    use super::*;

    /// Deposit funds from sender to PDA
    /// Only the sender can call this function
    pub fn deposit(ctx: Context<DepositCtx>, amount_to_deposit: u64) -> Result<()> {
        // Validate amount is not zero
        require!(amount_to_deposit > 0, TransferError::ZeroAmount);

        let sender = &mut ctx.accounts.sender;
        let balance_holder_pda = &mut ctx.accounts.balance_holder_pda;
        let system_program = &ctx.accounts.system_program;

        // Transfer lamports from sender to PDA
        let transfer_instruction = system_program::Transfer {
            from: sender.to_account_info(),
            to: balance_holder_pda.to_account_info(),
        };

        let cpi_ctx = CpiContext::new(
            system_program.to_account_info(),
            transfer_instruction,
        );

        system_program::transfer(cpi_ctx, amount_to_deposit)?;

        // Initialize or update PDA state
        balance_holder_pda.sender = sender.key();
        balance_holder_pda.recipient = ctx.accounts.recipient.key();
        balance_holder_pda.amount = balance_holder_pda.amount
            .checked_add(amount_to_deposit)
            .ok_or(TransferError::ArithmeticOverflow)?;

        msg!("Deposited {} lamports to PDA", amount_to_deposit);
        Ok(())
    }

    /// Withdraw funds from PDA to recipient
    /// Only the designated recipient can call this function
    pub fn withdraw(ctx: Context<WithdrawCtx>, amount_to_withdraw: u64) -> Result<()> {
        // Validate amount is not zero
        require!(amount_to_withdraw > 0, TransferError::ZeroAmount);

        let balance_holder_pda = &mut ctx.accounts.balance_holder_pda;
        let recipient = &mut ctx.accounts.recipient;

        // Validate sufficient balance
        require!(
            balance_holder_pda.amount >= amount_to_withdraw,
            TransferError::InsufficientFunds
        );

        // Validate recipient matches PDA state
        require!(
            balance_holder_pda.recipient == recipient.key(),
            TransferError::UnauthorizedRecipient
        );

        // Validate sender matches PDA state for additional security
        require!(
            balance_holder_pda.sender == ctx.accounts.sender.key(),
            TransferError::InvalidSender
        );

        // Calculate remaining balance
        let remaining_balance = balance_holder_pda.amount
            .checked_sub(amount_to_withdraw)
            .ok_or(TransferError::ArithmeticUnderflow)?;

        // Transfer lamports from PDA to recipient
        **balance_holder_pda.to_account_info().try_borrow_mut_lamports()? = balance_holder_pda
            .to_account_info()
            .lamports()
            .checked_sub(amount_to_withdraw)
            .ok_or(TransferError::ArithmeticUnderflow)?;

        **recipient.to_account_info().try_borrow_mut_lamports()? = recipient
            .to_account_info()
            .lamports()
            .checked_add(amount_to_withdraw)
            .ok_or(TransferError::ArithmeticOverflow)?;

        // Update PDA state
        balance_holder_pda.amount = remaining_balance;

        // Close PDA account if balance reaches zero
        if remaining_balance == 0 {
            // Return remaining lamports to sender
            let pda_lamports = balance_holder_pda.to_account_info().lamports();
            let sender_info = &ctx.accounts.sender.to_account_info();

            **balance_holder_pda.to_account_info().try_borrow_mut_lamports()? = 0;
            **sender_info.try_borrow_mut_lamports()? = sender_info
                .lamports()
                .checked_add(pda_lamports)
                .ok_or(TransferError::ArithmeticOverflow)?;

            msg!("PDA account closed, remaining lamports returned to sender");
        }

        msg!("Withdrew {} lamports from PDA", amount_to_withdraw);
        Ok(())
    }
}

#[derive(Accounts)]
pub struct DepositCtx<'info> {
    #[account(
        mut,
        constraint = sender.key() != recipient.key() @ TransferError::SenderRecipientSame
    )]
    pub sender: Signer<'info>,

    /// CHECK: Used only for PDA derivation, no additional validation needed
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
pub struct WithdrawCtx<'info> {
    #[account(
        mut,
        constraint = recipient.key() != sender.key() @ TransferError::SenderRecipientSame
    )]
    pub recipient: Signer<'info>,

    /// CHECK: Used for PDA derivation and validation, must be mutable to receive rent refund
    #[account(mut)]
    pub sender: UncheckedAccount<'info>,

    #[account(
        mut,
        seeds = [recipient.key().as_ref(), sender.key().as_ref()],
        bump,
        constraint = balance_holder_pda.sender == sender.key() @ TransferError::InvalidSender,
        constraint = balance_holder_pda.recipient == recipient.key() @ TransferError::UnauthorizedRecipient
    )]
    pub balance_holder_pda: Account<'info, BalanceHolderPDA>,

    pub rent: Sysvar<'info, Rent>,
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
    ZeroAmount,
    #[msg("Insufficient funds in PDA")]
    InsufficientFunds,
    #[msg("Unauthorized recipient")]
    UnauthorizedRecipient,
    #[msg("Invalid sender")]
    InvalidSender,
    #[msg("Sender and recipient cannot be the same")]
    SenderRecipientSame,
    #[msg("Arithmetic overflow")]
    ArithmeticOverflow,
    #[msg("Arithmetic underflow")]
    ArithmeticUnderflow,
}