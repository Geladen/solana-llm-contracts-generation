use anchor_lang::prelude::*;
use anchor_lang::system_program::{self, Transfer};

declare_id!("Ftb5qqEvToAbNUAV9g8BVq61M2fabLTcebuCE1gvjwCo");

#[program]
pub mod escrow_program {
    use super::*;

    /// Seller initializes the escrow PDA with amount & WaitDeposit state.
    pub fn initialize(
        ctx: Context<InitializeCtx>,
        amount_in_lamports: u64,
        _escrow_name: String,
    ) -> Result<()> {
        require!(amount_in_lamports > 0, EscrowError::ZeroAmount);

        let escrow = &mut ctx.accounts.escrow_info;
        escrow.seller = *ctx.accounts.seller.key;
        escrow.buyer = *ctx.accounts.buyer.key;
        escrow.amount_in_lamports = amount_in_lamports;
        escrow.state = State::WaitDeposit;
        escrow.bump = ctx.bumps.escrow_info;
        Ok(())
    }

    /// Buyer deposits exactly `amount_in_lamports` into the PDA.
    pub fn deposit(
        ctx: Context<DepositCtx>,
        _escrow_name: String,
    ) -> Result<()> {
        // validate
        let escrow = &ctx.accounts.escrow_info;
        require!(escrow.state == State::WaitDeposit,    EscrowError::InvalidState);
        require!(escrow.buyer == *ctx.accounts.buyer.key, EscrowError::Unauthorized);

        // CPI: buyer → PDA
        let amt = escrow.amount_in_lamports;
        let cpi_accounts = Transfer {
            from: ctx.accounts.buyer.to_account_info(),
            to:   ctx.accounts.escrow_info.to_account_info(),
        };
        let cpi_ctx = CpiContext::new(
            ctx.accounts.system_program.to_account_info(),
            cpi_accounts,
        );
        system_program::transfer(cpi_ctx, amt)?;

        // update state
        ctx.accounts.escrow_info.state = State::WaitRecipient;
        Ok(())
    }

    /// Buyer calls pay: closes the PDA, draining _all_ lamports (deposit + rent) to seller.
    pub fn pay(
        ctx: Context<PayCtx>,
        _escrow_name: String,
    ) -> Result<()> {
        let escrow = &ctx.accounts.escrow_info;
        require!(escrow.state == State::WaitRecipient,    EscrowError::InvalidState);
        require!(escrow.buyer == *ctx.accounts.buyer.key, EscrowError::Unauthorized);

        // `close = seller` on the PDA account attribute does the drain + close.
        Ok(())
    }

    /// Seller calls refund: manually refund the deposited lamports back to buyer,
    /// then `close = seller` returns rent back to seller.
    pub fn refund(
        ctx: Context<RefundCtx>,
        _escrow_name: String,
    ) -> Result<()> {
        let escrow_acc = &mut ctx.accounts.escrow_info;
        require!(escrow_acc.state == State::WaitRecipient, EscrowError::InvalidState);
        require!(escrow_acc.seller == *ctx.accounts.seller.key, EscrowError::Unauthorized);

        // move only the deposited lamports from PDA → buyer
        let deposit_amt = escrow_acc.amount_in_lamports;
        let escrow_ai = escrow_acc.to_account_info();
        let buyer_ai  = ctx.accounts.buyer.to_account_info();

        **escrow_ai.try_borrow_mut_lamports()? -= deposit_amt;
        **buyer_ai.try_borrow_mut_lamports()?  += deposit_amt;

        // update state to Closed; Anchor then runs the `close = seller`
        escrow_acc.state = State::Closed;
        Ok(())
    }
}

//
// Account Contexts
//

#[derive(Accounts)]
#[instruction(amount_in_lamports: u64, escrow_name: String)]
pub struct InitializeCtx<'info> {
    #[account(mut)]
    pub seller: Signer<'info>,

    /// CHECK: only used for PDA derivation & stored in state
    pub buyer: UncheckedAccount<'info>,

    #[account(
        init,
        payer  = seller,
        space  = EscrowInfo::LEN,
        seeds  = [escrow_name.as_bytes(), seller.key().as_ref(), buyer.key().as_ref()],
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

    /// CHECK: only used for PDA derivation
    pub seller: UncheckedAccount<'info>,

    #[account(
        mut,
        seeds = [escrow_name.as_bytes(), seller.key().as_ref(), buyer.key().as_ref()],
        bump  = escrow_info.bump
    )]
    pub escrow_info: Account<'info, EscrowInfo>,

    pub system_program: Program<'info, System>,
}

#[derive(Accounts)]
#[instruction(escrow_name: String)]
pub struct PayCtx<'info> {
    pub buyer: Signer<'info>,

    /// CHECK: only used for PDA derivation
    #[account(mut)]
    pub seller: UncheckedAccount<'info>,

    #[account(
        mut,
        seeds = [escrow_name.as_bytes(), seller.key().as_ref(), buyer.key().as_ref()],
        bump  = escrow_info.bump,
        close = seller
    )]
    pub escrow_info: Account<'info, EscrowInfo>,
}

#[derive(Accounts)]
#[instruction(escrow_name: String)]
pub struct RefundCtx<'info> {
    #[account(mut)]
    pub seller: Signer<'info>,

    /// CHECK: only used for PDA derivation & to receive refund
    #[account(mut)]
    pub buyer: UncheckedAccount<'info>,

    #[account(
        mut,
        seeds = [escrow_name.as_bytes(), seller.key().as_ref(), buyer.key().as_ref()],
        bump  = escrow_info.bump,
        close = seller
    )]
    pub escrow_info: Account<'info, EscrowInfo>,
}

//
// Shared State & Errors
//

#[account]
pub struct EscrowInfo {
    pub seller: Pubkey,
    pub buyer: Pubkey,
    pub amount_in_lamports: u64,
    pub state: State,
    pub bump: u8,
}

impl EscrowInfo {
    // 8 dsc + 32 seller + 32 buyer + 8 amount + 1 state + 1 bump = 82
    pub const LEN: usize = 8 + 32 + 32 + 8 + 1 + 1;
}

#[derive(AnchorSerialize, AnchorDeserialize, Clone, PartialEq, Eq)]
pub enum State {
    WaitDeposit,
    WaitRecipient,
    Closed,
}

#[error_code]
pub enum EscrowError {
    #[msg("Transfer amount must be greater than zero")]
    ZeroAmount,
    #[msg("Invalid escrow state for this operation")]
    InvalidState,
    #[msg("Unauthorized signer for this instruction")]
    Unauthorized,
    #[msg("Escrow does not hold sufficient funds")]
    InsufficientFunds,
}
