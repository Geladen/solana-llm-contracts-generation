use anchor_lang::prelude::*;
use anchor_lang::system_program;

declare_id!("7evrYYGbzmwaVqYX1r4NpG7NwNj7FVn6aw3x8MqKr3mr");

#[program]
pub mod simple_transfer {
    use super::*;

    pub fn deposit(ctx: Context<DepositCtx>, amount_to_deposit: u64) -> Result<()> {
        // Validate amount
        require!(amount_to_deposit > 0, ErrorCode::InvalidAmount);

        let balance_holder = &mut ctx.accounts.balance_holder_pda;
        
        // Initialize or update PDA
        if balance_holder.amount == 0 {
            balance_holder.sender = ctx.accounts.sender.key();
            balance_holder.recipient = ctx.accounts.recipient.key();
            balance_holder.amount = amount_to_deposit;
        } else {
            // Verify existing accounts match
            require!(
                balance_holder.sender == ctx.accounts.sender.key(),
                ErrorCode::InvalidSender
            );
            require!(
                balance_holder.recipient == ctx.accounts.recipient.key(),
                ErrorCode::InvalidRecipient
            );
            balance_holder.amount = balance_holder.amount
                .checked_add(amount_to_deposit)
                .ok_or(ErrorCode::Overflow)?;
        }

        // Transfer funds to PDA
        let cpi_context = CpiContext::new(
            ctx.accounts.system_program.to_account_info(),
            system_program::Transfer {
                from: ctx.accounts.sender.to_account_info(),
                to: ctx.accounts.balance_holder_pda.to_account_info(),
            },
        );
        system_program::transfer(cpi_context, amount_to_deposit)?;

        Ok(())
    }

    pub fn withdraw(ctx: Context<WithdrawCtx>, amount_to_withdraw: u64) -> Result<()> {
        // Validate amount and balance
        require!(amount_to_withdraw > 0, ErrorCode::InvalidAmount);
        require!(
            amount_to_withdraw <= ctx.accounts.balance_holder_pda.amount,
            ErrorCode::InsufficientFunds
        );

        // Get account info references first
        let balance_holder_info = ctx.accounts.balance_holder_pda.to_account_info();
        let recipient_info = ctx.accounts.recipient.to_account_info();
        let sender_info = ctx.accounts.sender.to_account_info();
        
        // Verify account consistency using the account data directly
        let balance_holder = &mut ctx.accounts.balance_holder_pda;
        require!(
            balance_holder.recipient == ctx.accounts.recipient.key(),
            ErrorCode::InvalidRecipient
        );
        require!(
            balance_holder.sender == ctx.accounts.sender.key(),
            ErrorCode::InvalidSender
        );

        // Check if this withdrawal will zero out the balance
        let will_close_account = balance_holder.amount == amount_to_withdraw;
        
        // Update balance
        balance_holder.amount = balance_holder.amount
            .checked_sub(amount_to_withdraw)
            .ok_or(ErrorCode::Underflow)?;

        // Transfer requested amount to recipient
        **balance_holder_info.try_borrow_mut_lamports()? = balance_holder_info
            .lamports()
            .checked_sub(amount_to_withdraw)
            .ok_or(ErrorCode::InsufficientLamports)?;
        **recipient_info.try_borrow_mut_lamports()? = recipient_info
            .lamports()
            .checked_add(amount_to_withdraw)
            .ok_or(ErrorCode::Overflow)?;

        // Close account if balance reaches zero and transfer remaining lamports (rent) to sender
        if will_close_account {
            let remaining_lamports = balance_holder_info.lamports();
            if remaining_lamports > 0 {
                **balance_holder_info.try_borrow_mut_lamports()? = 0;
                **sender_info.try_borrow_mut_lamports()? = sender_info
                    .lamports()
                    .checked_add(remaining_lamports)
                    .ok_or(ErrorCode::Overflow)?;
                
                // Mark account as closed by setting discriminator to zero
                let mut data = balance_holder_info.try_borrow_mut_data()?;
                for i in 0..data.len() {
                    data[i] = 0;
                }
            }
        }

        Ok(())
    }
}

#[derive(Accounts)]
pub struct DepositCtx<'info> {
    #[account(mut)]
    pub sender: Signer<'info>,
    /// CHECK: Used only for PDA derivation and state storage
    pub recipient: AccountInfo<'info>,
    #[account(
        init_if_needed,
        payer = sender,
        space = BalanceHolderPDA::SIZE,
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
    /// CHECK: Used for PDA derivation and rent recipient
    #[account(mut)]
    pub sender: AccountInfo<'info>,
    #[account(
        mut,
        seeds = [recipient.key().as_ref(), sender.key().as_ref()],
        bump,
        constraint = balance_holder_pda.amount > 0 @ ErrorCode::InsufficientFunds
    )]
    pub balance_holder_pda: Account<'info, BalanceHolderPDA>,
}

#[account]
pub struct BalanceHolderPDA {
    pub sender: Pubkey,
    pub recipient: Pubkey,
    pub amount: u64,
}

impl BalanceHolderPDA {
    pub const SIZE: usize = 8 + // discriminator
        32 + // sender
        32 + // recipient
        8; // amount
}

#[error_code]
pub enum ErrorCode {
    #[msg("Amount must be greater than zero")]
    InvalidAmount,
    #[msg("Insufficient funds in account")]
    InsufficientFunds,
    #[msg("Invalid sender account")]
    InvalidSender,
    #[msg("Invalid recipient account")]
    InvalidRecipient,
    #[msg("Insufficient lamports for operation")]
    InsufficientLamports,
    #[msg("Arithmetic overflow")]
    Overflow,
    #[msg("Arithmetic underflow")]
    Underflow,
}