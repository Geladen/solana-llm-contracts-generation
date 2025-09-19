use anchor_lang::prelude::*;
use anchor_lang::system_program;

declare_id!("3cm888HuhMBQhEC62MuYZ3XVqJej3U2J3V5yGwwUNjyd");

#[program]
pub mod transfer_contract {
    use super::*;

    pub fn initialize(ctx: Context<InitializeCtx>) -> Result<()> {
        let balance_holder = &mut ctx.accounts.balance_holder_pda;
        balance_holder.sender = ctx.accounts.sender.key();
        balance_holder.recipient = ctx.accounts.recipient.key();
        balance_holder.amount = 0;
        Ok(())
    }

    pub fn deposit(ctx: Context<DepositCtx>, amount_to_deposit: u64) -> Result<()> {
        // Validate amount is positive
        require!(amount_to_deposit > 0, ErrorCode::InvalidAmount);

        let balance_holder = &mut ctx.accounts.balance_holder_pda;
        
        // Update PDA amount
        balance_holder.amount = balance_holder.amount
            .checked_add(amount_to_deposit)
            .ok_or(ErrorCode::Overflow)?;

        // Transfer funds to PDA
        anchor_lang::system_program::transfer(
            CpiContext::new(
                ctx.accounts.system_program.to_account_info(),
                anchor_lang::system_program::Transfer {
                    from: ctx.accounts.sender.to_account_info(),
                    to: ctx.accounts.balance_holder_pda.to_account_info(),
                },
            ),
            amount_to_deposit,
        )?;

        Ok(())
    }

    pub fn withdraw(ctx: Context<WithdrawCtx>, amount_to_withdraw: u64) -> Result<()> {
        // Validate amount is positive and sufficient balance
        require!(amount_to_withdraw > 0, ErrorCode::InvalidAmount);
        require!(
            amount_to_withdraw <= ctx.accounts.balance_holder_pda.amount,
            ErrorCode::InsufficientFunds
        );

        // Update PDA balance
        let balance_holder = &mut ctx.accounts.balance_holder_pda;
        balance_holder.amount = balance_holder.amount
            .checked_sub(amount_to_withdraw)
            .ok_or(ErrorCode::Underflow)?;

        // Transfer funds to recipient
        let current_balance = balance_holder.amount;
        **balance_holder.to_account_info().try_borrow_mut_lamports()? = balance_holder
            .to_account_info()
            .lamports()
            .checked_sub(amount_to_withdraw)
            .ok_or(ErrorCode::InsufficientLamports)?;
        **ctx.accounts.recipient.try_borrow_mut_lamports()? = ctx
            .accounts
            .recipient
            .lamports()
            .checked_add(amount_to_withdraw)
            .ok_or(ErrorCode::Overflow)?;

        // Close PDA if balance is zero
        if current_balance == 0 {
            let balance_holder_lamports = balance_holder.to_account_info().lamports();
            **balance_holder.to_account_info().try_borrow_mut_lamports()? = 0;
            **ctx.accounts.sender.as_ref().try_borrow_mut_lamports()? = ctx
                .accounts
                .sender
                .lamports()
                .checked_add(balance_holder_lamports)
                .ok_or(ErrorCode::Overflow)?;
        }

        Ok(())
    }
}

#[derive(Accounts)]
pub struct InitializeCtx<'info> {
    #[account(mut)]
    sender: Signer<'info>,
    /// CHECK: Used for PDA derivation
    recipient: AccountInfo<'info>,
    #[account(
        init,
        payer = sender,
        space = 8 + BalanceHolderPDA::INIT_SPACE,
        seeds = [
            recipient.key().as_ref(),
            sender.key().as_ref()
        ],
        bump
    )]
    balance_holder_pda: Account<'info, BalanceHolderPDA>,
    system_program: Program<'info, System>,
}

#[derive(Accounts)]
pub struct DepositCtx<'info> {
    #[account(mut)]
    sender: Signer<'info>,
    /// CHECK: Used for PDA derivation
    recipient: AccountInfo<'info>,
    #[account(
        mut,
        seeds = [
            recipient.key().as_ref(),
            sender.key().as_ref()
        ],
        bump,
        has_one = sender,
        has_one = recipient
    )]
    balance_holder_pda: Account<'info, BalanceHolderPDA>,
    system_program: Program<'info, System>,
}

#[derive(Accounts)]
pub struct WithdrawCtx<'info> {
    #[account(mut)]
    recipient: Signer<'info>,
    /// CHECK: Used for PDA derivation and validation
    sender: AccountInfo<'info>,
    #[account(
        mut,
        seeds = [
            recipient.key().as_ref(),
            sender.key().as_ref()
        ],
        bump,
        has_one = sender,
        has_one = recipient
    )]
    balance_holder_pda: Account<'info, BalanceHolderPDA>,
    /// CHECK: Required for potential account closure
    #[account(address = anchor_lang::solana_program::sysvar::rent::ID)]
    rent: AccountInfo<'info>,
}

#[account]
#[derive(InitSpace)]
pub struct BalanceHolderPDA {
    sender: Pubkey,
    recipient: Pubkey,
    amount: u64,
}

#[error_code]
pub enum ErrorCode {
    #[msg("Amount must be greater than zero")]
    InvalidAmount,
    #[msg("Insufficient funds in PDA")]
    InsufficientFunds,
    #[msg("Insufficient lamports for operation")]
    InsufficientLamports,
    #[msg("Arithmetic overflow")]
    Overflow,
    #[msg("Arithmetic underflow")]
    Underflow,
}
