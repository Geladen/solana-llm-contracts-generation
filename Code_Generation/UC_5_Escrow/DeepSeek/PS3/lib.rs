use anchor_lang::prelude::*;
use anchor_lang::system_program::{transfer, Transfer};

declare_id!("9TKZy4qREbqXk3RhWsY8rfD2bQrncHFmnaBgCLXRFmCq");

#[program]
pub mod escrow_contract {
    use super::*;

    pub fn initialize(
        ctx: Context<InitializeCtx>,
        amount_in_lamports: u64,
        escrow_name: String,
    ) -> Result<()> {
        require!(amount_in_lamports > 0, EscrowError::ZeroAmount);
        
        let escrow_info = &mut ctx.accounts.escrow_info;
        escrow_info.seller = ctx.accounts.seller.key();
        escrow_info.buyer = ctx.accounts.buyer.key();
        escrow_info.amount_in_lamports = amount_in_lamports;
        escrow_info.state = State::WaitDeposit;

        Ok(())
    }

    pub fn deposit(ctx: Context<DepositCtx>, _escrow_name: String) -> Result<()> {
        require!(
            ctx.accounts.escrow_info.state == State::WaitDeposit,
            EscrowError::InvalidState
        );
        require!(
            ctx.accounts.buyer.key() == ctx.accounts.escrow_info.buyer,
            EscrowError::InvalidBuyer
        );

        let transfer_ctx = CpiContext::new(
            ctx.accounts.system_program.to_account_info(),
            Transfer {
                from: ctx.accounts.buyer.to_account_info(),
                to: ctx.accounts.escrow_info.to_account_info(),
            },
        );

        transfer(transfer_ctx, ctx.accounts.escrow_info.amount_in_lamports)?;

        ctx.accounts.escrow_info.state = State::WaitRecipient;

        Ok(())
    }

    pub fn pay(ctx: Context<PayCtx>, _escrow_name: String) -> Result<()> {
        require!(
            ctx.accounts.escrow_info.state == State::WaitRecipient,
            EscrowError::InvalidState
        );

        let escrow_balance = ctx.accounts.escrow_info.to_account_info().lamports();
        let seller_lamports = ctx.accounts.seller.to_account_info().lamports();
        
        **ctx.accounts.escrow_info.to_account_info().try_borrow_mut_lamports()? = 0;
        **ctx.accounts.seller.to_account_info().try_borrow_mut_lamports()? = seller_lamports + escrow_balance;

        ctx.accounts.escrow_info.state = State::Closed;

        Ok(())
    }

    pub fn refund(ctx: Context<RefundCtx>, _escrow_name: String) -> Result<()> {
        require!(
            ctx.accounts.escrow_info.state == State::WaitRecipient,
            EscrowError::InvalidState
        );

        let escrow_balance = ctx.accounts.escrow_info.to_account_info().lamports();
        let rent_exempt_reserve = Rent::get()?.minimum_balance(ctx.accounts.escrow_info.to_account_info().data_len());
        let refund_amount = escrow_balance.checked_sub(rent_exempt_reserve).ok_or(EscrowError::InsufficientFunds)?;
        
        let buyer_lamports = ctx.accounts.buyer.to_account_info().lamports();
        let seller_lamports = ctx.accounts.seller.to_account_info().lamports();
        
        **ctx.accounts.escrow_info.to_account_info().try_borrow_mut_lamports()? = 0;
        **ctx.accounts.buyer.to_account_info().try_borrow_mut_lamports()? = buyer_lamports + refund_amount;
        **ctx.accounts.seller.to_account_info().try_borrow_mut_lamports()? = seller_lamports + rent_exempt_reserve;

        ctx.accounts.escrow_info.state = State::Closed;

        Ok(())
    }
}

#[derive(Accounts)]
#[instruction(amount_in_lamports: u64, escrow_name: String)]
pub struct InitializeCtx<'info> {
    #[account(mut)]
    pub seller: Signer<'info>,
    /// CHECK: Used for PDA derivation only
    pub buyer: AccountInfo<'info>,
    #[account(
        init,
        payer = seller,
        space = 8 + 32 + 32 + 8 + 1,
        seeds = [escrow_name.as_bytes(), seller.key().as_ref(), buyer.key().as_ref()],
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
    /// CHECK: Validated against escrow_info
    pub seller: AccountInfo<'info>,
    #[account(
        mut,
        seeds = [escrow_name.as_bytes(), seller.key().as_ref(), buyer.key().as_ref()],
        bump
    )]
    pub escrow_info: Account<'info, EscrowInfo>,
    pub system_program: Program<'info, System>,
}

#[derive(Accounts)]
#[instruction(escrow_name: String)]
pub struct PayCtx<'info> {
    pub buyer: Signer<'info>,
    #[account(mut)]
    /// CHECK: Receives funds from escrow
    pub seller: AccountInfo<'info>,
    #[account(
        mut,
        seeds = [escrow_name.as_bytes(), seller.key().as_ref(), buyer.key().as_ref()],
        bump
    )]
    pub escrow_info: Account<'info, EscrowInfo>,
}

#[derive(Accounts)]
#[instruction(escrow_name: String)]
pub struct RefundCtx<'info> {
    #[account(mut)] // Added mut here - this was the missing piece
    pub seller: Signer<'info>,
    #[account(mut)]
    /// CHECK: Receives refund from escrow
    pub buyer: AccountInfo<'info>,
    #[account(
        mut,
        seeds = [escrow_name.as_bytes(), seller.key().as_ref(), buyer.key().as_ref()],
        bump
    )]
    pub escrow_info: Account<'info, EscrowInfo>,
}

#[account]
pub struct EscrowInfo {
    pub seller: Pubkey,
    pub buyer: Pubkey,
    pub amount_in_lamports: u64,
    pub state: State,
}

#[derive(AnchorSerialize, AnchorDeserialize, Clone, Copy, PartialEq, Eq)]
pub enum State {
    WaitDeposit,
    WaitRecipient,
    Closed,
}

impl Default for State {
    fn default() -> Self {
        State::WaitDeposit
    }
}

#[error_code]
pub enum EscrowError {
    #[msg("Amount must be greater than zero")]
    ZeroAmount,
    #[msg("Invalid state for operation")]
    InvalidState,
    #[msg("Invalid buyer account")]
    InvalidBuyer,
    #[msg("Insufficient funds for refund")]
    InsufficientFunds,
}