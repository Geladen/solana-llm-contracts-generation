#![allow(unexpected_cfgs)]
use anchor_lang::prelude::*;
use anchor_lang::system_program;

declare_id!("7Esh6BPKDq34MyeZXYYaSdGeAWK4v6vS719VZYcJ9Wn9");

#[program]
pub mod simple_transfer {
    use super::*;

    pub fn deposit(ctx: Context<Deposit>, amount: u64) -> Result<()> {
        require!(amount > 0, ErrorCode::ZeroAmount);

        let pda = &mut ctx.accounts.balance_holder_pda;
        if pda.amount == 0 {
            pda.sender    = ctx.accounts.sender.key();
            pda.recipient = ctx.accounts.recipient.key();
        } else {
            require!(
                pda.sender == ctx.accounts.sender.key() &&
                pda.recipient == ctx.accounts.recipient.key(),
                ErrorCode::InvalidPDA
            );
        }

        // bump on‐chain amount
        pda.amount = pda.amount.checked_add(amount).ok_or(ErrorCode::Overflow)?;

        // transfer lamports from sender → PDA via CPI (sender owned by system)
        let cpi = CpiContext::new(
            ctx.accounts.system_program.to_account_info(),
            anchor_lang::system_program::Transfer {
                from: ctx.accounts.sender.to_account_info(),
                to:   pda.to_account_info(),
            },
        );
        anchor_lang::system_program::transfer(cpi, amount)?;
        Ok(())
    }

    pub fn withdraw(ctx: Context<Withdraw>, amount: u64) -> Result<()> {
        require!(amount > 0, ErrorCode::ZeroAmount);

        let pda_acc = &mut ctx.accounts.balance_holder_pda;

        // only the designated recipient may withdraw
        require!(
            ctx.accounts.recipient.key() == pda_acc.recipient,
            ErrorCode::Unauthorized
        );
        require!(amount <= pda_acc.amount, ErrorCode::InsufficientFunds);

        // === DIRECT LAMPORTS MOVE ===
        let pda_info       = pda_acc.to_account_info();
        let recipient_info = ctx.accounts.recipient.to_account_info();

        // debit the PDA
        **pda_info.lamports.borrow_mut() = 
            pda_info.lamports().checked_sub(amount).unwrap();
        // credit the recipient
        **recipient_info.lamports.borrow_mut() += amount;

        // update on‐chain balance
        pda_acc.amount = pda_acc.amount.checked_sub(amount).ok_or(ErrorCode::Overflow)?;

        // if emptied, drain remaining lamports AND zero‐out data
        if pda_acc.amount == 0 {
            // any rent‐exempt remainder
            let rem = **pda_info.lamports.borrow();
            **pda_info.lamports.borrow_mut() = 0;
            **ctx.accounts.sender.to_account_info().lamports.borrow_mut() += rem;

            // zero the account data so it’s effectively closed
            let data = &mut *pda_info.data.borrow_mut();
            data.fill(0);
        }

        Ok(())
    }
}

#[derive(Accounts)]
#[instruction(amount: u64)]
pub struct Deposit<'info> {
    #[account(mut, signer)]
    pub sender: SystemAccount<'info>,

    /// CHECK: only used as PDA seed
    pub recipient: UncheckedAccount<'info>,

    /// init on first call, seeds=[recipient, sender]
    #[account(
        init,
        payer = sender,
        space = BalanceHolderPDA::LEN,
        seeds = [recipient.key().as_ref(), sender.key().as_ref()],
        bump
    )]
    pub balance_holder_pda: Account<'info, BalanceHolderPDA>,

    pub system_program: Program<'info, System>,
}

#[derive(Accounts)]
#[instruction(amount: u64)]
pub struct Withdraw<'info> {
    #[account(mut, signer)]
    pub recipient: SystemAccount<'info>,

    #[account(mut)]
    pub sender: SystemAccount<'info>,

    #[account(
        mut,
        seeds = [recipient.key().as_ref(), sender.key().as_ref()],
        bump
    )]
    pub balance_holder_pda: Account<'info, BalanceHolderPDA>,

    pub system_program: Program<'info, System>,
}

#[account]
pub struct BalanceHolderPDA {
    pub sender:    Pubkey,
    pub recipient: Pubkey,
    pub amount:    u64,
}

impl BalanceHolderPDA {
    pub const LEN: usize = 8 + 32 + 32 + 8;
}

#[error_code]
pub enum ErrorCode {
    #[msg("Amount must be greater than zero.")]
    ZeroAmount,

    #[msg("Only the designated recipient may withdraw.")]
    Unauthorized,

    #[msg("Insufficient funds in PDA.")]
    InsufficientFunds,

    #[msg("Overflow or underflow on balance.")]
    Overflow,

    #[msg("PDA seeds or state mismatch.")]
    InvalidPDA,
}
