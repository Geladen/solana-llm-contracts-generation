use anchor_lang::prelude::*;
use anchor_lang::system_program;

declare_id!("H8xcZLcJbeVQWEXuGFYAGT1rxaDMMuLrsnmZMTcJ1SZ8");

#[program]
pub mod transfer_contract {
    use super::*;

    pub fn deposit(ctx: Context<DepositCtx>, amount_to_deposit: u64) -> Result<()> {
        require!(amount_to_deposit > 0, ErrorCode::InvalidAmount);

        let balance_holder = &mut ctx.accounts.balance_holder_pda;
        
        if balance_holder.amount == 0 {
            balance_holder.sender = ctx.accounts.sender.key();
            balance_holder.recipient = ctx.accounts.recipient.key();
        }
        balance_holder.amount = balance_holder.amount.checked_add(amount_to_deposit).unwrap();

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

        Ok(())
    }

    pub fn withdraw(ctx: Context<WithdrawCtx>, amount_to_withdraw: u64) -> Result<()> {
        require!(amount_to_withdraw > 0, ErrorCode::InvalidAmount);
        
        // First, get the account info and amount before mutable borrow
        let pda_info = ctx.accounts.balance_holder_pda.to_account_info();
        let current_amount = ctx.accounts.balance_holder_pda.amount;
        
        require!(amount_to_withdraw <= current_amount, ErrorCode::InsufficientFunds);
        
        // Then update the balance
        let balance_holder = &mut ctx.accounts.balance_holder_pda;
        balance_holder.amount = current_amount.checked_sub(amount_to_withdraw).unwrap();
        
        // Transfer funds
        let recipient_info = ctx.accounts.recipient.to_account_info();
        **pda_info.try_borrow_mut_lamports()? -= amount_to_withdraw;
        **recipient_info.try_borrow_mut_lamports()? += amount_to_withdraw;

        // Check if we need to close the account
        if balance_holder.amount == 0 {
            let sender_info = ctx.accounts.sender.to_account_info();
            let current_lamports = pda_info.lamports();
            
            // Transfer ALL lamports (including rent exemption) to sender
            **pda_info.try_borrow_mut_lamports()? = 0;
            **sender_info.try_borrow_mut_lamports()? += current_lamports;
            
            // Close the account
            pda_info.assign(&system_program::ID);
            pda_info.resize(0)?;
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
pub struct DepositCtx<'info> {
    #[account(mut)]
    pub sender: Signer<'info>,
    /// CHECK: Only used for PDA derivation
    pub recipient: AccountInfo<'info>,
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
pub struct WithdrawCtx<'info> {
    #[account(mut)]
    pub recipient: Signer<'info>,
    /// CHECK: Only used for PDA derivation and validation - needs to be mutable for rent return
    #[account(mut)]
    pub sender: AccountInfo<'info>,
    #[account(
        mut,
        seeds = [recipient.key().as_ref(), sender.key().as_ref()],
        bump,
        has_one = recipient,
        has_one = sender
    )]
    pub balance_holder_pda: Account<'info, BalanceHolderPDA>,
    pub rent: Sysvar<'info, Rent>,
}

#[error_code]
pub enum ErrorCode {
    #[msg("Amount must be greater than zero")]
    InvalidAmount,
    #[msg("Insufficient funds in PDA")]
    InsufficientFunds,
}
