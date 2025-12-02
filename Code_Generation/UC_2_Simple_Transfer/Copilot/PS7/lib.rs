use anchor_lang::prelude::*;
use anchor_lang::system_program;

declare_id!("8EVoKJRrNJ4j3SMacZRsY4w6sEVRFLnytBpUXYGVC3cL");

#[program]
pub mod simple_transfer {
    use super::*;

    pub fn deposit(ctx: Context<DepositCtx>, amount_to_deposit: u64) -> Result<()> {
        require!(amount_to_deposit > 0, ErrorCode::ZeroAmount);

        // Transfer lamports from sender to PDA
        let cpi_accounts = system_program::Transfer {
            from: ctx.accounts.sender.to_account_info(),
            to: ctx.accounts.balance_holder_pda.to_account_info(),
        };
        let cpi_ctx = CpiContext::new(ctx.accounts.system_program.to_account_info(), cpi_accounts);
        system_program::transfer(cpi_ctx, amount_to_deposit)?;

        // Initialize or update PDA state
        let pda = &mut ctx.accounts.balance_holder_pda;
        pda.sender = ctx.accounts.sender.key();
        pda.recipient = ctx.accounts.recipient.key();
        pda.amount = pda.amount.checked_add(amount_to_deposit).ok_or(ErrorCode::Overflow)?;

        Ok(())
    }

    pub fn withdraw(ctx: Context<WithdrawCtx>, amount_to_withdraw: u64) -> Result<()> {
        require!(amount_to_withdraw > 0, ErrorCode::ZeroAmount);

        // Read-only checks (no mutable borrows yet)
        require_keys_eq!(
            ctx.accounts.balance_holder_pda.recipient,
            ctx.accounts.recipient.key(),
            ErrorCode::UnauthorizedRecipient
        );
        require_keys_eq!(
            ctx.accounts.balance_holder_pda.sender,
            ctx.accounts.sender.key(),
            ErrorCode::SenderMismatch
        );
        require!(
            ctx.accounts.balance_holder_pda.amount >= amount_to_withdraw,
            ErrorCode::InsufficientFunds
        );

        // Compute new recorded amount
        let new_recorded_amount = ctx
            .accounts
            .balance_holder_pda
            .amount
            .checked_sub(amount_to_withdraw)
            .ok_or(ErrorCode::Overflow)?;

        // Account infos bound to locals so temporaries live long enough
        let pda_ai = ctx.accounts.balance_holder_pda.to_account_info();
        let sender_ai = ctx.accounts.sender.to_account_info();
        let recipient_ai = ctx.accounts.recipient.to_account_info();

        // Rent and space calculation
        let account_space: usize = 8 + 32 + 32 + 8;
        let rent_exempt_min = ctx.accounts.rent.minimum_balance(account_space);

        // Validate PDA has sufficient lamports given rent requirements
        let pda_lamports = **pda_ai.lamports.borrow();
        if new_recorded_amount > 0 {
            let required = amount_to_withdraw
                .checked_add(rent_exempt_min)
                .ok_or(ErrorCode::Overflow)?;
            require!(pda_lamports >= required, ErrorCode::InsufficientFunds);
        } else {
            require!(pda_lamports >= amount_to_withdraw, ErrorCode::InsufficientFunds);
        }

        // Update PDA state BEFORE moving lamports to avoid Anchor close races
        {
            let pda = &mut ctx.accounts.balance_holder_pda;
            pda.amount = new_recorded_amount;
        }

        // Move lamports from PDA -> recipient
        {
            let mut from_lamports = pda_ai.try_borrow_mut_lamports()?;
            let mut to_lamports = recipient_ai.try_borrow_mut_lamports()?;

            let new_from = (*from_lamports)
                .checked_sub(amount_to_withdraw)
                .ok_or(ErrorCode::InsufficientFunds)?;
            let new_to = (*to_lamports)
                .checked_add(amount_to_withdraw)
                .ok_or(ErrorCode::Overflow)?;

            **from_lamports = new_from;
            **to_lamports = new_to;
        }

        // If the internal recorded amount reached zero, explicitly transfer any remaining lamports
        // from the PDA to the sender so tests and clients observe funds returned.
        if new_recorded_amount == 0 {
            // Re-borrow pda lamports to drain remainder to sender
            let mut from_lamports = pda_ai.try_borrow_mut_lamports()?;
            if **from_lamports > 0 {
                let mut to_lamports = sender_ai.try_borrow_mut_lamports()?;
                // read remaining into a u64 (not a RefMut) before arithmetic
                let remaining: u64 = **from_lamports;
                **to_lamports = (**to_lamports)
                    .checked_add(remaining)
                    .ok_or(ErrorCode::Overflow)?;
                **from_lamports = 0;
            }
            // keep PDA data present (sender, recipient, amount == 0) -- explicit close not performed here
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

/// Deposit context
#[derive(Accounts)]
pub struct DepositCtx<'info> {
    /// The sender / owner making the deposit (must sign)
    #[account(mut)]
    pub sender: Signer<'info>,

    /// CHECK: used only as reference for PDA derivation and stored in PDA state
    pub recipient: UncheckedAccount<'info>,

    /// PDA that stores state and lamports. Created if needed.
    #[account(
        init_if_needed,
        payer = sender,
        space = 8 + 32 + 32 + 8,
        seeds = [ recipient.key().as_ref(), sender.key().as_ref() ],
        bump
    )]
    pub balance_holder_pda: Account<'info, BalanceHolderPDA>,

    /// System program required for account creation and transfers
    pub system_program: Program<'info, System>,
}

/// Withdraw context
#[derive(Accounts)]
pub struct WithdrawCtx<'info> {
    /// The recipient who is allowed to withdraw (must sign)
    #[account(mut)]
    pub recipient: Signer<'info>,

    /// CHECK: used as reference for PDA derivation and beneficiary to receive lamports when PDA drained
    #[account(mut)]
    pub sender: UncheckedAccount<'info>,

    /// PDA storing state and lamports. Seeds must match [recipient, sender].
    /// Note: removed `close = sender` to avoid Anchor auto-close races; closure is handled explicitly by program logic if needed.
    #[account(
        mut,
        seeds = [ recipient.key().as_ref(), sender.key().as_ref() ],
        bump,
        constraint = balance_holder_pda.recipient == recipient.key() @ ErrorCode::RecipientMismatch,
        constraint = balance_holder_pda.sender == sender.key() @ ErrorCode::SenderMismatch
    )]
    pub balance_holder_pda: Account<'info, BalanceHolderPDA>,

    /// Rent sysvar included as requested
    pub rent: Sysvar<'info, Rent>,

    /// System program required for transfers
    pub system_program: Program<'info, System>,
}

#[error_code]
pub enum ErrorCode {
    #[msg("Amount must be greater than zero")]
    ZeroAmount,
    #[msg("Insufficient funds")]
    InsufficientFunds,
    #[msg("Sender mismatch")]
    SenderMismatch,
    #[msg("Recipient mismatch")]
    RecipientMismatch,
    #[msg("Unauthorized recipient")]
    UnauthorizedRecipient,
    #[msg("Overflow in arithmetic")]
    Overflow,
    #[msg("Missing PDA bump")]
    MissingBump,
}
