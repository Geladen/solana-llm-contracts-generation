#![allow(unexpected_cfgs)]
use anchor_lang::prelude::*;
use anchor_lang::solana_program::{system_instruction, program::invoke};

declare_id!("ABHkYugQeLho88hFg7iuStareXtNofPVCnoj1K3jDTa4");

#[program]
pub mod simple_transfer_pda {
    use super::*;

    /// First deposit: initializes the PDA
    pub fn deposit_init(ctx: Context<DepositInitCtx>, amount_to_deposit: u64) -> Result<()> {
        if amount_to_deposit == 0 {
            return err!(ErrorCode::InvalidAmount);
        }

        let sender_key = ctx.accounts.sender.key();
        let recipient_key = ctx.accounts.recipient.key();
        let pda = &mut ctx.accounts.balance_holder_pda;

        // Initialize fields
        pda.sender = sender_key;
        pda.recipient = recipient_key;
        pda.amount = amount_to_deposit;

        // Transfer lamports from sender to PDA
        let ix = system_instruction::transfer(
            &sender_key,
            &ctx.accounts.balance_holder_pda.key(),
            amount_to_deposit,
        );
        invoke(
            &ix,
            &[
                ctx.accounts.sender.to_account_info(),
                ctx.accounts.balance_holder_pda.to_account_info(),
                ctx.accounts.system_program.to_account_info(),
            ],
        )?;

        Ok(())
    }

    /// Subsequent deposits: mutates existing PDA
    pub fn deposit(ctx: Context<DepositCtx>, amount: u64) -> Result<()> {
        require!(amount > 0, ErrorCode::InvalidAmount);

        let pda = &mut ctx.accounts.balance_holder_pda;

        // initialize PDA state
        pda.sender = ctx.accounts.sender.key();
        pda.recipient = ctx.accounts.recipient.key();
        pda.amount = pda
            .amount
            .checked_add(amount)
            .ok_or(ErrorCode::AmountOverflow)?;

        // transfer lamports from sender → PDA
        let ix = anchor_lang::solana_program::system_instruction::transfer(
            &ctx.accounts.sender.key(),
            &pda.key(),
            amount,
        );
        anchor_lang::solana_program::program::invoke(
            &ix,
            &[
                ctx.accounts.sender.to_account_info(),
                ctx.accounts.balance_holder_pda.to_account_info(),
            ],
        )?;

        Ok(())
    }


    /// withdraw: recipient can withdraw, supports partial/full withdrawal
    pub fn withdraw(ctx: Context<WithdrawCtx>, amount_to_withdraw: u64) -> Result<()> {
        require!(amount_to_withdraw > 0, ErrorCode::InvalidAmount);

        // === VALIDATION ===
        let pda_key = ctx.accounts.balance_holder_pda.key();
        require!(
            ctx.accounts.balance_holder_pda.recipient == ctx.accounts.recipient.key(),
            ErrorCode::InvalidRecipient
        );
        require!(
            ctx.accounts.balance_holder_pda.sender == ctx.accounts.sender.key(),
            ErrorCode::InvalidSender
        );
        require!(
            ctx.accounts.balance_holder_pda.amount >= amount_to_withdraw,
            ErrorCode::InsufficientFunds
        );

        // === TRANSFER ===
        {
            let pda_info = ctx.accounts.balance_holder_pda.to_account_info();
            let recipient_info = ctx.accounts.recipient.to_account_info();

            **pda_info.try_borrow_mut_lamports()? -= amount_to_withdraw;
            **recipient_info.try_borrow_mut_lamports()? += amount_to_withdraw;
        }

        // === STATE UPDATE (after transfer) ===
        let pda_state = &mut ctx.accounts.balance_holder_pda;
        pda_state.amount = pda_state
            .amount
            .checked_sub(amount_to_withdraw)
            .ok_or(ErrorCode::AmountOverflow)?;

        // === CLOSE PDA IF EMPTY ===
        if pda_state.amount == 0 {
            let pda_info = ctx.accounts.balance_holder_pda.to_account_info();
            let sender_info = ctx.accounts.sender.to_account_info();

            {
                let mut pda_lamports = pda_info.try_borrow_mut_lamports()?;
                let mut sender_lamports = sender_info.try_borrow_mut_lamports()?;

                if **pda_lamports > 0 {
                    **sender_lamports += **pda_lamports;
                    **pda_lamports = 0;
                }
            }

            {
                let mut data = pda_info.try_borrow_mut_data()?;
                for b in data.iter_mut() {
                    *b = 0;
                }
                const CLOSED: [u8; 8] = [255u8; 8];
                data[..8].copy_from_slice(&CLOSED);
            }
        }

        Ok(())
    }


}

/* -----------------------------
   Contexts
------------------------------*/

#[derive(Accounts)]
pub struct DepositInitCtx<'info> {
    #[account(mut)]
    pub sender: Signer<'info>,
    /// CHECK: only used for seeds
    pub recipient: UncheckedAccount<'info>,
    #[account(
        init,
        payer = sender,
        space = 8 + 32 + 32 + 8,
        seeds = [recipient.key().as_ref(), sender.key().as_ref()],
        bump
    )]
    pub balance_holder_pda: Account<'info, BalanceHolderPDA>,
    pub system_program: Program<'info, System>,
}

#[derive(Accounts)]
pub struct DepositCtx<'info> {
    #[account(
        init,                                   // create on first deposit
        payer = sender,                         // sender funds the PDA rent
        space = 8 + 32 + 32 + 8,                // discriminator + sender + recipient + amount
        seeds = [recipient.key().as_ref(), sender.key().as_ref()],
        bump
    )]
    pub balance_holder_pda: Account<'info, BalanceHolderPDA>,

    #[account(mut)]
    pub sender: Signer<'info>,

    /// CHECK: only used as a public key
    pub recipient: UncheckedAccount<'info>,

    pub system_program: Program<'info, System>,
}


#[derive(Accounts)]
pub struct WithdrawCtx<'info> {
    #[account(mut)]
    pub recipient: Signer<'info>,
    /// CHECK: used for seeds and rent return
    #[account(mut)]
    pub sender: UncheckedAccount<'info>,
    #[account(
        mut,
        seeds = [recipient.key().as_ref(), sender.key().as_ref()],
        bump
    )]
    pub balance_holder_pda: Account<'info, BalanceHolderPDA>,
    pub rent: Sysvar<'info, Rent>,
}

/* -----------------------------
   PDA State
------------------------------*/

#[account]
pub struct BalanceHolderPDA {
    pub sender: Pubkey,
    pub recipient: Pubkey,
    pub amount: u64,
}

/* -----------------------------
   Errors
------------------------------*/

#[error_code]
pub enum ErrorCode {
    #[msg("Amount must be greater than zero.")]
    InvalidAmount,
    #[msg("The recipient is not authorized to withdraw from this account.")]
    InvalidRecipient,
    #[msg("The sender does not match the PDA's expected sender.")]
    InvalidSender,
    #[msg("Insufficient funds in PDA account.")]
    InsufficientFunds,
    #[msg("Arithmetic overflow occurred.")]
    AmountOverflow,
    #[msg("PDA account does not match expected derivation.")]
    PDAMismatch,
}

