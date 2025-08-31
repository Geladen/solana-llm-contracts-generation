#![allow(unexpected_cfgs)]
use anchor_lang::prelude::*;
use anchor_lang::solana_program::{program::invoke_signed, system_instruction};

declare_id!("6gnE2sbs9nyGLzw2MzZnHCwvD5YTu1LcoypRkxfqKZc2");

#[program]
pub mod simple_transfer {
    use super::*;

    pub fn deposit(ctx: Context<DepositCtx>, amount_to_deposit: u64) -> Result<()> {
        require!(amount_to_deposit > 0, ErrorCode::ZeroAmount);

        // Transfer SOL from sender → PDA
        anchor_lang::system_program::transfer(
            CpiContext::new(
                ctx.accounts.system_program.to_account_info(),
                anchor_lang::system_program::Transfer {
                    from: ctx.accounts.sender.to_account_info(),
                    to: ctx.accounts.balance_holder_pda.to_account_info(),
                },
            ),
            amount_to_deposit,
        )?;

        let bal = &mut ctx.accounts.balance_holder_pda;

        // First deposit after init
        if bal.amount == 0 && bal.sender == Pubkey::default() && bal.recipient == Pubkey::default()
        {
            bal.sender = ctx.accounts.sender.key();
            bal.recipient = ctx.accounts.recipient.key();
            bal.amount = amount_to_deposit;
        } else {
            require_keys_eq!(bal.sender, ctx.accounts.sender.key(), ErrorCode::SenderRecipientMismatch);
            require_keys_eq!(bal.recipient, ctx.accounts.recipient.key(), ErrorCode::SenderRecipientMismatch);

            bal.amount = bal
                .amount
                .checked_add(amount_to_deposit)
                .ok_or(ErrorCode::NumericalOverflow)?;
        }

        Ok(())
    }

pub fn withdraw(ctx: Context<WithdrawCtx>, amount_to_withdraw: u64) -> Result<()> {
    require!(amount_to_withdraw > 0, ErrorCode::ZeroAmount);

    // structured account
    let bal = &mut ctx.accounts.balance_holder_pda;

    // validate relationship
    require_keys_eq!(bal.sender, ctx.accounts.sender.key(), ErrorCode::SenderRecipientMismatch);
    require_keys_eq!(bal.recipient, ctx.accounts.recipient.key(), ErrorCode::SenderRecipientMismatch);

    // validate funds
    require!(bal.amount >= amount_to_withdraw, ErrorCode::InsufficientFunds);

    // ---- Transfer lamports manually ----
    **bal.to_account_info().try_borrow_mut_lamports()? -= amount_to_withdraw;
    **ctx.accounts.recipient.to_account_info().try_borrow_mut_lamports()? += amount_to_withdraw;

    // update tracked balance
    bal.amount = bal
        .amount
        .checked_sub(amount_to_withdraw)
        .ok_or(ErrorCode::NumericalUnderflow)?;

    // ---- Close PDA if balance is zero ----
    if bal.amount == 0 {
        // return rent-exempt lamports back to sender
        let lamports = **bal.to_account_info().lamports.borrow();
        **ctx.accounts.sender.to_account_info().try_borrow_mut_lamports()? += lamports;
        **bal.to_account_info().try_borrow_mut_lamports()? = 0;

        // mark account as closed (zero its data)
        let bal_info = bal.to_account_info();
        let mut data = bal_info.try_borrow_mut_data()?;
        data.fill(0);
    }

    Ok(())
}



}

#[account]
pub struct BalanceHolderPDA {
    pub sender: Pubkey,
    pub recipient: Pubkey,
    pub amount: u64,
}

// ----------------- Contexts -----------------

#[derive(Accounts)]
pub struct DepositCtx<'info> {
    #[account(mut)]
    pub sender: Signer<'info>,

    /// CHECK: only used for PDA seeds
    pub recipient: AccountInfo<'info>,

    #[account(
        init,
        payer = sender,
        space = 8 + 32 + 32 + 8,
        seeds = [ recipient.key.as_ref(), sender.key.as_ref() ],
        bump
    )]
    pub balance_holder_pda: Account<'info, BalanceHolderPDA>,

    pub system_program: Program<'info, System>,
}

#[derive(Accounts)]
pub struct WithdrawCtx <'info> {
    #[account(mut)]
    pub recipient: Signer<'info>,    // needs `mut` since lamports are credited
    #[account(mut)]
    /// CHECK: validated via PDA constraint
    pub sender: UncheckedAccount<'info>, // must be `mut` since lamports may be refunded on close
    #[account(
        mut,
        seeds = [recipient.key().as_ref(), sender.key().as_ref()],
        bump,
        constraint = balance_holder_pda.sender == sender.key(),
        constraint = balance_holder_pda.recipient == recipient.key(),
    )]
    pub balance_holder_pda: Account<'info, BalanceHolderPDA>,
    pub rent: Sysvar<'info, Rent>,
}

// ----------------- Errors -----------------

#[error_code]
pub enum ErrorCode {
    #[msg("Amount cannot be zero")]
    ZeroAmount,
    #[msg("Sender and recipient mismatch")]
    SenderRecipientMismatch,
    #[msg("Numerical overflow")]
    NumericalOverflow,
    #[msg("Numerical underflow")]
    NumericalUnderflow,
    #[msg("Insufficient funds in PDA")]
    InsufficientFunds,
}
