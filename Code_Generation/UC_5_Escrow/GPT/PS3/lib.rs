use anchor_lang::prelude::*;
use anchor_lang::system_program::{self, Transfer};

declare_id!("4hb4wXnGRjWtV5moDpbhLrG3qvWSBfmZftuAgjvoJNTm");

#[program]
pub mod escrow {
    use super::*;

    pub fn initialize(
        ctx: Context<Initialize>,
        escrow_name: String,
        amount_in_lamports: u64,
    ) -> Result<()> {
        let escrow = &mut ctx.accounts.escrow_info;
        escrow.seller = *ctx.accounts.seller.key;
        escrow.buyer = *ctx.accounts.buyer.key;
        escrow.amount_in_lamports = amount_in_lamports;
        escrow.state = State::WaitDeposit;
        Ok(())
    }

    pub fn deposit(ctx: Context<Deposit>, _escrow_name: String) -> Result<()> {
        require!(
            ctx.accounts.escrow_info.state == State::WaitDeposit,
            EscrowError::InvalidState
        );

        let amount = ctx.accounts.escrow_info.amount_in_lamports;

        let cpi_ctx = CpiContext::new(
            ctx.accounts.system_program.to_account_info(),
            Transfer {
                from: ctx.accounts.buyer.to_account_info(),
                to: ctx.accounts.escrow_info.to_account_info(),
            },
        );

        system_program::transfer(cpi_ctx, amount)?;

        let escrow = &mut ctx.accounts.escrow_info;
        escrow.state = State::WaitRecipient;
        Ok(())
    }

    pub fn pay(ctx: Context<Pay>, escrow_name: String) -> Result<()> {
        require!(
            ctx.accounts.escrow_info.state == State::WaitRecipient,
            EscrowError::InvalidState
        );

        let amount = ctx.accounts.escrow_info.amount_in_lamports;

        let escrow_seeds: &[&[u8]] = &[
            escrow_name.as_bytes(),
            ctx.accounts.escrow_info.seller.as_ref(),
            ctx.accounts.escrow_info.buyer.as_ref(),
            &[ctx.bumps.escrow_info],
        ];
        let signer_seeds = &[escrow_seeds];

        let cpi_ctx = CpiContext::new_with_signer(
            ctx.accounts.system_program.to_account_info(),
            Transfer {
                from: ctx.accounts.escrow_info.to_account_info(),
                to: ctx.accounts.seller.to_account_info(),
            },
            signer_seeds,
        );

        system_program::transfer(cpi_ctx, amount)?;

        let escrow = &mut ctx.accounts.escrow_info;
        escrow.state = State::Closed;
        Ok(())
    }

    pub fn refund(ctx: Context<Refund>, escrow_name: String) -> Result<()> {
        require!(
            ctx.accounts.escrow_info.state == State::WaitRecipient,
            EscrowError::InvalidState
        );

        let amount = ctx.accounts.escrow_info.amount_in_lamports;

        let escrow_seeds: &[&[u8]] = &[
            escrow_name.as_bytes(),
            ctx.accounts.escrow_info.seller.as_ref(),
            ctx.accounts.escrow_info.buyer.as_ref(),
            &[ctx.bumps.escrow_info],
        ];
        let signer_seeds = &[escrow_seeds];

        let cpi_ctx = CpiContext::new_with_signer(
            ctx.accounts.system_program.to_account_info(),
            Transfer {
                from: ctx.accounts.escrow_info.to_account_info(),
                to: ctx.accounts.buyer.to_account_info(),
            },
            signer_seeds,
        );

        system_program::transfer(cpi_ctx, amount)?;

        let escrow = &mut ctx.accounts.escrow_info;
        escrow.state = State::Closed;
        Ok(())
    }
}

#[derive(Accounts)]
#[instruction(escrow_name: String)]
pub struct Initialize<'info> {
    #[account(mut)]
    pub seller: Signer<'info>,

    /// CHECK: This is only used as a PDA seed and for validation in business logic.
    pub buyer: UncheckedAccount<'info>,

    #[account(
        init,
        payer = seller,
        space = 8 + EscrowInfo::SIZE,
        seeds = [escrow_name.as_bytes(), seller.key().as_ref(), buyer.key().as_ref()],
        bump,
    )]
    pub escrow_info: Account<'info, EscrowInfo>,

    pub system_program: Program<'info, System>,
}



#[derive(Accounts)]
#[instruction(escrow_name: String)]
pub struct Deposit<'info> {
    #[account(mut)]
    pub buyer: Signer<'info>,

    /// CHECK: only validated by business logic
    pub seller: UncheckedAccount<'info>,

    #[account(
        mut,
        seeds = [escrow_name.as_bytes(), escrow_info.seller.as_ref(), escrow_info.buyer.as_ref()],
        bump,
    )]
    pub escrow_info: Account<'info, EscrowInfo>,

    pub system_program: Program<'info, System>,
}

#[derive(Accounts)]
#[instruction(escrow_name: String)]
pub struct Pay<'info> {
    #[account(mut)]
    pub seller: Signer<'info>,

    /// CHECK: only validated by business logic
    pub buyer: UncheckedAccount<'info>,

    #[account(
        mut,
        seeds = [escrow_name.as_bytes(), escrow_info.seller.as_ref(), escrow_info.buyer.as_ref()],
        bump,
    )]
    pub escrow_info: Account<'info, EscrowInfo>,

    pub system_program: Program<'info, System>,
}

#[derive(Accounts)]
#[instruction(escrow_name: String)]
pub struct Refund<'info> {
    #[account(mut)]
    pub seller: Signer<'info>,

    /// CHECK: Only used for validation, no data read
    #[account(mut)]
    pub buyer: UncheckedAccount<'info>,

    #[account(
        mut,
        seeds = [escrow_name.as_bytes(), escrow_info.seller.as_ref(), escrow_info.buyer.as_ref()],
        bump,
    )]
    pub escrow_info: Account<'info, EscrowInfo>,

    pub system_program: Program<'info, System>,
}

#[account]
pub struct EscrowInfo {
    pub seller: Pubkey,
    pub buyer: Pubkey,
    pub amount_in_lamports: u64,
    pub state: State,
}

impl EscrowInfo {
    pub const SIZE: usize = 32 + 32 + 8 + 1;
}

#[derive(AnchorSerialize, AnchorDeserialize, Clone, PartialEq, Eq)]
pub enum State {
    WaitDeposit,
    WaitRecipient,
    Closed,
}

#[error_code]
pub enum EscrowError {
    #[msg("The escrow state is invalid for this operation")]
    InvalidState,
    #[msg("Invalid PDA seeds used")]
    InvalidPda,
}
