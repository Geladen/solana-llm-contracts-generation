use anchor_lang::prelude::*;
use anchor_lang::system_program;

declare_id!("Cj2EdyUpSUm6uaBu3NokV8oqSKNgpf8Qa2vD1cDWgQxc");

#[program]
pub mod simple_transfer {
    use super::*;

    pub fn deposit(ctx: Context<DepositCtx>, amount_to_deposit: u64) -> Result<()> {
        // Reject zero deposits
        require!(amount_to_deposit > 0, ErrorCode::ZeroDeposit);

        let sender_key = ctx.accounts.sender.key();
        let recipient_key = ctx.accounts.recipient.key();

        // Separate PDA borrow to avoid reborrow conflicts
        {
            let pda = &mut ctx.accounts.balance_holder_pda;

            // Initialize if new
            if pda.sender == Pubkey::default() && pda.recipient == Pubkey::default() {
                pda.sender = sender_key;
                pda.recipient = recipient_key;
                pda.amount = 0;
            } else {
                require_keys_eq!(pda.sender, sender_key, ErrorCode::SenderMismatch);
                require_keys_eq!(pda.recipient, recipient_key, ErrorCode::RecipientMismatch);
            }
        }

        // Clone the PDA's AccountInfo for the CPI (avoids borrow conflict)
        let pda_ai = ctx.accounts.balance_holder_pda.to_account_info();
        let cpi_accounts = system_program::Transfer {
            from: ctx.accounts.sender.to_account_info(),
            to: pda_ai,
        };
        let cpi_program = ctx.accounts.system_program.to_account_info();
        system_program::transfer(CpiContext::new(cpi_program, cpi_accounts), amount_to_deposit)?;

        // Update PDA amount after CPI
        let pda = &mut ctx.accounts.balance_holder_pda;
        pda.amount = pda
            .amount
            .checked_add(amount_to_deposit)
            .ok_or(ErrorCode::ArithmeticOverflow)?;

        Ok(())
    }

    pub fn withdraw(ctx: Context<WithdrawCtx>, amount_to_withdraw: u64) -> Result<()> {
        require!(amount_to_withdraw > 0, ErrorCode::ZeroWithdrawal);

        let sender_key = ctx.accounts.sender.key();
        let recipient_key = ctx.accounts.recipient.key();

        // Clone PDA AccountInfo early (before mutable borrow)
        let pda_info = ctx.accounts.balance_holder_pda.to_account_info();
        let sender_info = ctx.accounts.sender.to_account_info();
        let recipient_info = ctx.accounts.recipient.to_account_info();

        // Now safely mutably borrow PDA data
        let pda = &mut ctx.accounts.balance_holder_pda;

        require_keys_eq!(pda.sender, sender_key, ErrorCode::SenderMismatch);
        require_keys_eq!(pda.recipient, recipient_key, ErrorCode::RecipientMismatch);
        require!(pda.amount >= amount_to_withdraw, ErrorCode::InsufficientRecordedFunds);

        // Move lamports from PDA → recipient
        {
            let mut pda_lamports = pda_info.try_borrow_mut_lamports()?;
            let mut recipient_lamports = recipient_info.try_borrow_mut_lamports()?;

            require!(
                **pda_lamports >= amount_to_withdraw,
                ErrorCode::InsufficientPdaLamports
            );

            **pda_lamports -= amount_to_withdraw;
            **recipient_lamports += amount_to_withdraw;
        }

        // Update logical balance
        pda.amount = pda
            .amount
            .checked_sub(amount_to_withdraw)
            .ok_or(ErrorCode::ArithmeticOverflow)?;

        // If balance reaches zero → close PDA
        if pda.amount == 0 {
            let mut pda_lamports = pda_info.try_borrow_mut_lamports()?;
            let mut sender_lamports = sender_info.try_borrow_mut_lamports()?;

            let remaining = **pda_lamports;
            if remaining > 0 {
                **sender_lamports = sender_lamports
                    .checked_add(remaining)
                    .ok_or(ErrorCode::ArithmeticOverflow)?;
                **pda_lamports = 0;
            }

            pda_info.resize(0)?;
            pda_info.assign(&system_program::ID);
        }

        Ok(())
    }
}

/// PDA state
#[account]
pub struct BalanceHolderPDA {
    pub sender: Pubkey,
    pub recipient: Pubkey,
    pub amount: u64,
}

/// Deposit instruction accounts
#[derive(Accounts)]
pub struct DepositCtx<'info> {
    #[account(mut)]
    pub sender: Signer<'info>,

    /// CHECK: only used as seed
    pub recipient: UncheckedAccount<'info>,

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

/// Withdraw instruction accounts
#[derive(Accounts)]
pub struct WithdrawCtx<'info> {
    #[account(mut)]
    pub recipient: Signer<'info>,

    /// CHECK: only used as seed
    #[account(mut)] // ✅ must be mutable, since we refund rent lamports on PDA close
    pub sender: UncheckedAccount<'info>,

    #[account(
        mut,
        seeds = [recipient.key().as_ref(), sender.key().as_ref()],
        bump
    )]
    pub balance_holder_pda: Account<'info, BalanceHolderPDA>,

    pub rent: Sysvar<'info, Rent>,
}

#[error_code]
pub enum ErrorCode {
    #[msg("Deposit amount must be non-zero")]
    ZeroDeposit,
    #[msg("Withdrawal amount must be non-zero")]
    ZeroWithdrawal,
    #[msg("Recorded PDA balance is insufficient")]
    InsufficientRecordedFunds,
    #[msg("PDA has insufficient lamports")]
    InsufficientPdaLamports,
    #[msg("Sender mismatch")]
    SenderMismatch,
    #[msg("Recipient mismatch")]
    RecipientMismatch,
    #[msg("Arithmetic overflow/underflow")]
    ArithmeticOverflow,
}
