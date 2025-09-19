use anchor_lang::prelude::*;
use anchor_lang::system_program;

declare_id!("HmJez9BGuzv4TDFeRhZdFjJyiys21G1DPyGKb8hx8hU9");

#[program]
pub mod escrow {
    use super::*;

    /// initialize(ctx, amount, escrow_name)
    ///  • seller = signer
    ///  • seeds = [escrow_name, seller, buyer]
    ///  • creates PDA account
    ///  • state = WaitDeposit
    pub fn initialize(
        ctx: Context<InitializeCtx>,
        amount_in_lamports: u64,
        escrow_name: String,
    ) -> Result<()> {
        // validate
        if amount_in_lamports == 0 {
            return Err(ErrorCode::ZeroAmount.into());
        }
        // re-derive PDA + bump, check it matches
        let (pda, _bump) = Pubkey::find_program_address(
            &[
                escrow_name.as_bytes(),
                ctx.accounts.seller.key.as_ref(),
                ctx.accounts.buyer.key.as_ref(),
            ],
            ctx.program_id,
        );
        require!(pda == ctx.accounts.escrow_info.key(), ErrorCode::InvalidPDA);

        // initialize
        let escrow = &mut ctx.accounts.escrow_info;
        escrow.seller             = *ctx.accounts.seller.key;
        escrow.buyer              = *ctx.accounts.buyer.key;
        escrow.amount_in_lamports = amount_in_lamports;
        escrow.state              = State::WaitDeposit;
        Ok(())
    }

    /// deposit(ctx, escrow_name)
    ///  • buyer = signer
    ///  • must be WaitDeposit
    ///  • transfers exact amount into PDA
    ///  • state → WaitRecipient
    pub fn deposit(
        ctx: Context<DepositCtx>,
        escrow_name: String,
    ) -> Result<()> {
        // manual PDA check
        let (pda, _bump) = Pubkey::find_program_address(
            &[
                escrow_name.as_bytes(),
                ctx.accounts.seller.key.as_ref(),
                ctx.accounts.buyer.key.as_ref(),
            ],
            ctx.program_id,
        );
        require!(pda == ctx.accounts.escrow_info.key(), ErrorCode::InvalidPDA);

        // state + authority checks
        require!(
            ctx.accounts.escrow_info.state == State::WaitDeposit,
            ErrorCode::InvalidState
        );
        require!(
            ctx.accounts.escrow_info.buyer == ctx.accounts.buyer.key(),
            ErrorCode::Unauthorized
        );

        // do the transfer
        let amount = ctx.accounts.escrow_info.amount_in_lamports;
        let cpi_ctx = CpiContext::new(
            ctx.accounts.system_program.to_account_info(),
            system_program::Transfer {
                from: ctx.accounts.buyer.to_account_info(),
                to:   ctx.accounts.escrow_info.to_account_info(),
            },
        );
        system_program::transfer(cpi_ctx, amount)?;

        // flip state
        ctx.accounts.escrow_info.state = State::WaitRecipient;
        Ok(())
    }

    /// pay(ctx, escrow_name)
    ///  • buyer = signer
    ///  • must be WaitRecipient
    ///  • transfers full PDA lamports → seller
    ///  • closes PDA (rent goes to seller)
    ///  • state → Closed
    pub fn pay(
        ctx: Context<PayCtx>,
        escrow_name: String,
    ) -> Result<()> {
        // manual PDA check
        let (pda, bump) = Pubkey::find_program_address(
            &[
                escrow_name.as_bytes(),
                ctx.accounts.seller.key.as_ref(),
                ctx.accounts.buyer.key.as_ref(),
            ],
            ctx.program_id,
        );
        require!(pda == ctx.accounts.escrow_info.key(), ErrorCode::InvalidPDA);

        // state + authority
        require!(
            ctx.accounts.escrow_info.state == State::WaitRecipient,
            ErrorCode::InvalidState
        );
        require!(
            ctx.accounts.escrow_info.buyer == ctx.accounts.buyer.key(),
            ErrorCode::Unauthorized
        );
        require!(
            ctx.accounts.escrow_info.seller == ctx.accounts.seller.key(),
            ErrorCode::Unauthorized
        );

        // send all lamports out
        let total_balance = {
            let ai = ctx.accounts.escrow_info.to_account_info();
            ai.lamports()
        };
        let seeds = &[
            escrow_name.as_bytes(),
            ctx.accounts.seller.key.as_ref(),
            ctx.accounts.buyer.key.as_ref(),
            &[bump],
        ];
        let signer = &[&seeds[..]];
        {
            let cpi = CpiContext::new_with_signer(
                ctx.accounts.system_program.to_account_info(),
                system_program::Transfer {
                    from: ctx.accounts.escrow_info.to_account_info(),
                    to:   ctx.accounts.seller.to_account_info(),
                },
                signer,
            );
            system_program::transfer(cpi, total_balance)?;
        }

        ctx.accounts.escrow_info.state = State::Closed;
        Ok(())
    }

