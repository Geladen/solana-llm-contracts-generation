use anchor_lang::prelude::*;

declare_id!("8K1TN4wdhK1X7ZrbGGsoSqegi2d2Cbx537QRKrfpBDPH");

#[program]
pub mod escrow_gpt {
    use super::*;

    pub fn initialize(
        ctx: Context<InitializeCtx>,
        amount_in_lamports: u64,
        _escrow_name: String,
    ) -> Result<()> {
        require!(amount_in_lamports > 0, EscrowError::InvalidAmount);

        let escrow = &mut ctx.accounts.escrow_info;
        escrow.seller = ctx.accounts.seller.key();
        escrow.buyer = ctx.accounts.buyer.key();
        escrow.amount_in_lamports = amount_in_lamports;
        escrow.state = State::WaitDeposit;

        Ok(())
    }

    pub fn deposit(ctx: Context<DepositCtx>, _escrow_name: String) -> Result<()> {
        // Take escrow AccountInfo first
        let escrow_ai = ctx.accounts.escrow_info.to_account_info();

        let escrow = &mut ctx.accounts.escrow_info;
        require!(escrow.state == State::WaitDeposit, EscrowError::InvalidState);
        require!(ctx.accounts.buyer.key() == escrow.buyer, EscrowError::Unauthorized);

        // Transfer lamports from buyer to PDA
        let cpi_ctx = CpiContext::new(
            ctx.accounts.system_program.to_account_info(),
            anchor_lang::system_program::Transfer {
                from: ctx.accounts.buyer.to_account_info(),
                to: escrow_ai,
            },
        );
        anchor_lang::system_program::transfer(cpi_ctx, escrow.amount_in_lamports)?;

        escrow.state = State::WaitRecipient;
        Ok(())
    }

    pub fn pay(ctx: Context<PayCtx>, _escrow_name: String) -> Result<()> {
        let escrow_ai = ctx.accounts.escrow_info.to_account_info();
        let seller_ai = ctx.accounts.seller.to_account_info();

        let escrow = &mut ctx.accounts.escrow_info;
        require!(escrow.state == State::WaitRecipient, EscrowError::InvalidState);
        require!(ctx.accounts.buyer.key() == escrow.buyer, EscrowError::Unauthorized);

        let balance = escrow_ai.lamports();
        **seller_ai.try_borrow_mut_lamports()? += balance;
        **escrow_ai.try_borrow_mut_lamports()? = 0;

        escrow.state = State::Closed;
        Ok(())
    }

    pub fn refund(ctx: Context<RefundCtx>, _escrow_name: String) -> Result<()> {
        let escrow_ai = ctx.accounts.escrow_info.to_account_info();
        let buyer_ai = ctx.accounts.buyer.to_account_info();
        let seller_ai = ctx.accounts.seller.to_account_info();

        let escrow = &mut ctx.accounts.escrow_info;
        require!(escrow.state == State::WaitRecipient, EscrowError::InvalidState);
        require!(ctx.accounts.seller.key() == escrow.seller, EscrowError::Unauthorized);

        let balance = escrow_ai.lamports();
        let rent = Rent::get()?;
        let rent_exempt = rent.minimum_balance(EscrowInfo::LEN);

        let refund_amount = balance.saturating_sub(rent_exempt);
        **buyer_ai.try_borrow_mut_lamports()? += refund_amount;
        **seller_ai.try_borrow_mut_lamports()? += rent_exempt;
        **escrow_ai.try_borrow_mut_lamports()? = 0;

        escrow.state = State::Closed;
        Ok(())
    }
}

/* ===== ACCOUNTS ===== */

#[derive(Accounts)]
#[instruction(escrow_name: String)]
pub struct InitializeCtx<'info> {
    #[account(mut)]
    pub seller: Signer<'info>,

    /// CHECK: used only for PDA derivation
    pub buyer: UncheckedAccount<'info>,

    #[account(
        init,
        payer = seller,
        space = 8 + EscrowInfo::LEN,
        seeds = [escrow_name.as_ref(), seller.key().as_ref(), buyer.key().as_ref()],
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

    /// CHECK: for validation only
    pub seller: UncheckedAccount<'info>,

    #[account(
        mut,
        seeds = [escrow_name.as_ref(), seller.key().as_ref(), buyer.key().as_ref()],
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
    pub seller: SystemAccount<'info>,

    #[account(
        mut,
        seeds = [escrow_name.as_ref(), seller.key().as_ref(), buyer.key().as_ref()],
        bump
    )]
    pub escrow_info: Account<'info, EscrowInfo>,
}

#[derive(Accounts)]
#[instruction(escrow_name: String)]
pub struct RefundCtx<'info> {
    pub seller: Signer<'info>,

    #[account(mut)]
    pub buyer: SystemAccount<'info>,

    #[account(
        mut,
        seeds = [escrow_name.as_ref(), seller.key().as_ref(), buyer.key().as_ref()],
        bump
    )]
    pub escrow_info: Account<'info, EscrowInfo>,
}

/* ===== STATE ===== */

#[account]
pub struct EscrowInfo {
    pub seller: Pubkey,
    pub buyer: Pubkey,
    pub amount_in_lamports: u64,
    pub state: State,
}

impl EscrowInfo {
    pub const LEN: usize = 32 + 32 + 8 + 1; // 73
}


#[derive(AnchorSerialize, AnchorDeserialize, Clone, PartialEq, Eq, Debug)]
pub enum State {
    WaitDeposit,
    WaitRecipient,
    Closed,
}

/* ===== ERRORS ===== */

#[error_code]
pub enum EscrowError {
    #[msg("Invalid escrow state for this operation")]
    InvalidState,
    #[msg("Unauthorized operation for this signer")]
    Unauthorized,
    #[msg("Amount must be greater than zero")]
    InvalidAmount,
}
