#![allow(unexpected_cfgs)]
use anchor_lang::prelude::*;
use anchor_lang::system_program::{self, Transfer};
use anchor_lang::solana_program::system_program as sys_prog_module;

declare_id!("E3A3T3EAc3Pxo9ndX1Jy39ZUKp5Rn2svGsEmsa1gAodR");

#[program]
pub mod simple_copilot {
    use super::*;

    /// Single-call: create & fund the PDA, store sender/recipient/amount.
    pub fn deposit(ctx: Context<DepositCtx>, amount: u64) -> Result<()> {
        // Reject zero‐amount
        require!(amount > 0, ErrorCode::InvalidAmount);

        // Clone handles before mutably borrowing PDA
        let from_ai = ctx.accounts.sender.to_account_info().clone();
        let to_ai   = ctx.accounts.balance_holder_pda.to_account_info().clone();
        let sys_ai  = ctx.accounts.system_program.to_account_info().clone();

        // CPI: sender → PDA (brand-new, no data yet)
        let cpi_accounts = anchor_lang::system_program::Transfer { from: from_ai, to: to_ai };
        let cpi_ctx      = CpiContext::new(sys_ai, cpi_accounts);
        anchor_lang::system_program::transfer(cpi_ctx, amount)?;

        // Initialize PDA state on‐chain
        let pda = &mut ctx.accounts.balance_holder_pda;
        pda.sender    = ctx.accounts.sender.key();
        pda.recipient = ctx.accounts.recipient.key();
        pda.amount    = amount;

        Ok(())
    }

    /// Withdraw lamports up to `pda.amount`.
    /// - Partial: updates `amount`, leaves PDA alive.
/// - Full:    moves all lamports out, then Anchor auto-closes the PDA (rent → sender).
    pub fn withdraw(ctx: Context<WithdrawCtx>, amount: u64) -> Result<()> {
        // Reject zero‐amount
        require!(amount > 0, ErrorCode::InvalidAmount);

        // Snapshot on-chain balance
        let current = ctx.accounts.balance_holder_pda.amount;
        require!(amount <= current, ErrorCode::InsufficientFunds);

        // Manually move lamports: PDA → recipient
        {
            // Clone AccountInfos to avoid borrow conflicts
            let mut pda_ai  = ctx.accounts.balance_holder_pda.to_account_info().clone();
            let mut recp_ai = ctx.accounts.recipient.to_account_info().clone();

            let mut from_lams = pda_ai.lamports.borrow_mut();   // RefMut<&mut u64>
            let mut to_lams   = recp_ai.lamports.borrow_mut();  // RefMut<&mut u64>

            // double-deref to read/write the inner u64
            let new_from = (**from_lams).checked_sub(amount).unwrap();
            let new_to   = (**to_lams).checked_add(amount).unwrap();

            **from_lams = new_from;
            **to_lams   = new_to;
        }

        // Update PDA’s stored amount
        let remaining = current.checked_sub(amount).unwrap();
        let pda = &mut ctx.accounts.balance_holder_pda;
        pda.amount = remaining;

        // When remaining == 0, Anchor’s `close = sender` will:
        //  - Transfer the PDA’s rent‐reserve lamports into `sender`
        //  - Deallocate the PDA (so getAccount returns null, balance 0)
        Ok(())
    }
}

//
// PDA State
//
#[account]
pub struct BalanceHolderPDA {
    pub sender:    Pubkey,
    pub recipient: Pubkey,
    pub amount:    u64,
}

//
// Error Codes
//
#[error_code]
pub enum ErrorCode {
    #[msg("Amount must be greater than zero")]
    InvalidAmount,

    #[msg("Insufficient funds in PDA")]
    InsufficientFunds,
}

//
// Accounts Contexts
//

/// Deposit in one shot: alloc + fund the PDA
#[derive(Accounts)]
#[instruction(amount: u64)]
pub struct DepositCtx<'info> {
    /// Owner funding the PDA
    #[account(mut, signer)]
    pub sender: Signer<'info>,

    /// Designated recipient (PDA seed only)
    /// CHECK: no on-chain data read/write
    pub recipient: UncheckedAccount<'info>,

    /// PDA storing { sender, recipient, amount }
    #[account(
        init,
        payer = sender,
        space = 8 + 32 + 32 + 8,
        seeds = [recipient.key().as_ref(), sender.key().as_ref()],
        bump
    )]
    pub balance_holder_pda: Account<'info, BalanceHolderPDA>,

    /// System program for the transfer CPI
    pub system_program: Program<'info, System>,
}

/// Withdraw lamports; closes PDA on full drain
#[derive(Accounts)]
#[instruction(amount: u64)]
pub struct WithdrawCtx<'info> {
    /// Recipient pulling funds
    #[account(mut, signer)]
    pub recipient: Signer<'info>,

    /// Sender seeds the PDA and receives rent back when we close it
    #[account(mut)]
    pub sender: SystemAccount<'info>,

    /// PDA holding the balance
    #[account(
        mut,
        seeds = [recipient.key().as_ref(), sender.key().as_ref()],
        bump,
        has_one = recipient,
        has_one = sender,
        close = sender
    )]
    pub balance_holder_pda: Account<'info, BalanceHolderPDA>,

    /// Rent sysvar required by `close = sender`
    pub rent: Sysvar<'info, Rent>,
}