    /// refund(ctx, escrow_name)
    ///  • seller = signer
    ///  • must be WaitRecipient
    ///  • refunds `amount_in_lamports` → buyer
    ///  • closes PDA (rent → seller)
    ///  • state → Closed
    pub fn refund(
        ctx: Context<RefundCtx>,
        escrow_name: String,
    ) -> Result<()> {
        // manual PDA check
        let (pda, bump) = Pubkey::find_program_address(
            &[
                escrow_name.as_bytes(),
                ctx.accounts.seller.key.as_ref(),
                ctx.accounts.buyer.key.as_ref(),
            ],
            ctx.program_id,
        );
        require!(pda == ctx.accounts.escrow_info.key(), ErrorCode::InvalidPDA);

        // state + authority
        require!(
            ctx.accounts.escrow_info.state == State::WaitRecipient,
            ErrorCode::InvalidState
        );
        require!(
            ctx.accounts.escrow_info.buyer == ctx.accounts.buyer.key(),
            ErrorCode::Unauthorized
        );
        require!(
            ctx.accounts.escrow_info.seller == ctx.accounts.seller.key(),
            ErrorCode::Unauthorized
        );

        // refund the buyer
        let amount = ctx.accounts.escrow_info.amount_in_lamports;
        let seeds = &[
            escrow_name.as_bytes(),
            ctx.accounts.seller.key.as_ref(),
            ctx.accounts.buyer.key.as_ref(),
            &[bump],
        ];
        let signer = &[&seeds[..]];
        {
            let cpi = CpiContext::new_with_signer(
                ctx.accounts.system_program.to_account_info(),
                system_program::Transfer {
                    from: ctx.accounts.escrow_info.to_account_info(),
                    to:   ctx.accounts.buyer.to_account_info(),
                },
                signer,
            );
            system_program::transfer(cpi, amount)?;
        }

        ctx.accounts.escrow_info.state = State::Closed;
        Ok(())
    }
}

//----------------------------- CONTEXTS -----------------------------

#[derive(Accounts)]
#[instruction(escrow_name: String)]
pub struct InitializeCtx<'info> {
    #[account(mut)]
    pub seller: Signer<'info>,

    /// CHECK: Only used to derive PDA; no on-chain checks needed here.
    pub buyer: UncheckedAccount<'info>,

    #[account(
        init,
        seeds = [
            escrow_name.as_bytes(),
            seller.key().as_ref(),
            buyer.key().as_ref()
        ],
        bump,
        payer = seller,
        space  = 8   // discriminator
               + 32  // seller
               + 32  // buyer
               + 8   // amount
               + 1    // state
    )]
    pub escrow_info: Account<'info, EscrowInfo>,

    pub system_program: Program<'info, System>,
}

#[derive(Accounts)]
pub struct DepositCtx<'info> {
    #[account(mut)]
    pub buyer: Signer<'info>,

    /// CHECK: will be validated inside `deposit()` against `escrow_info.seller`
    pub seller: UncheckedAccount<'info>,

    #[account(mut)]
    pub escrow_info: Account<'info, EscrowInfo>,

    pub system_program: Program<'info, System>,
}

#[derive(Accounts)]
pub struct PayCtx<'info> {
    #[account(mut)]
    pub buyer: Signer<'info>,

    /// CHECK: validated in `pay()` against `escrow_info.seller`
    #[account(mut)]
    pub seller: UncheckedAccount<'info>,

    #[account(mut, close = seller)]
    pub escrow_info: Account<'info, EscrowInfo>,

    pub system_program: Program<'info, System>,
}

#[derive(Accounts)]
pub struct RefundCtx<'info> {
    #[account(mut)]
    pub seller: Signer<'info>,

    /// CHECK: validated in `refund()` against `escrow_info.buyer`
    #[account(mut)]
    pub buyer: UncheckedAccount<'info>,

    #[account(mut, close = seller)]
    pub escrow_info: Account<'info, EscrowInfo>,

    pub system_program: Program<'info, System>,
}

//----------------------------- STATE & ERRORS -----------------------------

#[account]
pub struct EscrowInfo {
    pub seller:             Pubkey,
    pub buyer:              Pubkey,
    pub amount_in_lamports: u64,
    pub state:              State,
}

#[derive(AnchorSerialize, AnchorDeserialize, Clone, PartialEq, Eq)]
pub enum State {
    WaitDeposit,
    WaitRecipient,
    Closed,
}

#[error_code]
pub enum ErrorCode {
    #[msg("Amount must be greater than zero")]
    ZeroAmount,

    #[msg("Invalid state for this operation")]
    InvalidState,

    #[msg("Unauthorized account")]
    Unauthorized,

    #[msg("PDA derivation mismatch")]
    InvalidPDA,
}
