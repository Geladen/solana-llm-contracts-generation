use anchor_lang::prelude::*;
use anchor_lang::system_program::{transfer, Transfer};

declare_id!("6QdsFpMVikLt37CgnMieWjwBtsUWrTHzznX1NckoR8di");

#[program]
pub mod simple_transfer {
    use super::*;

    /// Deposit funds into the contract
    /// Can only be called by the sender/owner
    /// Initializes or updates the PDA with sender, recipient, and amount
    pub fn deposit(ctx: Context<DepositCtx>, amount_to_deposit: u64) -> Result<()> {
        require!(amount_to_deposit > 0, ErrorCode::ZeroAmount);

        // Transfer lamports from sender to PDA
        let transfer_accounts = Transfer {
            from: ctx.accounts.sender.to_account_info(),
            to: ctx.accounts.balance_holder_pda.to_account_info(),
        };
        let cpi_context = CpiContext::new(
            ctx.accounts.system_program.to_account_info(),
            transfer_accounts,
        );
        transfer(cpi_context, amount_to_deposit)?;

        // Update PDA state
        let pda = &mut ctx.accounts.balance_holder_pda;
        pda.sender = ctx.accounts.sender.key();
        pda.recipient = ctx.accounts.recipient.key();
        pda.amount = pda.amount.checked_add(amount_to_deposit)
            .ok_or(ErrorCode::Overflow)?;

        msg!("Deposited {} lamports. New balance: {}", amount_to_deposit, pda.amount);
        Ok(())
    }

    /// Withdraw funds from the contract
    /// Can only be called by the designated recipient
    /// Closes PDA when balance reaches zero, returning remaining lamports to sender
    pub fn withdraw(ctx: Context<WithdrawCtx>, amount_to_withdraw: u64) -> Result<()> {
        require!(amount_to_withdraw > 0, ErrorCode::ZeroAmount);

        // Verify recipient matches stored recipient
        require!(
            ctx.accounts.balance_holder_pda.recipient == ctx.accounts.recipient.key(),
            ErrorCode::UnauthorizedRecipient
        );

        // Verify sender matches stored sender
        require!(
            ctx.accounts.balance_holder_pda.sender == ctx.accounts.sender.key(),
            ErrorCode::InvalidSender
        );

        // Check sufficient balance
        require!(
            ctx.accounts.balance_holder_pda.amount >= amount_to_withdraw,
            ErrorCode::InsufficientFunds
        );

        // Calculate new balance
        let new_balance = ctx.accounts.balance_holder_pda.amount
            .checked_sub(amount_to_withdraw)
            .ok_or(ErrorCode::Underflow)?;

        // Transfer lamports from PDA to recipient
        let pda_info = ctx.accounts.balance_holder_pda.to_account_info();
        let recipient_info = ctx.accounts.recipient.to_account_info();
        
        **pda_info.try_borrow_mut_lamports()? = pda_info
            .lamports()
            .checked_sub(amount_to_withdraw)
            .ok_or(ErrorCode::InsufficientFunds)?;
        
        **recipient_info.try_borrow_mut_lamports()? = recipient_info
            .lamports()
            .checked_add(amount_to_withdraw)
            .ok_or(ErrorCode::Overflow)?;

        if new_balance == 0 {
            // Close PDA account and return remaining lamports to sender
            let remaining_lamports = pda_info.lamports();
            let sender_info = ctx.accounts.sender.to_account_info();
            
            **pda_info.try_borrow_mut_lamports()? = 0;
            **sender_info.try_borrow_mut_lamports()? = sender_info
                .lamports()
                .checked_add(remaining_lamports)
                .ok_or(ErrorCode::Overflow)?;

            msg!("Withdrawn {} lamports. PDA closed, remaining lamports returned to sender.", amount_to_withdraw);
        } else {
            // Update balance
            ctx.accounts.balance_holder_pda.amount = new_balance;
            msg!("Withdrawn {} lamports. Remaining balance: {}", amount_to_withdraw, new_balance);
        }

        Ok(())
    }
}

#[derive(Accounts)]
#[instruction(amount_to_deposit: u64)]
pub struct DepositCtx<'info> {
    #[account(mut)]
    pub sender: Signer<'info>,
    
    /// CHECK: Used only for PDA derivation
    pub recipient: AccountInfo<'info>,
    
    #[account(
        init_if_needed,
        payer = sender,
        space = 8 + BalanceHolderPDA::INIT_SPACE,
        seeds = [recipient.key().as_ref(), sender.key().as_ref()],
        bump
    )]
    pub balance_holder_pda: Account<'info, BalanceHolderPDA>,
    
    pub system_program: Program<'info, System>,
}

#[derive(Accounts)]
#[instruction(amount_to_withdraw: u64)]
pub struct WithdrawCtx<'info> {
    #[account(mut)]
    pub recipient: Signer<'info>,
    
    /// CHECK: Used for PDA derivation and validation
    #[account(mut)]
    pub sender: AccountInfo<'info>,
    
    #[account(
        mut,
        seeds = [recipient.key().as_ref(), sender.key().as_ref()],
        bump,
        constraint = balance_holder_pda.sender == sender.key() @ ErrorCode::InvalidSender,
        constraint = balance_holder_pda.recipient == recipient.key() @ ErrorCode::UnauthorizedRecipient
    )]
    pub balance_holder_pda: Account<'info, BalanceHolderPDA>,
    
    pub rent: Sysvar<'info, Rent>,
}

#[account]
#[derive(InitSpace)]
pub struct BalanceHolderPDA {
    pub sender: Pubkey,
    pub recipient: Pubkey,
    pub amount: u64,
}

#[error_code]
pub enum ErrorCode {
    #[msg("Amount must be greater than zero")]
    ZeroAmount,
    #[msg("Insufficient funds in contract")]
    InsufficientFunds,
    #[msg("Unauthorized recipient")]
    UnauthorizedRecipient,
    #[msg("Invalid sender")]
    InvalidSender,
    #[msg("Arithmetic overflow")]
    Overflow,
    #[msg("Arithmetic underflow")]
    Underflow,
}