use anchor_lang::prelude::*;
use anchor_lang::system_program::{transfer, Transfer};

declare_id!("FcKTHyfuGjwjztkgQginLKsZGUsDachGbMNZFG9JXQLo");

#[program]
pub mod transfer_contract {
    use super::*;

    /// Deposit funds into the contract (initial deposit that creates the PDA)
    /// Can only be called by the sender/owner
    pub fn deposit(ctx: Context<DepositCtx>, amount_to_deposit: u64) -> Result<()> {
        // Validate amount is not zero
        require!(amount_to_deposit > 0, ErrorCode::ZeroAmountNotAllowed);

        // Get accounts
        let sender = &mut ctx.accounts.sender;
        let balance_holder_pda = &mut ctx.accounts.balance_holder_pda;
        let system_program = &ctx.accounts.system_program;

        // Transfer lamports from sender to PDA
        let transfer_instruction = Transfer {
            from: sender.to_account_info(),
            to: balance_holder_pda.to_account_info(),
        };

        let cpi_ctx = CpiContext::new(
            system_program.to_account_info(),
            transfer_instruction,
        );

        transfer(cpi_ctx, amount_to_deposit)?;

        // Initialize PDA state (this is the first deposit)
        balance_holder_pda.sender = sender.key();
        balance_holder_pda.recipient = ctx.accounts.recipient.key();
        balance_holder_pda.amount = amount_to_deposit;

        msg!("Deposited {} lamports. Total balance: {}", amount_to_deposit, balance_holder_pda.amount);

        Ok(())
    }

    /// Add more funds to an existing contract
    /// Can only be called by the sender/owner
    pub fn add_deposit(ctx: Context<AddDepositCtx>, amount_to_deposit: u64) -> Result<()> {
        // Validate amount is not zero
        require!(amount_to_deposit > 0, ErrorCode::ZeroAmountNotAllowed);

        // Get accounts
        let sender = &mut ctx.accounts.sender;
        let balance_holder_pda = &mut ctx.accounts.balance_holder_pda;
        let system_program = &ctx.accounts.system_program;

        // Transfer lamports from sender to PDA
        let transfer_instruction = Transfer {
            from: sender.to_account_info(),
            to: balance_holder_pda.to_account_info(),
        };

        let cpi_ctx = CpiContext::new(
            system_program.to_account_info(),
            transfer_instruction,
        );

        transfer(cpi_ctx, amount_to_deposit)?;

        // Update PDA state
        balance_holder_pda.amount += amount_to_deposit;

        msg!("Added {} lamports. Total balance: {}", amount_to_deposit, balance_holder_pda.amount);

        Ok(())
    }

    /// Withdraw funds from the contract
    /// Can only be called by the designated recipient
    pub fn withdraw(ctx: Context<WithdrawCtx>, amount_to_withdraw: u64) -> Result<()> {
        // Validate amount is not zero
        require!(amount_to_withdraw > 0, ErrorCode::ZeroAmountNotAllowed);

        let balance_holder_pda = &mut ctx.accounts.balance_holder_pda;
        let recipient = &mut ctx.accounts.recipient;

        // Validate sufficient balance
        require!(
            balance_holder_pda.amount >= amount_to_withdraw,
            ErrorCode::InsufficientBalance
        );

        // Validate recipient matches the one stored in PDA
        require!(
            balance_holder_pda.recipient == recipient.key(),
            ErrorCode::UnauthorizedRecipient
        );

        // Validate sender matches the one stored in PDA
        require!(
            balance_holder_pda.sender == ctx.accounts.sender.key(),
            ErrorCode::InvalidSender
        );

        // Calculate remaining balance after withdrawal
        let remaining_balance = balance_holder_pda.amount - amount_to_withdraw;

        // Transfer lamports from PDA to recipient
        **balance_holder_pda.to_account_info().try_borrow_mut_lamports()? -= amount_to_withdraw;
        **recipient.to_account_info().try_borrow_mut_lamports()? += amount_to_withdraw;

        // Update PDA state
        balance_holder_pda.amount = remaining_balance;

        msg!("Withdrawn {} lamports. Remaining balance: {}", amount_to_withdraw, remaining_balance);

        // If balance reaches zero, close the account and return remaining lamports to sender
        if remaining_balance == 0 {
            let sender_info = &ctx.accounts.sender.to_account_info();
            let pda_info = balance_holder_pda.to_account_info();
            
            // Transfer remaining lamports (rent) back to sender
            let pda_lamports = pda_info.lamports();
            **pda_info.try_borrow_mut_lamports()? = 0;
            **sender_info.try_borrow_mut_lamports()? += pda_lamports;

            msg!("PDA closed. Returned {} lamports to sender", pda_lamports);
        }

        Ok(())
    }
}

#[derive(Accounts)]
pub struct DepositCtx<'info> {
    #[account(mut)]
    pub sender: Signer<'info>,
    
    /// CHECK: This account is used for PDA derivation and validation only
    pub recipient: AccountInfo<'info>,
    
    #[account(
        init,
        payer = sender,
        space = 8 + 32 + 32 + 8, // discriminator + sender + recipient + amount
        seeds = [recipient.key().as_ref(), sender.key().as_ref()],
        bump
    )]
    pub balance_holder_pda: Account<'info, BalanceHolderPDA>,
    
    pub system_program: Program<'info, System>,
}

#[derive(Accounts)]
pub struct AddDepositCtx<'info> {
    #[account(mut)]
    pub sender: Signer<'info>,
    
    /// CHECK: This account is used for PDA derivation and validation only
    pub recipient: AccountInfo<'info>,
    
    #[account(
        mut,
        seeds = [recipient.key().as_ref(), sender.key().as_ref()],
        bump,
        constraint = balance_holder_pda.sender == sender.key() @ ErrorCode::InvalidSender
    )]
    pub balance_holder_pda: Account<'info, BalanceHolderPDA>,
    
    pub system_program: Program<'info, System>,
}

#[derive(Accounts)]
pub struct WithdrawCtx<'info> {
    #[account(mut)]
    pub recipient: Signer<'info>,
    
    /// CHECK: This account is used for PDA validation and receiving returned lamports
    #[account(mut)]
    pub sender: AccountInfo<'info>,
    
    #[account(
        mut,
        seeds = [recipient.key().as_ref(), sender.key().as_ref()],
        bump,
        constraint = balance_holder_pda.recipient == recipient.key() @ ErrorCode::UnauthorizedRecipient,
        constraint = balance_holder_pda.sender == sender.key() @ ErrorCode::InvalidSender
    )]
    pub balance_holder_pda: Account<'info, BalanceHolderPDA>,
    
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
    #[msg("Amount must be greater than zero")]
    ZeroAmountNotAllowed,
    
    #[msg("Insufficient balance for withdrawal")]
    InsufficientBalance,
    
    #[msg("Unauthorized recipient")]
    UnauthorizedRecipient,
    
    #[msg("Invalid sender")]
    InvalidSender,
}
