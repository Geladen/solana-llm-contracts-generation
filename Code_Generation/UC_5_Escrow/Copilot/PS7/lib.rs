use anchor_lang::prelude::*;
use anchor_lang::system_program::{self, Transfer};

declare_id!("3NVKnJmYvf3a2va2eJZXGAfcJJ13GgZ8fEDBdRXdKDna");

#[program]
pub mod escrow {
    use super::*;

    pub fn initialize(
        ctx: Context<InitializeCtx>,
        amount_in_lamports: u64,
        _escrow_name: String,
    ) -> Result<()> {
        if amount_in_lamports == 0 {
            return err!(EscrowError::ZeroAmount);
        }
        let info = &mut ctx.accounts.escrow_info;
        info.seller = *ctx.accounts.seller.key;
        info.buyer = *ctx.accounts.buyer.key;
        info.amount_in_lamports = amount_in_lamports;
        info.state = State::WaitDeposit;
        info.bump = ctx.bumps.escrow_info;
        Ok(())
    }

    pub fn deposit(ctx: Context<DepositCtx>, escrow_name: String) -> Result<()> {
        // pull AccountInfos before mutably borrowing escrow_info
        let buyer_ai = ctx.accounts.buyer.to_account_info();
        let escrow_ai = ctx.accounts.escrow_info.to_account_info();
        let system_ai = ctx.accounts.system_program.to_account_info();

        let info = &mut ctx.accounts.escrow_info;
        require!(info.state == State::WaitDeposit, EscrowError::InvalidState);

        // build PDA signer seeds
        let seller_key = ctx.accounts.seller.key();
        let buyer_key = ctx.accounts.buyer.key();
        let name_bytes = escrow_name.as_bytes();
        let seeds: &[&[u8]] = &[
            name_bytes,
            seller_key.as_ref(),
            buyer_key.as_ref(),
            &[info.bump],
        ];
        let signer_seeds: &[&[&[u8]]] = &[seeds];

        // transfer the exact amount from buyer → PDA
        let cpi_ctx = CpiContext::new_with_signer(
            system_ai,
            Transfer { from: buyer_ai, to: escrow_ai },
            signer_seeds,
        );
        system_program::transfer(cpi_ctx, info.amount_in_lamports)?;

        info.state = State::WaitRecipient;
        Ok(())
    }

    pub fn pay(ctx: Context<PayCtx>, _escrow_name: String) -> Result<()> {
        let info = &mut ctx.accounts.escrow_info;
        require!(info.state == State::WaitRecipient, EscrowError::InvalidState);

        // Anchor will close the PDA and send all lamports (deposit + rent) to seller
        info.state = State::Closed;
        Ok(())
    }

    pub fn refund(ctx: Context<RefundCtx>, _escrow_name: String) -> Result<()> {
        // pull AccountInfos before mutably borrowing escrow_info
        let escrow_ai = ctx.accounts.escrow_info.to_account_info();
        let buyer_ai = ctx.accounts.buyer.to_account_info();

        let info = &mut ctx.accounts.escrow_info;
        require!(info.state == State::WaitRecipient, EscrowError::InvalidState);

        // refund exactly the deposit portion back to buyer
        let deposit_amt = info.amount_in_lamports;
        let mut escrow_lams = escrow_ai.try_borrow_mut_lamports()?;
        let mut buyer_lams = buyer_ai.try_borrow_mut_lamports()?;

        let buyer_new = (**buyer_lams)
            .checked_add(deposit_amt)
            .ok_or(EscrowError::Overflow)?;
        let escrow_new = (**escrow_lams)
            .checked_sub(deposit_amt)
            .ok_or(EscrowError::Overflow)?;

        **buyer_lams = buyer_new;
        **escrow_lams = escrow_new;

        // Anchor will close the PDA and send leftover rent lamports to seller
        info.state = State::Closed;
        Ok(())
    }
}

#[derive(Accounts)]
#[instruction(amount_in_lamports: u64, escrow_name: String)]
pub struct InitializeCtx<'info> {
    #[account(mut)]
    pub seller: Signer<'info>,

    /// CHECK: only used as a PDA seed for derivation
    pub buyer: UncheckedAccount<'info>,

    #[account(
        init,
        payer = seller,
        space = 8 + EscrowInfo::SIZE,
        seeds = [
            escrow_name.as_bytes(),
            seller.key().as_ref(),
            buyer.key().as_ref(),
        ],
        bump
    )]
    pub escrow_info: Account<'info, EscrowInfo>,

    pub system_program: Program<'info, System>,
}

#[derive(Accounts)]
#[instruction(escrow_name: String)]
pub struct DepositCtx<'info> {
    #[account(mut)]
    pub buyer: Signer<'info>,

    /// CHECK: only used as a PDA seed for derivation
    pub seller: UncheckedAccount<'info>,

    #[account(
        mut,
        seeds = [
            escrow_name.as_bytes(),
            seller.key().as_ref(),
            buyer.key().as_ref(),
        ],
        bump = escrow_info.bump,
        has_one = seller,
        has_one = buyer,
        constraint = escrow_info.state == State::WaitDeposit @ EscrowError::InvalidState
    )]
    pub escrow_info: Account<'info, EscrowInfo>,

    pub system_program: Program<'info, System>,
}

#[derive(Accounts)]
#[instruction(escrow_name: String)]
pub struct PayCtx<'info> {
    #[account(signer)]
    pub buyer: Signer<'info>,

    /// CHECK: only used as a PDA seed for derivation
    #[account(mut)]
    pub seller: UncheckedAccount<'info>,

    #[account(
        mut,
        seeds = [
            escrow_name.as_bytes(),
            seller.key().as_ref(),
            buyer.key().as_ref(),
        ],
        bump = escrow_info.bump,
        has_one = seller,
        has_one = buyer,
        close = seller,
        constraint = escrow_info.state == State::WaitRecipient @ EscrowError::InvalidState
    )]
    pub escrow_info: Account<'info, EscrowInfo>,

    pub system_program: Program<'info, System>,
}

#[derive(Accounts)]
#[instruction(escrow_name: String)]
pub struct RefundCtx<'info> {
    /// Seller must be mutable so the `close = seller` rent‐refund works
    #[account(mut, signer)]
    pub seller: Signer<'info>,

    /// CHECK: only used as a PDA seed for derivation
    #[account(mut)]
    pub buyer: UncheckedAccount<'info>,

    #[account(
        mut,
        seeds = [
            escrow_name.as_bytes(),
            seller.key().as_ref(),
            buyer.key().as_ref(),
        ],
        bump = escrow_info.bump,
        has_one = seller,
        has_one = buyer,
        close = seller,
        constraint = escrow_info.state == State::WaitRecipient @ EscrowError::InvalidState
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
    pub bump: u8,
}

impl EscrowInfo {
    // 32 + 32 + 8 + 1 + 1 = 74
    pub const SIZE: usize = 32 + 32 + 8 + 1 + 1;
}

#[derive(AnchorSerialize, AnchorDeserialize, Clone, Debug, PartialEq, Eq)]
pub enum State {
    WaitDeposit,
    WaitRecipient,
    Closed,
}

#[error_code]
pub enum EscrowError {
    #[msg("Escrow amount must be nonzero.")]
    ZeroAmount,
    #[msg("Operation not allowed in the current state.")]
    InvalidState,
    #[msg("Lamport arithmetic overflow or underflow.")]
    Overflow,
}
