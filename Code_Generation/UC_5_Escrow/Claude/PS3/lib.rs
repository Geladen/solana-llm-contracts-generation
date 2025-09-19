use anchor_lang::prelude::*;

declare_id!("4LaRe8u6cXmRsvwuAtuZwRGUL339nMoCan96hq9UNqiM");

#[program]
pub mod escrow {
    use super::*;

    pub fn initialize(
        ctx: Context<InitializeCtx>, 
        amount_in_lamports: u64, 
        escrow_name: String
    ) -> Result<()> {
        // Validate amount is not zero
        require!(amount_in_lamports > 0, EscrowError::InvalidAmount);

        // Initialize escrow info
        ctx.accounts.escrow_info.seller = ctx.accounts.seller.key();
        ctx.accounts.escrow_info.buyer = ctx.accounts.buyer.key();
        ctx.accounts.escrow_info.amount_in_lamports = amount_in_lamports;
        ctx.accounts.escrow_info.state = State::WaitDeposit;

        msg!("Escrow '{}' initialized by seller {} for buyer {} with amount: {} lamports", 
             escrow_name, 
             ctx.accounts.seller.key(), 
             ctx.accounts.buyer.key(), 
             amount_in_lamports);

        Ok(())
    }

    pub fn deposit(ctx: Context<DepositCtx>, escrow_name: String) -> Result<()> {
        // Validate current state
        require!(ctx.accounts.escrow_info.state == State::WaitDeposit, EscrowError::InvalidState);
        
        // Validate buyer matches
        require!(ctx.accounts.buyer.key() == ctx.accounts.escrow_info.buyer, EscrowError::InvalidBuyer);
        
        // Get amount before borrowing mutably
        let amount_to_transfer = ctx.accounts.escrow_info.amount_in_lamports;
        
        // Transfer lamports from buyer to escrow PDA
        let transfer_instruction = anchor_lang::system_program::Transfer {
            from: ctx.accounts.buyer.to_account_info(),
            to: ctx.accounts.escrow_info.to_account_info(),
        };
        
        let cpi_context = CpiContext::new(
            ctx.accounts.system_program.to_account_info(),
            transfer_instruction,
        );
        
        anchor_lang::system_program::transfer(cpi_context, amount_to_transfer)?;
        
        // Update state
        ctx.accounts.escrow_info.state = State::WaitRecipient;
        
        msg!("Buyer {} deposited {} lamports to escrow '{}'", 
             ctx.accounts.buyer.key(), 
             amount_to_transfer,
             escrow_name);

        Ok(())
    }

    pub fn pay(ctx: Context<PayCtx>, escrow_name: String) -> Result<()> {
        // Validate current state
        require!(ctx.accounts.escrow_info.state == State::WaitRecipient, EscrowError::InvalidState);
        
        // Validate buyer is calling this function
        require!(ctx.accounts.buyer.key() == ctx.accounts.escrow_info.buyer, EscrowError::InvalidBuyer);
        
        // Get current PDA balance before modifying anything
        let escrow_balance = ctx.accounts.escrow_info.to_account_info().lamports();
        
        // Transfer entire PDA balance to seller
        **ctx.accounts.escrow_info.to_account_info().try_borrow_mut_lamports()? -= escrow_balance;
        **ctx.accounts.seller.to_account_info().try_borrow_mut_lamports()? += escrow_balance;
        
        // Update state
        ctx.accounts.escrow_info.state = State::Closed;
        
        msg!("Buyer {} paid {} lamports to seller {} for escrow '{}'", 
             ctx.accounts.buyer.key(), 
             escrow_balance,
             ctx.accounts.seller.key(),
             escrow_name);

        Ok(())
    }

