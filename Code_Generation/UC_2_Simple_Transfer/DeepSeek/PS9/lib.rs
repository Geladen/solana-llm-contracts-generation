use anchor_lang::prelude::*;
use anchor_lang::system_program;

declare_id!("pJJDwW7P7ckwKWcZYxPabaSP3xoYV9bfA3ZwLXT6CLh");

#[program]
pub mod simple_transfer {
    use super::*;

    pub fn deposit(ctx: Context<DepositCtx>, amount_to_deposit: u64) -> Result<()> {
        require!(amount_to_deposit > 0, TransferError::InvalidAmount);

        // Get PDA account info before mutable borrow
        let pda_account_info = ctx.accounts.balance_holder_pda.to_account_info();
        let sender_key = ctx.accounts.sender.key();
        let recipient_key = ctx.accounts.recipient.key();
        
        let balance_holder = &mut ctx.accounts.balance_holder_pda;
        
        if balance_holder.amount == 0 && balance_holder.sender == Pubkey::default() {
            balance_holder.sender = sender_key;
            balance_holder.recipient = recipient_key;
            balance_holder.amount = amount_to_deposit;
        } else {
            balance_holder.amount = balance_holder.amount
                .checked_add(amount_to_deposit)
                .ok_or(TransferError::AmountOverflow)?;
        }

        let cpi_context = CpiContext::new(
            ctx.accounts.system_program.to_account_info(),
            system_program::Transfer {
                from: ctx.accounts.sender.to_account_info(),
                to: pda_account_info,
            },
        );
        system_program::transfer(cpi_context, amount_to_deposit)?;

        emit!(DepositEvent {
            sender: sender_key,
            recipient: recipient_key,
            amount: amount_to_deposit,
            new_balance: balance_holder.amount
        });

        Ok(())
    }

    pub fn withdraw(ctx: Context<WithdrawCtx>, amount_to_withdraw: u64) -> Result<()> {
        require!(amount_to_withdraw > 0, TransferError::InvalidAmount);
        
        // Store all necessary data in local variables first
        let current_amount = ctx.accounts.balance_holder_pda.amount;
        let sender_pubkey = ctx.accounts.balance_holder_pda.sender;
        let recipient_pubkey = ctx.accounts.balance_holder_pda.recipient;
        
        require!(
            current_amount >= amount_to_withdraw,
            TransferError::InsufficientFunds
        );

        let remaining_balance = current_amount
            .checked_sub(amount_to_withdraw)
            .ok_or(TransferError::AmountUnderflow)?;

        // Update the account state
        ctx.accounts.balance_holder_pda.amount = remaining_balance;

        // Get fresh account info references for lamport manipulation
        let pda_info = &ctx.accounts.balance_holder_pda.to_account_info();
        let recipient_info = &ctx.accounts.recipient.to_account_info();
        
        // Check available lamports
        let available_lamports = pda_info.lamports();
        require!(
            available_lamports >= amount_to_withdraw,
            TransferError::InsufficientLamports
        );

        // Transfer lamports directly (this is safe for PDAs)
        **pda_info.try_borrow_mut_lamports()? -= amount_to_withdraw;
        **recipient_info.try_borrow_mut_lamports()? += amount_to_withdraw;

        // Handle account closure if balance reaches zero
        if remaining_balance == 0 {
            // Get fresh references for closure operations
            let pda_info = &ctx.accounts.balance_holder_pda.to_account_info();
            let sender_info = &ctx.accounts.sender.to_account_info();
            
            // Calculate rent exemption
            let rent = Rent::get()?;
            let rent_lamports = rent.minimum_balance(pda_info.data_len());
            let remaining_lamports = pda_info.lamports();
            
            // Return rent exemption to sender if there are sufficient lamports
            if remaining_lamports >= rent_lamports {
                let refund_amount = remaining_lamports;
                **pda_info.try_borrow_mut_lamports()? -= refund_amount;
                **sender_info.try_borrow_mut_lamports()? += refund_amount;
            }
            
            // Zero out the account data
            let mut data = pda_info.try_borrow_mut_data()?;
            for byte in data.iter_mut() {
                *byte = 0;
            }
            
            // The account will be garbage collected by the runtime when zero lamports remain
        }

        emit!(WithdrawEvent {
            sender: sender_pubkey,
            recipient: recipient_pubkey,
            amount: amount_to_withdraw,
            remaining_balance
        });

        Ok(())
    }
}

#[derive(Accounts)]
pub struct DepositCtx<'info> {
    #[account(mut)]
    pub sender: Signer<'info>,
    
    /// CHECK: Used for PDA derivation only
    pub recipient: AccountInfo<'info>,
    
    #[account(
        init_if_needed,
        payer = sender,
        space = 8 + BalanceHolderPDA::INIT_SPACE,
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
    
    /// CHECK: Used for PDA derivation and to receive rent refund
    #[account(mut)]
    pub sender: AccountInfo<'info>,
    
    #[account(
        mut,
        seeds = [recipient.key().as_ref(), sender.key().as_ref()],
        bump,
        constraint = balance_holder_pda.recipient == recipient.key() @ TransferError::InvalidRecipient,
        constraint = balance_holder_pda.sender == sender.key() @ TransferError::InvalidSender
    )]
    pub balance_holder_pda: Account<'info, BalanceHolderPDA>,
    
    pub system_program: Program<'info, System>,
    pub rent: Sysvar<'info, Rent>,
}

#[account]
#[derive(InitSpace)]
pub struct BalanceHolderPDA {
    pub sender: Pubkey,
    pub recipient: Pubkey,
    pub amount: u64,
}

#[event]
pub struct DepositEvent {
    pub sender: Pubkey,
    pub recipient: Pubkey,
    pub amount: u64,
    pub new_balance: u64,
}

#[event]
pub struct WithdrawEvent {
    pub sender: Pubkey,
    pub recipient: Pubkey,
    pub amount: u64,
    pub remaining_balance: u64,
}

#[error_code]
pub enum TransferError {
    #[msg("Amount must be greater than zero")]
    InvalidAmount,
    #[msg("Insufficient funds in contract")]
    InsufficientFunds,
    #[msg("Invalid recipient for this contract")]
    InvalidRecipient,
    #[msg("Invalid sender for this contract")]
    InvalidSender,
    #[msg("Amount overflow occurred")]
    AmountOverflow,
    #[msg("Amount underflow occurred")]
    AmountUnderflow,
    #[msg("Insufficient lamports in PDA account")]
    InsufficientLamports,
}