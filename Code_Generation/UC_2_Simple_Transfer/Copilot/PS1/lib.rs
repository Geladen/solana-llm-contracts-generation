use anchor_lang::prelude::*;
use anchor_lang::solana_program::{program::invoke, system_instruction};

declare_id!("J6mYRs1puJbkmF12nUpnNNc6otLVkK4MnGk52yzw8Fut");

#[program]
pub mod simple_transfer {
    use super::*;

    pub fn deposit(ctx: Context<DepositCtx>, amount_to_deposit: u64) -> Result<()> {
        require!(amount_to_deposit > 0, CustomError::ZeroAmount);

        let sender_key = ctx.accounts.sender.key();
        let recipient_key = ctx.accounts.recipient.key();

        // Read PDA stored fields
        let stored_sender = ctx.accounts.balance_holder_pda.sender;
        let stored_recipient = ctx.accounts.balance_holder_pda.recipient;
        let stored_amount = ctx.accounts.balance_holder_pda.amount;

        // Initialize fields if uninitialized
        if stored_sender == Pubkey::default()
            && stored_recipient == Pubkey::default()
            && stored_amount == 0
        {
            let pda = &mut ctx.accounts.balance_holder_pda;
            pda.sender = sender_key;
            pda.recipient = recipient_key;
            pda.amount = 0u64;
        } else {
            require!(stored_sender == sender_key, CustomError::PdaSenderMismatch);
            require!(stored_recipient == recipient_key, CustomError::PdaRecipientMismatch);
        }

        // Transfer lamports sender -> PDA via system CPI (sender signs)
        let pda_key = ctx.accounts.balance_holder_pda.key();
        let ix = system_instruction::transfer(&sender_key, &pda_key, amount_to_deposit);
        invoke(
            &ix,
            &[
                ctx.accounts.sender.to_account_info(),
                ctx.accounts.balance_holder_pda.to_account_info(),
                ctx.accounts.system_program.to_account_info(),
            ],
        )?;

        // Update stored amount
        {
            let pda = &mut ctx.accounts.balance_holder_pda;
            pda.amount = pda
                .amount
                .checked_add(amount_to_deposit)
                .ok_or(CustomError::AmountOverflow)?;
        }

        Ok(())
    }

    pub fn withdraw(ctx: Context<WithdrawCtx>, amount_to_withdraw: u64) -> Result<()> {
        require!(amount_to_withdraw > 0, CustomError::ZeroAmount);

        // Validate PDA state matches provided accounts
        let pda_sender = ctx.accounts.balance_holder_pda.sender;
        let pda_recipient = ctx.accounts.balance_holder_pda.recipient;
        require!(pda_sender == ctx.accounts.sender.key(), CustomError::PdaSenderMismatch);
        require!(pda_recipient == ctx.accounts.recipient.key(), CustomError::PdaRecipientMismatch);

        // Ensure stored amount is sufficient
        require!(
            ctx.accounts.balance_holder_pda.amount >= amount_to_withdraw,
            CustomError::InsufficientFunds
        );

        // Move lamports: debit PDA (program-owned) then credit recipient
        let pda_info = ctx.accounts.balance_holder_pda.to_account_info();
        let recipient_info = ctx.accounts.recipient.to_account_info();

        // Debit PDA lamports
        **pda_info.try_borrow_mut_lamports()? = pda_info
            .lamports()
            .checked_sub(amount_to_withdraw)
            .ok_or(CustomError::InsufficientPdaLamports)?;

        // Credit recipient lamports
        **recipient_info.try_borrow_mut_lamports()? = recipient_info
            .lamports()
            .checked_add(amount_to_withdraw)
            .ok_or(CustomError::AmountOverflow)?;

        // Update stored amount
        {
            let pda = &mut ctx.accounts.balance_holder_pda;
            pda.amount = pda.amount.checked_sub(amount_to_withdraw).ok_or(CustomError::AmountUnderflow)?;
        }

        // If amount reached zero, manually return remaining lamports (rent) to sender and zero PDA data
        if ctx.accounts.balance_holder_pda.amount == 0 {
            let sender_info = ctx.accounts.sender.to_account_info();

            // Transfer remaining lamports from PDA to sender
            let remaining = pda_info.lamports();
            if remaining > 0 {
                **pda_info.try_borrow_mut_lamports()? = 0;
                **sender_info.try_borrow_mut_lamports()? = sender_info
                    .lamports()
                    .checked_add(remaining)
                    .ok_or(CustomError::AmountOverflow)?;
            }

            // Zero the PDA account data to prevent reuse
            {
                let mut data = pda_info.data.borrow_mut();
                for byte in data.iter_mut() {
                    *byte = 0u8;
                }
            }
        }

        Ok(())
    }
}

#[derive(Accounts)]
#[instruction(amount_to_deposit: u64)]
pub struct DepositCtx<'info> {
    /// CHECK: signer and payer; validated by signer attribute and by transfer CPI
    #[account(mut, signer)]
    pub sender: UncheckedAccount<'info>,

    /// CHECK: used only as a Pubkey seed for PDA derivation
    pub recipient: UncheckedAccount<'info>,

    /// PDA storing BalanceHolderPDA state; seeds exactly [recipient.key().as_ref(), sender.key().as_ref()]
    #[account(
        init_if_needed,
        payer = sender,
        space = 8 + BalanceHolderPDA::LEN,
        seeds = [recipient.key().as_ref(), sender.key().as_ref()],
        bump
    )]
    pub balance_holder_pda: Account<'info, BalanceHolderPDA>,

    /// System program required for creating account and transfer CPI
    pub system_program: Program<'info, System>,
}

#[derive(Accounts)]
pub struct WithdrawCtx<'info> {
    /// CHECK: recipient must sign to withdraw
    #[account(mut, signer)]
    pub recipient: UncheckedAccount<'info>,

    /// CHECK: sender receives lamports on PDA closure; validated against PDA.sender
    #[account(mut)]
    pub sender: UncheckedAccount<'info>,

    /// PDA account; mutable, seeds as required (no close attribute now)
    #[account(
        mut,
        seeds = [recipient.key().as_ref(), sender.key().as_ref()],
        bump
    )]
    pub balance_holder_pda: Account<'info, BalanceHolderPDA>,

    /// Rent sysvar included per spec
    pub rent: Sysvar<'info, Rent>,

    /// System program included for completeness; not used in manual lamport moves but present for CPI if needed
    pub system_program: Program<'info, System>,
}

#[account]
pub struct BalanceHolderPDA {
    pub sender: Pubkey,
    pub recipient: Pubkey,
    pub amount: u64,
}

impl BalanceHolderPDA {
    pub const LEN: usize = 32 + 32 + 8;
}

#[error_code]
pub enum CustomError {
    #[msg("Amount cannot be zero")]
    ZeroAmount,
    #[msg("PDA sender mismatch")]
    PdaSenderMismatch,
    #[msg("PDA recipient mismatch")]
    PdaRecipientMismatch,
    #[msg("Insufficient funds in PDA")]
    InsufficientFunds,
    #[msg("Insufficient PDA lamports")]
    InsufficientPdaLamports,
    #[msg("Amount overflow")]
    AmountOverflow,
    #[msg("Amount underflow")]
    AmountUnderflow,
}
