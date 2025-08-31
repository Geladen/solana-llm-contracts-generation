#![allow(unexpected_cfgs)]
use anchor_lang::prelude::*;
use anchor_lang::system_program;

declare_id!("3cm888HuhMBQhEC62MuYZ3XVqJej3U2J3V5yGwwUNjyd");

#[program]
pub mod secure_transfer {
    use super::*;

    pub fn initialize(ctx: Context<InitializeCtx>) -> Result<()> {
        // Initialize the PDA account with zero balance
        let account = &mut ctx.accounts.balance_holder_pda;
        account.sender = ctx.accounts.sender.key();
        account.recipient = ctx.accounts.recipient.key();
        account.amount = 0;
        
        Ok(())
    }

    pub fn deposit(ctx: Context<DepositCtx>, amount_to_deposit: u64) -> Result<()> {
        // Validate amount
        require!(amount_to_deposit > 0, TransferError::ZeroAmount);

        // Calculate PDA bump
        let (pda, bump) = Pubkey::find_program_address(
            &[
                ctx.accounts.recipient.key().as_ref(),
                ctx.accounts.sender.key().as_ref(),
            ],
            ctx.program_id,
        );
        require!(ctx.accounts.balance_holder_pda.key() == pda, TransferError::InvalidPda);

        // Validate PDA data
        require!(
            ctx.accounts.balance_holder_pda.sender == ctx.accounts.sender.key() &&
            ctx.accounts.balance_holder_pda.recipient == ctx.accounts.recipient.key(),
            TransferError::Unauthorized
        );

        // Transfer lamports to PDA
        system_program::transfer(
            CpiContext::new(
                ctx.accounts.system_program.to_account_info(),
                system_program::Transfer {
                    from: ctx.accounts.sender.to_account_info(),
                    to: ctx.accounts.balance_holder_pda.to_account_info(),
                },
            ),
            amount_to_deposit,
        )?;

        // Update PDA account balance
        ctx.accounts.balance_holder_pda.amount = ctx.accounts.balance_holder_pda.amount
            .checked_add(amount_to_deposit)
            .ok_or(TransferError::Overflow)?;

        Ok(())
    }

    pub fn withdraw(ctx: Context<WithdrawCtx>, amount_to_withdraw: u64) -> Result<()> {
        // Validate amount
        require!(amount_to_withdraw > 0, TransferError::ZeroAmount);

        // Verify PDA derivation
        let (pda, _bump) = Pubkey::find_program_address(
            &[
                ctx.accounts.recipient.key().as_ref(),
                ctx.accounts.sender.key().as_ref(),
            ],
            ctx.program_id,
        );
        require!(ctx.accounts.balance_holder_pda.key() == pda, TransferError::InvalidPda);

        // Validate PDA data
        require!(
            ctx.accounts.balance_holder_pda.recipient == ctx.accounts.recipient.key() &&
            ctx.accounts.balance_holder_pda.sender == ctx.accounts.sender.key(),
            TransferError::Unauthorized
        );
        require!(ctx.accounts.balance_holder_pda.amount >= amount_to_withdraw, TransferError::InsufficientFunds);

        // Update balance
        ctx.accounts.balance_holder_pda.amount = ctx.accounts.balance_holder_pda.amount
            .checked_sub(amount_to_withdraw)
            .ok_or(TransferError::Underflow)?;

        // Transfer funds to recipient
        **ctx.accounts.balance_holder_pda.to_account_info().try_borrow_mut_lamports()? -= amount_to_withdraw;
        **ctx.accounts.recipient.to_account_info().try_borrow_mut_lamports()? += amount_to_withdraw;

        // Close account if balance is zero
        if ctx.accounts.balance_holder_pda.amount == 0 {
            let remaining_lamports = ctx.accounts.balance_holder_pda.to_account_info().lamports();
            **ctx.accounts.balance_holder_pda.to_account_info().try_borrow_mut_lamports()? = 0;
            **ctx.accounts.sender.to_account_info().try_borrow_mut_lamports()? += remaining_lamports;
        }

        Ok(())
    }
}

#[derive(Accounts)]
pub struct InitializeCtx<'info> {
    #[account(mut)]
    pub sender: Signer<'info>,
    /// CHECK: Used only for PDA derivation validation
    pub recipient: AccountInfo<'info>,
    #[account(
        init,
        seeds = [recipient.key().as_ref(), sender.key().as_ref()],
        bump,
        payer = sender,
        space = 8 + 32 + 32 + 8
    )]
    pub balance_holder_pda: Account<'info, BalanceHolderPDA>,
    pub system_program: Program<'info, System>,
}

#[derive(Accounts)]
pub struct DepositCtx<'info> {
    #[account(mut)]
    pub sender: Signer<'info>,
    /// CHECK: Used only for PDA derivation validation
    pub recipient: AccountInfo<'info>,
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
    #[account(mut)]
    pub recipient: Signer<'info>,
    /// CHECK: Used only for PDA derivation and validation
    pub sender: AccountInfo<'info>,
    #[account(
        mut,
        seeds = [recipient.key().as_ref(), sender.key().as_ref()],
        bump
    )]
    pub balance_holder_pda: Account<'info, BalanceHolderPDA>,
    /// CHECK: Required for potential account closure
    #[account(address = anchor_lang::solana_program::sysvar::rent::ID)]
    pub rent: AccountInfo<'info>,
}

#[account]
pub struct BalanceHolderPDA {
    pub sender: Pubkey,
    pub recipient: Pubkey,
    pub amount: u64,
}

#[error_code]
pub enum TransferError {
    #[msg("Amount must be greater than zero")]
    ZeroAmount,
    #[msg("Invalid PDA account derivation")]
    InvalidPda,
    #[msg("Unauthorized withdrawal attempt")]
    Unauthorized,
    #[msg("Insufficient funds in PDA")]
    InsufficientFunds,
    #[msg("Arithmetic overflow occurred")]
    Overflow,
    #[msg("Arithmetic underflow occurred")]
    Underflow,
}