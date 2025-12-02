use anchor_lang::prelude::*;

declare_id!("8Domuw8u9TjdY5NMpkxcxXqJLumbU6Jv5vkzDW2VCy9n");

#[program]
pub mod simple_transfer {
    use super::*;

    pub fn deposit(ctx: Context<DepositCtx>, amount_to_deposit: u64) -> Result<()> {
        require!(amount_to_deposit > 0, ErrorCode::InvalidAmount);

        // CPI: transfer lamports from sender to PDA
        let sender_ai = ctx.accounts.sender.to_account_info();
        let pda_ai = ctx.accounts.balance_holder_pda.to_account_info();
        let sys_ai = ctx.accounts.system_program.to_account_info();

        let ix = anchor_lang::solana_program::system_instruction::transfer(
            sender_ai.key,
            pda_ai.key,
            amount_to_deposit,
        );
        anchor_lang::solana_program::program::invoke(
            &ix,
            &[
                sender_ai.clone(),
                pda_ai.clone(),
                sys_ai.clone(),
            ],
        )?;

        // Update PDA state
        let pda = &mut ctx.accounts.balance_holder_pda;
        if pda.sender == Pubkey::default() && pda.recipient == Pubkey::default() && pda.amount == 0 {
            pda.sender = ctx.accounts.sender.key();
            pda.recipient = ctx.accounts.recipient.key();
            pda.amount = amount_to_deposit;
        } else {
            require_keys_eq!(pda.sender, ctx.accounts.sender.key(), ErrorCode::InvalidPdaOwner);
            require_keys_eq!(pda.recipient, ctx.accounts.recipient.key(), ErrorCode::InvalidPdaRecipient);
            pda.amount = pda.amount.checked_add(amount_to_deposit).ok_or(ErrorCode::AmountOverflow)?;
        }

        Ok(())
    }

    pub fn withdraw(ctx: Context<WithdrawCtx>, amount_to_withdraw: u64) -> Result<()> {
        require!(amount_to_withdraw > 0, ErrorCode::InvalidAmount);

        // Clone AccountInfos we will need for lamport mutation BEFORE mutable borrow
        let pda_ai = ctx.accounts.balance_holder_pda.to_account_info().clone();
        let recipient_ai = ctx.accounts.recipient.to_account_info().clone();
        let sender_ai = ctx.accounts.sender.to_account_info().clone();

        // Mutably borrow PDA state
        let pda = &mut ctx.accounts.balance_holder_pda;

        // Validate PDA stored keys match provided accounts
        require_keys_eq!(pda.sender, ctx.accounts.sender.key(), ErrorCode::InvalidPdaOwner);
        require_keys_eq!(pda.recipient, ctx.accounts.recipient.key(), ErrorCode::InvalidPdaRecipient);

        // Ensure sufficient logical balance
        require!(pda.amount >= amount_to_withdraw, ErrorCode::InsufficientFunds);

        // Move requested lamports from PDA -> recipient by direct lamport mutation
        {
            let mut pda_lamports_ref = pda_ai.try_borrow_mut_lamports()?;
            let mut recipient_lamports_ref = recipient_ai.try_borrow_mut_lamports()?;

            let pda_balance_before: u64 = **pda_lamports_ref;
            let recipient_balance_before: u64 = **recipient_lamports_ref;

            let pda_balance_after = pda_balance_before
                .checked_sub(amount_to_withdraw)
                .ok_or(ErrorCode::InsufficientFunds)?;
            let recipient_balance_after = recipient_balance_before
                .checked_add(amount_to_withdraw)
                .ok_or(ErrorCode::AmountOverflow)?;

            **pda_lamports_ref = pda_balance_after;
            **recipient_lamports_ref = recipient_balance_after;
        }

        // Update PDA logical balance
        pda.amount = pda.amount.checked_sub(amount_to_withdraw).ok_or(ErrorCode::AmountUnderflow)?;

        // If PDA logical balance reached zero, transfer any remaining lamports (rent) to sender and zero PDA fields
        if pda.amount == 0 {
            let mut pda_lamports_ref = pda_ai.try_borrow_mut_lamports()?;
            let mut sender_lamports_ref = sender_ai.try_borrow_mut_lamports()?;

            let remaining = **pda_lamports_ref;
            if remaining > 0 {
                // move remaining lamports to sender
                let sender_after = (**sender_lamports_ref)
                    .checked_add(remaining)
                    .ok_or(ErrorCode::AmountOverflow)?;
                **sender_lamports_ref = sender_after;
                **pda_lamports_ref = 0;
            }

            // zero PDA stored state so it can't be misused; PDA account remains allocated but empty
            pda.sender = Pubkey::default();
            pda.recipient = Pubkey::default();
            pda.amount = 0;
        }

        Ok(())
    }
}

#[derive(Accounts)]
pub struct DepositCtx<'info> {
    /// Sender must sign and pay for init_if_needed
    #[account(mut, signer)]
    pub sender: Signer<'info>,

    /// CHECK: recipient is used only as a PDA seed and validated against stored PDA fields
    pub recipient: UncheckedAccount<'info>,

    /// PDA with seeds [recipient, sender]
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
pub struct WithdrawCtx<'info> {
    /// Recipient must sign to withdraw funds
    #[account(mut, signer)]
    pub recipient: Signer<'info>,

    /// CHECK: sender is a reference used for PDA derivation and validated against PDA stored sender
    /// must be mutable so we can refund rent when PDA hits zero
    #[account(mut)]
    pub sender: UncheckedAccount<'info>,

    /// PDA account (no close attribute)
    #[account(
        mut,
        seeds = [recipient.key().as_ref(), sender.key().as_ref()],
        bump
    )]
    pub balance_holder_pda: Account<'info, BalanceHolderPDA>,

    /// Rent sysvar included per specification
    pub rent: Sysvar<'info, Rent>,

    pub system_program: Program<'info, System>,
}

#[account]
pub struct BalanceHolderPDA {
    pub sender: Pubkey,
    pub recipient: Pubkey,
    pub amount: u64,
}

#[error_code]
pub enum ErrorCode {
    #[msg("Amount must be greater than zero")]
    InvalidAmount,
    #[msg("PDA stored sender does not match provided sender")]
    InvalidPdaOwner,
    #[msg("PDA stored recipient does not match provided recipient")]
    InvalidPdaRecipient,
    #[msg("Insufficient funds in PDA")]
    InsufficientFunds,
    #[msg("Amount overflow")]
    AmountOverflow,
    #[msg("Amount underflow")]
    AmountUnderflow,
}
