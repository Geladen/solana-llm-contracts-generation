use anchor_lang::prelude::*;
use anchor_lang::system_program;

declare_id!("Ws9avtvV4nKhTRMp8YUeAYGpfCyQmrAs67g9ZBTZdWc");

#[program]
pub mod simple_transfer {
    use super::*;

    /// Deposit funds into the contract
    /// Can only be called by the sender/owner
    /// Initializes the PDA with sender, recipient, and amount information
    pub fn deposit(ctx: Context<DepositCtx>, amount_to_deposit: u64) -> Result<()> {
        // Validate amount is not zero
        require!(amount_to_deposit > 0, TransferError::ZeroAmount);

        // Transfer lamports from sender to PDA
        let transfer_ix = system_program::Transfer {
            from: ctx.accounts.sender.to_account_info(),
            to: ctx.accounts.balance_holder_pda.to_account_info(),
        };
        let cpi_ctx = CpiContext::new(
            ctx.accounts.system_program.to_account_info(),
            transfer_ix,
        );
        system_program::transfer(cpi_ctx, amount_to_deposit)?;

        // Update PDA state
        let balance_holder = &mut ctx.accounts.balance_holder_pda;
        balance_holder.sender = ctx.accounts.sender.key();
        balance_holder.recipient = ctx.accounts.recipient.key();
        balance_holder.amount += amount_to_deposit;

        msg!(
            "Deposited {} lamports. Total balance: {}",
            amount_to_deposit,
            balance_holder.amount
        );

        Ok(())
    }

    /// Withdraw funds from the contract
    /// Can only be called by the designated recipient
    /// Closes PDA when balance reaches zero
    pub fn withdraw(ctx: Context<WithdrawCtx>, amount_to_withdraw: u64) -> Result<()> {
        // Validate amount is not zero
        require!(amount_to_withdraw > 0, TransferError::ZeroAmount);

        // Validate sufficient balance
        require!(
            ctx.accounts.balance_holder_pda.amount >= amount_to_withdraw,
            TransferError::InsufficientFunds
        );

        // Validate recipient matches stored recipient
        require!(
            ctx.accounts.balance_holder_pda.recipient == ctx.accounts.recipient.key(),
            TransferError::UnauthorizedRecipient
        );

        // Validate sender matches stored sender (for PDA derivation)
        require!(
            ctx.accounts.balance_holder_pda.sender == ctx.accounts.sender.key(),
            TransferError::InvalidSender
        );

        // Store remaining balance for logging and closure check
        let remaining_balance = ctx.accounts.balance_holder_pda.amount - amount_to_withdraw;

        // Update balance before transfer
        ctx.accounts.balance_holder_pda.amount = remaining_balance;

        // Transfer lamports from PDA to recipient
        **ctx.accounts.balance_holder_pda.to_account_info().try_borrow_mut_lamports()? -= amount_to_withdraw;
        **ctx.accounts.recipient.to_account_info().try_borrow_mut_lamports()? += amount_to_withdraw;

        msg!(
            "Withdrew {} lamports. Remaining balance: {}",
            amount_to_withdraw,
            remaining_balance
        );

        // Close PDA if balance is zero
        if remaining_balance == 0 {
            // Transfer remaining lamports (rent) to sender
            let remaining_lamports = ctx.accounts.balance_holder_pda.to_account_info().lamports();
            
            **ctx.accounts.balance_holder_pda.to_account_info().try_borrow_mut_lamports()? = 0;
            **ctx.accounts.sender.to_account_info().try_borrow_mut_lamports()? += remaining_lamports;

            msg!("PDA closed. Rent returned to sender.");
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
        constraint = balance_holder_pda.sender == sender.key() @ TransferError::InvalidSender,
        constraint = balance_holder_pda.recipient == recipient.key() @ TransferError::UnauthorizedRecipient
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
pub enum TransferError {
    #[msg("Amount must be greater than zero")]
    ZeroAmount,
    #[msg("Insufficient funds in contract")]
    InsufficientFunds,
    #[msg("Unauthorized recipient")]
    UnauthorizedRecipient,
    #[msg("Invalid sender")]
    InvalidSender,
}