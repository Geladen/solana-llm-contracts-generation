#![allow(unexpected_cfgs)]
use anchor_lang::prelude::*;
use anchor_lang::system_program;

declare_id!("DLykriTeJXQ73Vr1DxTUAvm5AfQVFHAkFHvMy3ay3VWy");

#[program]
pub mod simple_copilot {
    use super::*;

    pub fn deposit(ctx: Context<Deposit>, amount: u64) -> Result<()> {
        // 1) Reject zero
        if amount == 0 {
            return Err(ErrorCode::InvalidAmount.into());
        }

        // 2) Initialize the PDA’s state
        let pda = &mut ctx.accounts.balance_holder_pda;
        pda.sender = ctx.accounts.sender.key();
        pda.recipient = ctx.accounts.recipient.key();
        pda.amount = amount;

        // 3) Transfer lamports from sender → PDA (on top of the rent‐exempt balance)
        let cpi = CpiContext::new(
            ctx.accounts.system_program.to_account_info(),
            anchor_lang::system_program::Transfer {
                from: ctx.accounts.sender.to_account_info(),
                to: pda.to_account_info(),
            },
        );
        anchor_lang::system_program::transfer(cpi, amount)?;

        Ok(())
    }

    pub fn withdraw(ctx: Context<Withdraw>, amount: u64) -> Result<()> {
        // 1) Reject zero
        if amount == 0 {
            return Err(ErrorCode::InvalidAmount.into());
        }

        let pda = &mut ctx.accounts.balance_holder_pda;

        // 2) Only the designated recipient may withdraw
        if ctx.accounts.recipient.key() != pda.recipient {
            return Err(ErrorCode::InvalidRecipient.into());
        }

        // 3) Must have enough funds
        if amount > pda.amount {
            return Err(ErrorCode::InsufficientFunds.into());
        }

        // 4) Move lamports out
        **pda.to_account_info().try_borrow_mut_lamports()? -= amount;
        **ctx
            .accounts
            .recipient
            .to_account_info()
            .try_borrow_mut_lamports()? += amount;

        // 5) Update on‐chain state; when `amount == 0`, `close = sender` fires automatically
        pda.amount = pda.amount.checked_sub(amount).unwrap();
        Ok(())
    }
}

//
// Deposit Context
//
#[derive(Accounts)]
pub struct Deposit<'info> {
    /// PDA storing (sender, recipient, amount)
    /// - init:   create it now (rent‐exempt lamports from `sender`)
    /// - payer:  `sender`
    /// - seeds:  [recipient, sender]
    /// - bump:   auto‐derived
    /// - space:  8 (disc) + 32 + 32 + 8
    #[account(
        init,
        payer = sender,
        seeds = [recipient.key().as_ref(), sender.key().as_ref()],
        bump,
        space = 8 + 32 + 32 + 8
    )]
    pub balance_holder_pda: Account<'info, BalanceHolderPda>,

    /// The wallet that funds rent + deposit, must sign.
    #[account(mut)]
    pub sender: Signer<'info>,

    /// The future recipient’s Pubkey (only used for PDA seeds).
    /// CHECK: no data is ever read from or written to this account.
    pub recipient: UncheckedAccount<'info>,

    /// System program for CPI.
    pub system_program: Program<'info, System>,

    /// Rent sysvar for Anchor’s `init` under the hood.
    pub rent: Sysvar<'info, Rent>,
}

//
// Withdraw Context
//
#[derive(Accounts)]
pub struct Withdraw<'info> {
    /// The designated recipient withdrawing funds, must sign.
    #[account(mut)]
    pub recipient: Signer<'info>,

    /// The original sender Pubkey (used for PDA seeds and rent refund on close).
    /// CHECK: no data is ever read from or written to this account.
    pub sender: UncheckedAccount<'info>,

    /// PDA must:
    /// - exist with the same seeds & bump  
    /// - `has_one = recipient` and `has_one = sender` guard fields  
    /// - `close = sender` when drained
    #[account(
        mut,
        seeds = [recipient.key().as_ref(), sender.key().as_ref()],
        bump,
        has_one = recipient,
        has_one = sender,
        close = sender
    )]
    pub balance_holder_pda: Account<'info, BalanceHolderPda>,

    /// System program for the underlying close CPI.
    pub system_program: Program<'info, System>,
}

//
// PDA State
//
#[account]
pub struct BalanceHolderPda {
    pub sender:    Pubkey,
    pub recipient: Pubkey,
    pub amount:    u64,
}

//
// Errors
//
#[error_code]
pub enum ErrorCode {
    #[msg("Amount must be greater than zero")]
    InvalidAmount,

    #[msg("Caller is not the designated recipient")]
    InvalidRecipient,

    #[msg("Insufficient funds in PDA")]
    InsufficientFunds,
}
