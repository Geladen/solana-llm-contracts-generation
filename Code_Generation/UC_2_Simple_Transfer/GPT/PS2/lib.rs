use anchor_lang::prelude::*;
use anchor_lang::system_program;
use anchor_lang::solana_program::system_instruction;

declare_id!("E8f81TCHVzNaVkqxeVa3S6Znn5PEfTECKZoSF8EjnaHv");

#[program]
pub mod simple_gpt {
    use super::*;

    // First deposit: initializes PDA
    pub fn deposit_init(ctx: Context<DepositInit>, amount_to_deposit: u64) -> Result<()> {
        require!(amount_to_deposit > 0, ErrorCode::InvalidAmount);

        let sender = &ctx.accounts.sender;
        let recipient = &ctx.accounts.recipient;
        let pda = &mut ctx.accounts.balance_holder_pda;

        // Transfer lamports
        let ix = system_instruction::transfer(&sender.key(), &pda.key(), amount_to_deposit);
        anchor_lang::solana_program::program::invoke(
            &ix,
            &[
                sender.to_account_info(),
                pda.to_account_info(),
                ctx.accounts.system_program.to_account_info(),
            ],
        )?;

        // Initialize PDA fields
        pda.sender = sender.key();
        pda.recipient = recipient.key();
        pda.amount = amount_to_deposit;

        Ok(())
    }

    // Subsequent deposits: update PDA
    pub fn deposit(ctx: Context<DepositUpdate>, amount_to_deposit: u64) -> Result<()> {
        require!(amount_to_deposit > 0, ErrorCode::InvalidAmount);

        let sender = &ctx.accounts.sender;
        let recipient = &ctx.accounts.recipient;
        let pda = &mut ctx.accounts.balance_holder_pda;

        require!(pda.sender == sender.key(), ErrorCode::SenderMismatch);
        require!(pda.recipient == recipient.key(), ErrorCode::RecipientMismatch);

        // Transfer lamports
        let ix = system_instruction::transfer(&sender.key(), &pda.key(), amount_to_deposit);
        anchor_lang::solana_program::program::invoke(
            &ix,
            &[
                sender.to_account_info(),
                pda.to_account_info(),
                ctx.accounts.system_program.to_account_info(),
            ],
        )?;

        // Update balance
        pda.amount = pda
            .amount
            .checked_add(amount_to_deposit)
            .ok_or(ErrorCode::Overflow)?;

        Ok(())
    }

    pub fn withdraw(ctx: Context<WithdrawCtx>, amount_to_withdraw: u64) -> Result<()> {
        require!(amount_to_withdraw > 0, ErrorCode::InvalidAmount);

        let recipient = &ctx.accounts.recipient;
        let sender = &ctx.accounts.sender;
        let pda = &mut ctx.accounts.balance_holder_pda;

        require!(pda.sender == sender.key(), ErrorCode::SenderMismatch);
        require!(pda.recipient == recipient.key(), ErrorCode::RecipientMismatch);
        require!(pda.amount >= amount_to_withdraw, ErrorCode::InsufficientFunds);

        // Transfer lamports from PDA to recipient
        **pda.to_account_info().try_borrow_mut_lamports()? -= amount_to_withdraw;
        **recipient.to_account_info().try_borrow_mut_lamports()? += amount_to_withdraw;

        pda.amount -= amount_to_withdraw;

        // Close PDA if balance is now zero
        if pda.amount == 0 {
            let pda_info = pda.to_account_info();
            let sender_info = sender.to_account_info();
            anchor_lang::system_program::close_account(CpiContext::new(
                ctx.accounts.system_program.to_account_info(),
                system_program::CloseAccount {
                    account: pda_info,
                    destination: sender_info,
                },
            ))?;
        }

        Ok(())
    }
}

#[derive(Accounts)]
pub struct DepositInit<'info> {
    #[account(mut)]
    pub sender: Signer<'info>,
    /// CHECK: only used for PDA derivation
    pub recipient: UncheckedAccount<'info>,
    #[account(
        init,
        payer = sender,
        space = 8 + 32 + 32 + 8,
        seeds = [recipient.key().as_ref(), sender.key().as_ref()],
        bump
    )]
    pub balance_holder_pda: Account<'info, BalanceHolderPDA>,
    pub system_program: Program<'info, System>,
}

#[derive(Accounts)]
pub struct DepositUpdate<'info> {
    #[account(mut)]
    pub sender: Signer<'info>,
    /// CHECK: only used for PDA derivation
    pub recipient: UncheckedAccount<'info>,
    #[account(
        mut,
        seeds = [recipient.key().as_ref(), sender.key().as_ref()],
        bump
    )]
    pub balance_holder_pda: Account<'info, BalanceHolderPDA>,
    pub system_program: Program<'info, System>,
}

#[derive(Accounts)]
pub struct WithdrawCtx<'info> {
    pub recipient: Signer<'info>,
    /// CHECK: only used for PDA derivation
    pub sender: UncheckedAccount<'info>,
    #[account(
        mut,
        seeds = [recipient.key().as_ref(), sender.key().as_ref()],
        bump,
        close = sender
    )]
    pub balance_holder_pda: Account<'info, BalanceHolderPDA>,
    pub system_program: Program<'info, System>,
    pub rent: Sysvar<'info, Rent>,
}

#[account]
pub struct BalanceHolderPDA {
    pub sender: Pubkey,
    pub recipient: Pubkey,
    pub amount: u64,
}

#[error_code]
pub enum ErrorCode {
    #[msg("Deposit/withdrawal amount must be greater than zero")]
    InvalidAmount,

    #[msg("Math overflow occurred")]
    Overflow,

    #[msg("Sender does not match PDA record")]
    SenderMismatch,

    #[msg("Recipient does not match PDA record")]
    RecipientMismatch,

    #[msg("Insufficient funds in PDA")]
    InsufficientFunds,
}

