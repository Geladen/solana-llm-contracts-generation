use anchor_lang::prelude::*;
use anchor_lang::system_program;

declare_id!("BtPavqaHQh4tgb7hkS5jBuwQ4EeJ5reKW1KgzyLG7thf");

#[program]
pub mod transfer_contract {
    use super::*;

    /// Initialize a new transfer contract and deposit funds
    pub fn deposit(ctx: Context<DepositCtx>, amount_to_deposit: u64) -> Result<()> {
        // Validate amount is not zero
        require!(amount_to_deposit > 0, TransferError::InvalidAmount);

        let sender = &ctx.accounts.sender;
        let recipient = &ctx.accounts.recipient;
        let balance_holder_pda = &mut ctx.accounts.balance_holder_pda;
        let system_program = &ctx.accounts.system_program;

        // Initialize PDA state
        balance_holder_pda.sender = sender.key();
        balance_holder_pda.recipient = recipient.key();
        balance_holder_pda.amount = amount_to_deposit;

        // Transfer lamports from sender to PDA
        system_program::transfer(
            CpiContext::new(
                system_program.to_account_info(),
                system_program::Transfer {
                    from: sender.to_account_info(),
                    to: balance_holder_pda.to_account_info(),
                },
            ),
            amount_to_deposit,
        )?;

        emit!(DepositEvent {
            sender: sender.key(),
            recipient: recipient.key(),
            amount: amount_to_deposit,
            total_balance: balance_holder_pda.amount,
        });

        Ok(())
    }

    /// Withdraw funds from the contract
    pub fn withdraw(ctx: Context<WithdrawCtx>, amount_to_withdraw: u64) -> Result<()> {
        // Validate amount is not zero
        require!(amount_to_withdraw > 0, TransferError::InvalidAmount);

        let recipient = &ctx.accounts.recipient;
        let sender = &ctx.accounts.sender;
        let balance_holder_pda = &mut ctx.accounts.balance_holder_pda;

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

        // Validate sender matches PDA state
        require!(
            balance_holder_pda.sender == sender.key(),
            TransferError::InvalidSender
        );

        // Update balance
        balance_holder_pda.amount = balance_holder_pda
            .amount
            .checked_sub(amount_to_withdraw)
            .ok_or(TransferError::ArithmeticUnderflow)?;

        // Transfer lamports from PDA to recipient
        **balance_holder_pda.to_account_info().try_borrow_mut_lamports()? -= amount_to_withdraw;
        **recipient.to_account_info().try_borrow_mut_lamports()? += amount_to_withdraw;

        let remaining_balance = balance_holder_pda.amount;

        emit!(WithdrawEvent {
            sender: sender.key(),
            recipient: recipient.key(),
            amount: amount_to_withdraw,
            remaining_balance,
        });

        // Close PDA account if balance reaches zero
        if remaining_balance == 0 {
            // Get remaining rent lamports before closing
            let pda_account_info = balance_holder_pda.to_account_info();
            let rent_lamports = pda_account_info.lamports();
            
            // Transfer all remaining lamports to sender
            **pda_account_info.try_borrow_mut_lamports()? = 0;
            **sender.try_borrow_mut_lamports()? += rent_lamports;

            emit!(AccountClosedEvent {
                sender: sender.key(),
                recipient: recipient.key(),
                rent_returned: rent_lamports,
            });
        }

        Ok(())
    }
}

#[derive(Accounts)]
pub struct DepositCtx<'info> {
    #[account(mut)]
    pub sender: Signer<'info>,
    
    /// CHECK: This account is used only for PDA derivation
    pub recipient: AccountInfo<'info>,
    
    #[account(
        init,
        payer = sender,
        space = 8 + 32 + 32 + 8, // discriminator + sender pubkey + recipient pubkey + amount u64
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
    pub sender: AccountInfo<'info>,
    
    #[account(
        mut,
        seeds = [recipient.key().as_ref(), sender.key().as_ref()],
        bump
    )]
    pub balance_holder_pda: Account<'info, BalanceHolderPDA>,
    
    pub rent: Sysvar<'info, Rent>,
}

#[account]
pub struct BalanceHolderPDA {
    pub sender: Pubkey,
    pub recipient: Pubkey,
    pub amount: u64,
}

#[event]
pub struct DepositEvent {
    pub sender: Pubkey,
    pub recipient: Pubkey,
    pub amount: u64,
    pub total_balance: u64,
}

#[event]
pub struct WithdrawEvent {
    pub sender: Pubkey,
    pub recipient: Pubkey,
    pub amount: u64,
    pub remaining_balance: u64,
}

#[event]
pub struct AccountClosedEvent {
    pub sender: Pubkey,
    pub recipient: Pubkey,
    pub rent_returned: u64,
}

#[error_code]
pub enum TransferError {
    #[msg("Amount must be greater than zero")]
    InvalidAmount,
    #[msg("Insufficient funds in the contract")]
    InsufficientFunds,
    #[msg("Unauthorized recipient")]
    UnauthorizedRecipient,
    #[msg("Invalid sender")]
    InvalidSender,
    #[msg("Arithmetic overflow")]
    ArithmeticOverflow,
    #[msg("Arithmetic underflow")]
    ArithmeticUnderflow,
}
