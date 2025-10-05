use anchor_lang::prelude::*;
use anchor_lang::system_program;


declare_id!("7J5WJsJq21xoMvzpCjQxS3PLgk5ie4kz47qxUmE84BV");

#[program]
pub mod simple_transfer {
    use super::*;

    pub fn deposit(ctx: Context<DepositCtx>, amount_to_deposit: u64) -> Result<()> {
        require!(amount_to_deposit > 0, ErrorCode::ZeroAmount);

        let sender_key = ctx.accounts.sender.key();
        let recipient_key = ctx.accounts.recipient.key();

        // Compute PDA seeds
        let bump = ctx.bumps.balance_holder_pda;
        let seeds: &[&[u8]] = &[
            recipient_key.as_ref(),
            sender_key.as_ref(),
            &[bump],
        ];

        let pda_info = ctx.accounts.balance_holder_pda.to_account_info();
        let pda_account = &mut ctx.accounts.balance_holder_pda;

        // Transfer lamports from sender to PDA
        let cpi_ctx = CpiContext::new(
            ctx.accounts.system_program.to_account_info(),
            system_program::Transfer {
                from: ctx.accounts.sender.to_account_info(),
                to: pda_info.clone(),
            },
        );
        system_program::transfer(cpi_ctx, amount_to_deposit)?;

        // Update PDA state
        pda_account.sender = sender_key;
        pda_account.recipient = recipient_key;
        pda_account.amount = pda_account
            .amount
            .checked_add(amount_to_deposit)
            .ok_or(ErrorCode::Overflow)?;

        Ok(())
    }

    pub fn withdraw(ctx: Context<WithdrawCtx>, amount_to_withdraw: u64) -> Result<()> {
        require!(amount_to_withdraw > 0, ErrorCode::ZeroAmount);

        let sender_key = ctx.accounts.sender.key();
        let recipient_key = ctx.accounts.recipient.key();

        let bump = ctx.bumps.balance_holder_pda;
        let seeds: &[&[&[u8]]] = &[&[
            recipient_key.as_ref(),
            sender_key.as_ref(),
            &[bump],
        ]];

        let pda_info = ctx.accounts.balance_holder_pda.to_account_info();
        let pda_account = &mut ctx.accounts.balance_holder_pda;

        require!(
            amount_to_withdraw <= pda_account.amount,
            ErrorCode::Underflow
        );

        // Transfer lamports from PDA to recipient using signer seeds
        let cpi_ctx = CpiContext::new_with_signer(
            ctx.accounts.system_program.to_account_info(),
            system_program::Transfer {
                from: pda_info.clone(),
                to: ctx.accounts.recipient.to_account_info(),
            },
            seeds,
        );
        system_program::transfer(cpi_ctx, amount_to_withdraw)?;

        // Update PDA state
        pda_account.amount = pda_account.amount.checked_sub(amount_to_withdraw).unwrap();

        // Close PDA if empty
        if pda_account.amount == 0 {
            **ctx.accounts.sender.lamports.borrow_mut() = ctx
                .accounts
                .sender
                .lamports()
                .checked_add(pda_info.lamports())
                .unwrap();
            **pda_info.lamports.borrow_mut() = 0;
            let _ = pda_info.try_borrow_mut_data()?.fill(0);
        }

        Ok(())
    }
}

#[account]
pub struct BalanceHolderPDA {
    pub sender: Pubkey,
    pub recipient: Pubkey,
    pub amount: u64,
}

#[derive(Accounts)]
#[instruction(amount_to_deposit: u64)]
pub struct DepositCtx<'info> {
    #[account(mut)]
    pub sender: Signer<'info>,

    /// CHECK: used for PDA derivation
    pub recipient: UncheckedAccount<'info>,

    #[account(
        init_if_needed,
        payer = sender,
        space = 8 + 32 + 32 + 8,
        seeds = [recipient.key().as_ref(), sender.key().as_ref()],
        bump
    )]
    pub balance_holder_pda: Account<'info, BalanceHolderPDA>,

    pub system_program: Program<'info, System>,
}

#[derive(Accounts)]
#[instruction(amount_to_withdraw: u64)]
pub struct WithdrawCtx<'info> {
    #[account(mut)]
    pub recipient: Signer<'info>,

    /// CHECK: used for PDA derivation
    pub sender: UncheckedAccount<'info>,

    #[account(
        mut,
        seeds = [recipient.key().as_ref(), sender.key().as_ref()],
        bump
    )]
    pub balance_holder_pda: Account<'info, BalanceHolderPDA>,

    pub system_program: Program<'info, System>,
}

#[error_code]
pub enum ErrorCode {
    #[msg("Amount must be greater than zero.")]
    ZeroAmount,
    #[msg("Overflow in calculation.")]
    Overflow,
    #[msg("Not enough funds.")]
    Underflow,
}
