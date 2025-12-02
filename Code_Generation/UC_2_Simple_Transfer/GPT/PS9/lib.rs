use anchor_lang::prelude::*;
use anchor_lang::system_program;
declare_id!("GcbmimoUEGBHC2TBxc1HK7sEa514HpY6P4URuP79pRVA");

#[program]
pub mod simple_transfer {
    use super::*;

    pub fn deposit(ctx: Context<DepositCtx>, amount_to_deposit: u64) -> Result<()> {
        // Reject zero deposits
        require!(amount_to_deposit > 0, CustomError::InvalidAmount);

        let balance_holder = &mut ctx.accounts.balance_holder_pda;

        // Initialize or update PDA state
        balance_holder.sender = ctx.accounts.sender.key();
        balance_holder.recipient = ctx.accounts.recipient.key();
        balance_holder.amount = balance_holder.amount.checked_add(amount_to_deposit)
            .ok_or(CustomError::Overflow)?;

        // Transfer lamports from sender to PDA
        let cpi_ctx = CpiContext::new(
            ctx.accounts.system_program.to_account_info(),
            system_program::Transfer {
                from: ctx.accounts.sender.to_account_info(),
                to: balance_holder.to_account_info(),
            },
        );
        system_program::transfer(cpi_ctx, amount_to_deposit)?;

        Ok(())
    }

    pub fn withdraw(ctx: Context<WithdrawCtx>, amount_to_withdraw: u64) -> Result<()> {
        // Reject zero withdrawals
        require!(amount_to_withdraw > 0, CustomError::InvalidAmount);

        let balance_holder = &mut ctx.accounts.balance_holder_pda;

        // Ensure the PDA has enough funds
        require!(balance_holder.amount >= amount_to_withdraw, CustomError::InsufficientFunds);

        // Transfer lamports from PDA to recipient
        **balance_holder.to_account_info().try_borrow_mut_lamports()? -= amount_to_withdraw;
        **ctx.accounts.recipient.to_account_info().try_borrow_mut_lamports()? += amount_to_withdraw;

        // Update PDA state
        balance_holder.amount = balance_holder.amount.checked_sub(amount_to_withdraw)
            .ok_or(CustomError::Overflow)?;

        // Close PDA if balance is zero
        if balance_holder.amount == 0 {
            let sender_info = ctx.accounts.sender.to_account_info();
            **sender_info.try_borrow_mut_lamports()? += balance_holder.to_account_info().lamports();
            **balance_holder.to_account_info().try_borrow_mut_lamports()? = 0;
            // Note: Anchor will automatically close PDA account when its lamports reach rent-exempt minimum
        }

        Ok(())
    }
}

// PDA state structure
#[account]
pub struct BalanceHolderPDA {
    pub sender: Pubkey,
    pub recipient: Pubkey,
    pub amount: u64,
}

#[derive(Accounts)]
#[instruction(amount_to_deposit: u64)]
pub struct DepositCtx<'info> {
    #[account(mut, signer)]
    pub sender: Signer<'info>, // Use Signer type to satisfy Anchor safety

    /// CHECK: Recipient is only used for PDA derivation; no lamports or data are accessed
    pub recipient: AccountInfo<'info>,

    #[account(
        init_if_needed,
        payer = sender,
        space = 8 + 32 + 32 + 8,
        seeds = [recipient.key().as_ref(), sender.key().as_ref()],
        bump
    )]
    pub balance_holder_pda: Account<'info, BalanceHolderPDA>,

    pub system_program: Program<'info, System>,
}

#[derive(Accounts)]
#[instruction(amount_to_withdraw: u64)]
pub struct WithdrawCtx<'info> {
    #[account(mut, signer)]
    pub recipient: Signer<'info>,

    /// CHECK: Sender is only used for PDA derivation and to receive remaining lamports on PDA closure
    #[account(mut)]
    pub sender: AccountInfo<'info>,

    #[account(
        mut,
        seeds = [recipient.key().as_ref(), sender.key().as_ref()],
        bump,
        has_one = recipient,
        has_one = sender,
    )]
    pub balance_holder_pda: Account<'info, BalanceHolderPDA>,

    pub rent: Sysvar<'info, Rent>,
}

// Custom errors
#[error_code]
pub enum CustomError {
    #[msg("Amount must be greater than zero")]
    InvalidAmount,
    #[msg("Insufficient funds in PDA")]
    InsufficientFunds,
    #[msg("Overflow occurred")]
    Overflow,
}
