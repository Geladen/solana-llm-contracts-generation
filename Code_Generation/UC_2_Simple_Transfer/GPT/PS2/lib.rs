#![allow(unexpected_cfgs)]
use anchor_lang::prelude::*;
use anchor_lang::system_program;
use anchor_lang::AccountsClose;

declare_id!("E8f81TCHVzNaVkqxeVa3S6Znn5PEfTECKZoSF8EjnaHv");

#[program]
pub mod simple_gpt {
    use super::*;

    pub fn deposit(ctx: Context<DepositCtx>, amount: u64) -> Result<()> {
        // Transfer lamports: sender → PDA
        let cpi_accounts = system_program::Transfer {
            from: ctx.accounts.sender.to_account_info(),
            to: ctx.accounts.balance_holder_pda.to_account_info(),
        };
        let cpi_ctx = CpiContext::new(ctx.accounts.system_program.to_account_info(), cpi_accounts);
        system_program::transfer(cpi_ctx, amount)?;

        // Update PDA state
        let balance_holder = &mut ctx.accounts.balance_holder_pda;
        balance_holder.sender = ctx.accounts.sender.key();
        balance_holder.recipient = ctx.accounts.recipient.key();
        balance_holder.amount += amount;

        Ok(())
    }



    pub fn withdraw(ctx: Context<WithdrawCtx>, amount: u64) -> Result<()> {
        let balance_holder_info = ctx.accounts.balance_holder_pda.to_account_info();
        let recipient_info = ctx.accounts.recipient.to_account_info();

        // Ensure enough funds
        require!(
            ctx.accounts.balance_holder_pda.amount >= amount,
            ErrorCode::InsufficientFunds
        );

        // Transfer lamports from PDA to recipient
        **balance_holder_info.try_borrow_mut_lamports()? -= amount;
        **recipient_info.try_borrow_mut_lamports()? += amount;

        // Decrease tracked balance
        ctx.accounts.balance_holder_pda.amount = ctx.accounts
            .balance_holder_pda
            .amount
            .checked_sub(amount)
            .unwrap();

        // ✅ Only close when balance reaches zero
        if ctx.accounts.balance_holder_pda.amount == 0 {
            // refund rent to sender
            return ctx
                .accounts
                .balance_holder_pda
                .close(ctx.accounts.sender.to_account_info());
        }

        Ok(())
    }

}

#[derive(Accounts)]
pub struct DepositCtx<'info> {
    #[account(
        init,
        payer = sender,
        space = BalanceHolderPDA::LEN,
        seeds = [recipient.key().as_ref(), sender.key().as_ref()],
        bump
    )]
    pub balance_holder_pda: Account<'info, BalanceHolderPDA>,

    #[account(mut)]
    pub sender: Signer<'info>,

    /// CHECK: Only used for lamport transfers
    pub recipient: UncheckedAccount<'info>,

    pub system_program: Program<'info, System>,
}

#[derive(Accounts)]
pub struct WithdrawCtx<'info> {
    #[account(
        mut,
        seeds = [balance_holder_pda.recipient.as_ref(), balance_holder_pda.sender.as_ref()],
        bump,
        close = sender   // PDA closes only when amount == 0
    )]
    pub balance_holder_pda: Account<'info, BalanceHolderPDA>,

    #[account(mut)]
    pub recipient: Signer<'info>,

    #[account(mut)]
    pub sender: SystemAccount<'info>,

    pub rent: Sysvar<'info, Rent>,
}

#[account]
pub struct BalanceHolderPDA {
    pub sender: Pubkey,
    pub recipient: Pubkey,
    pub amount: u64,
}

impl BalanceHolderPDA {
    pub const LEN: usize = 8   // discriminator
        + 32  // sender
        + 32  // recipient
        + 8;  // amount
}


#[error_code]
pub enum ErrorCode {
    #[msg("Amount overflow")]
    AmountOverflow,
    #[msg("Insufficient balance")]
    InsufficientBalance,
    #[msg("The PDA account does not have enough funds to withdraw.")]
    InsufficientFunds,
}