    pub fn refund(ctx: Context<RefundCtx>, escrow_name: String) -> Result<()> {
        // Validate current state
        require!(ctx.accounts.escrow_info.state == State::WaitRecipient, EscrowError::InvalidState);
        
        // Validate seller is calling this function
        require!(ctx.accounts.seller.key() == ctx.accounts.escrow_info.seller, EscrowError::InvalidSeller);
        
        // Get values before modifying anything
        let escrow_balance = ctx.accounts.escrow_info.to_account_info().lamports();
        let refund_amount = ctx.accounts.escrow_info.amount_in_lamports;
        
        // Transfer deposited amount back to buyer
        **ctx.accounts.escrow_info.to_account_info().try_borrow_mut_lamports()? -= refund_amount;
        **ctx.accounts.buyer.to_account_info().try_borrow_mut_lamports()? += refund_amount;
        
        // Return remaining rent lamports to seller
        let remaining_lamports = escrow_balance - refund_amount;
        if remaining_lamports > 0 {
            **ctx.accounts.escrow_info.to_account_info().try_borrow_mut_lamports()? -= remaining_lamports;
            **ctx.accounts.seller.to_account_info().try_borrow_mut_lamports()? += remaining_lamports;
        }
        
        // Update state
        ctx.accounts.escrow_info.state = State::Closed;
        
        msg!("Seller {} refunded {} lamports to buyer {} for escrow '{}'", 
             ctx.accounts.seller.key(), 
             refund_amount,
             ctx.accounts.buyer.key(),
             escrow_name);

        Ok(())
    }
}

// Context Structs
#[derive(Accounts)]
#[instruction(amount_in_lamports: u64, escrow_name: String)]
pub struct InitializeCtx<'info> {
    #[account(mut)]
    pub seller: Signer<'info>,
    
    /// CHECK: This account is only used for PDA derivation, validation handled in seeds
    pub buyer: AccountInfo<'info>,
    
    #[account(
        init,
        payer = seller,
        space = 8 + EscrowInfo::INIT_SPACE,
        seeds = [escrow_name.as_ref(), seller.key().as_ref(), buyer.key().as_ref()],
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
    
    /// CHECK: This account is only used for validation, checked against escrow_info.seller
    pub seller: AccountInfo<'info>,
    
    #[account(
        mut,
        seeds = [escrow_name.as_ref(), seller.key().as_ref(), buyer.key().as_ref()],
        bump
    )]
    pub escrow_info: Account<'info, EscrowInfo>,
    
    pub system_program: Program<'info, System>,
}

#[derive(Accounts)]
#[instruction(escrow_name: String)]
pub struct PayCtx<'info> {
    pub buyer: Signer<'info>,
    
    #[account(mut)]
    /// CHECK: This account receives the payment, validated against escrow_info.seller
    pub seller: AccountInfo<'info>,
    
    #[account(
        mut,
        seeds = [escrow_name.as_ref(), seller.key().as_ref(), buyer.key().as_ref()],
        bump
    )]
    pub escrow_info: Account<'info, EscrowInfo>,
}

#[derive(Accounts)]
#[instruction(escrow_name: String)]
pub struct RefundCtx<'info> {
    #[account(mut)]
    pub seller: Signer<'info>,
    
    #[account(mut)]
    /// CHECK: This account receives the refund, validated against escrow_info.buyer
    pub buyer: AccountInfo<'info>,
    
    #[account(
        mut,
        seeds = [escrow_name.as_ref(), seller.key().as_ref(), buyer.key().as_ref()],
        bump
    )]
    pub escrow_info: Account<'info, EscrowInfo>,
}

// State Account
#[account]
#[derive(InitSpace)]
pub struct EscrowInfo {
    pub seller: Pubkey,
    pub buyer: Pubkey,
    pub amount_in_lamports: u64,
    pub state: State,
}

// State Enum
#[derive(AnchorSerialize, AnchorDeserialize, PartialEq, Eq, Clone, InitSpace)]
pub enum State {
    WaitDeposit,
    WaitRecipient,
    Closed,
}

// Custom Errors
#[error_code]
pub enum EscrowError {
    #[msg("Invalid amount: amount must be greater than 0")]
    InvalidAmount,
    
    #[msg("Invalid state for this operation")]
    InvalidState,
    
    #[msg("Invalid buyer: caller does not match escrow buyer")]
    InvalidBuyer,
    
    #[msg("Invalid seller: caller does not match escrow seller")]
    InvalidSeller,
}