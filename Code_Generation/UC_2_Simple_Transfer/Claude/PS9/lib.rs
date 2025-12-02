use anchor_lang::prelude::*;
use anchor_lang::system_program;

declare_id!("Hm6ZofyhVXCZLerpFSpfkt5n6vF42fmfJdeCtng4ATQ5");

#[program]
pub mod simple_transfer {
    use super::*;

    /// Deposit funds into the contract
    /// Must be called by the sender/owner
    /// Initializes the PDA with sender, recipient, and amount
    pub fn deposit(ctx: Context<DepositCtx>, amount_to_deposit: u64) -> Result<()> {
        // Validate amount is non-zero
        require!(amount_to_deposit > 0, TransferError::InvalidAmount);

        // Transfer lamports from sender to PDA
        let transfer_cpi_accounts = system_program::Transfer {
            from: ctx.accounts.sender.to_account_info(),
            to: ctx.accounts.balance_holder_pda.to_account_info(),
        };
        let transfer_cpi_ctx = CpiContext::new(
            ctx.accounts.system_program.to_account_info(),
            transfer_cpi_accounts,
        );
        system_program::transfer(transfer_cpi_ctx, amount_to_deposit)?;

        // Initialize or update PDA state
        let pda = &mut ctx.accounts.balance_holder_pda;
        pda.sender = ctx.accounts.sender.key();
        pda.recipient = ctx.accounts.recipient.key();
        pda.amount = pda.amount.checked_add(amount_to_deposit)
            .ok_or(TransferError::Overflow)?;

        msg!("Deposit successful: {} lamports", amount_to_deposit);
        msg!("Total balance: {} lamports", pda.amount);

        Ok(())
    }

    /// Withdraw funds from the contract
    /// Must be called by the designated recipient
    /// Closes PDA when balance reaches zero
    pub fn withdraw(ctx: Context<WithdrawCtx>, amount_to_withdraw: u64) -> Result<()> {
        // Validate amount is non-zero
        require!(amount_to_withdraw > 0, TransferError::InvalidAmount);

        // Validate sufficient balance
        require!(
            ctx.accounts.balance_holder_pda.amount >= amount_to_withdraw,
            TransferError::InsufficientFunds
        );

        // Calculate new balance
        let new_balance = ctx.accounts.balance_holder_pda.amount.checked_sub(amount_to_withdraw)
            .ok_or(TransferError::Underflow)?;

        // Transfer lamports from PDA to recipient
        **ctx.accounts.balance_holder_pda.to_account_info().try_borrow_mut_lamports()? -= amount_to_withdraw;
        **ctx.accounts.recipient.to_account_info().try_borrow_mut_lamports()? += amount_to_withdraw;

        // Update balance
        ctx.accounts.balance_holder_pda.amount = new_balance;

        msg!("Withdrawal successful: {} lamports", amount_to_withdraw);
        msg!("Remaining balance: {} lamports", new_balance);

        // Close PDA if balance is zero
        if new_balance == 0 {
            msg!("Balance is zero, closing PDA and returning rent to sender");
            
            // Get remaining lamports in PDA (rent)
            let remaining_lamports = ctx.accounts.balance_holder_pda.to_account_info().lamports();
            
            // Transfer remaining lamports to sender
            **ctx.accounts.balance_holder_pda.to_account_info().try_borrow_mut_lamports()? = 0;
            **ctx.accounts.sender.to_account_info().try_borrow_mut_lamports()? += remaining_lamports;
            
            msg!("Returned {} lamports rent to sender", remaining_lamports);
        }

        Ok(())
    }
}

#[derive(Accounts)]
#[instruction(amount_to_deposit: u64)]
pub struct DepositCtx<'info> {
    /// Sender account (signer, mutable)
    #[account(mut)]
    pub sender: Signer<'info>,

    /// Recipient account (reference, for PDA derivation)
    /// CHECK: This account is used only for PDA derivation
    pub recipient: AccountInfo<'info>,

    /// PDA to hold the balance
    #[account(
        init_if_needed,
        payer = sender,
        space = 8 + BalanceHolderPDA::INIT_SPACE,
        seeds = [recipient.key().as_ref(), sender.key().as_ref()],
        bump
    )]
    pub balance_holder_pda: Account<'info, BalanceHolderPDA>,

    /// System program
    pub system_program: Program<'info, System>,
}

#[derive(Accounts)]
#[instruction(amount_to_withdraw: u64)]
pub struct WithdrawCtx<'info> {
    /// Recipient account (signer, mutable)
    #[account(mut)]
    pub recipient: Signer<'info>,

    /// Sender account (reference, for PDA derivation and validation)
    /// CHECK: This account is validated through PDA seeds
    #[account(mut)]
    pub sender: AccountInfo<'info>,

    /// PDA holding the balance
    #[account(
        mut,
        seeds = [recipient.key().as_ref(), sender.key().as_ref()],
        bump,
        constraint = balance_holder_pda.sender == sender.key() @ TransferError::InvalidSender,
        constraint = balance_holder_pda.recipient == recipient.key() @ TransferError::InvalidRecipient,
    )]
    pub balance_holder_pda: Account<'info, BalanceHolderPDA>,

    /// Rent sysvar (for account closure)
    pub rent: Sysvar<'info, Rent>,
}

#[account]
#[derive(InitSpace)]
pub struct BalanceHolderPDA {
    /// The sender/owner who deposited funds
    pub sender: Pubkey,
    /// The designated recipient who can withdraw
    pub recipient: Pubkey,
    /// The amount of lamports held (tracked separately from actual lamports)
    pub amount: u64,
}

#[error_code]
pub enum TransferError {
    #[msg("Amount must be greater than zero")]
    InvalidAmount,
    #[msg("Insufficient funds in PDA")]
    InsufficientFunds,
    #[msg("Invalid sender")]
    InvalidSender,
    #[msg("Invalid recipient")]
    InvalidRecipient,
    #[msg("Arithmetic overflow")]
    Overflow,
    #[msg("Arithmetic underflow")]
    Underflow,
}