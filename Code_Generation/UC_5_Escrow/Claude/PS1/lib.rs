use anchor_lang::prelude::*;

declare_id!("5HW9RUa4vgexN3MqCG5aNYCwWBwAy8ut6UFjPZfiAHZz");

#[program]
pub mod escrow {
    use super::*;

    pub fn initialize(
        ctx: Context<InitializeCtx>,
        amount_in_lamports: u64,
        _escrow_name: String,
    ) -> Result<()> {
        // Validate amount is not zero
        require!(amount_in_lamports > 0, EscrowError::ZeroAmount);

        let escrow_info = &mut ctx.accounts.escrow_info;
        
        // Initialize escrow state
        escrow_info.seller = ctx.accounts.seller.key();
        escrow_info.buyer = ctx.accounts.buyer.key();
        escrow_info.amount_in_lamports = amount_in_lamports;
        escrow_info.state = State::WaitDeposit;

        msg!("Escrow initialized: {} lamports between seller {} and buyer {}",
             amount_in_lamports,
             ctx.accounts.seller.key(),
             ctx.accounts.buyer.key());

        Ok(())
    }

    pub fn deposit(ctx: Context<DepositCtx>, _escrow_name: String) -> Result<()> {
        // Store all needed values before any mutable borrows
        let amount_to_transfer = ctx.accounts.escrow_info.amount_in_lamports;
        let buyer_key = ctx.accounts.escrow_info.buyer;
        let current_state = ctx.accounts.escrow_info.state.clone();

        // Validate state
        require!(
            current_state == State::WaitDeposit,
            EscrowError::InvalidState
        );

        // Validate buyer matches
        require!(
            ctx.accounts.buyer.key() == buyer_key,
            EscrowError::UnauthorizedBuyer
        );

        // Transfer lamports from buyer to escrow PDA
        let transfer_instruction = anchor_lang::system_program::Transfer {
            from: ctx.accounts.buyer.to_account_info(),
            to: ctx.accounts.escrow_info.to_account_info(),
        };

        let cpi_ctx = CpiContext::new(
            ctx.accounts.system_program.to_account_info(),
            transfer_instruction,
        );

        anchor_lang::system_program::transfer(cpi_ctx, amount_to_transfer)?;

        // Now safely update state with mutable borrow
        let escrow_info = &mut ctx.accounts.escrow_info;
        escrow_info.state = State::WaitRecipient;

        msg!("Buyer deposited {} lamports to escrow", amount_to_transfer);

        Ok(())
    }

    pub fn pay(ctx: Context<PayCtx>, _escrow_name: String) -> Result<()> {
        // Get needed data before any mutable operations
        let buyer_key = ctx.accounts.escrow_info.buyer;
        let current_state = ctx.accounts.escrow_info.state.clone();

        // Validate state
        require!(
            current_state == State::WaitRecipient,
            EscrowError::InvalidState
        );

        // Validate buyer
        require!(
            ctx.accounts.buyer.key() == buyer_key,
            EscrowError::UnauthorizedBuyer
        );

        // Get escrow account balance - transfer everything to seller
        let escrow_account_info = ctx.accounts.escrow_info.to_account_info();
        let escrow_balance = escrow_account_info.lamports();

        // Transfer all funds from escrow PDA to seller (closing the account)
        {
            let seller_account_info = ctx.accounts.seller.to_account_info();
            let mut escrow_lamports = escrow_account_info.try_borrow_mut_lamports()?;
            let mut seller_lamports = seller_account_info.try_borrow_mut_lamports()?;
            **seller_lamports += escrow_balance;
            **escrow_lamports = 0;
        }

        // Now update state after lamport operations complete
        let escrow_info = &mut ctx.accounts.escrow_info;
        escrow_info.state = State::Closed;

        msg!("Payment of {} lamports transferred to seller", escrow_balance);

        Ok(())
    }

    pub fn refund(ctx: Context<RefundCtx>, _escrow_name: String) -> Result<()> {
        // Get needed data before any mutable operations
        let seller_key = ctx.accounts.escrow_info.seller;
        let current_state = ctx.accounts.escrow_info.state.clone();
        let refund_amount = ctx.accounts.escrow_info.amount_in_lamports;

        // Validate state
        require!(
            current_state == State::WaitRecipient,
            EscrowError::InvalidState
        );

        // Validate seller
        require!(
            ctx.accounts.seller.key() == seller_key,
            EscrowError::UnauthorizedSeller
        );
        
        // Get account info and calculate remaining balance
        let escrow_account_info = ctx.accounts.escrow_info.to_account_info();
        let buyer_account_info = ctx.accounts.buyer.to_account_info();
        let seller_account_info = ctx.accounts.seller.to_account_info();
        let escrow_balance = escrow_account_info.lamports();
        let remaining_balance = escrow_balance - refund_amount;
        
        // Perform all transfers - refund to buyer, rent to seller, close escrow
        {
            let mut escrow_lamports = escrow_account_info.try_borrow_mut_lamports()?;
            let mut buyer_lamports = buyer_account_info.try_borrow_mut_lamports()?;
            let mut seller_lamports = seller_account_info.try_borrow_mut_lamports()?;
            
            // Transfer deposited amount to buyer
            **buyer_lamports += refund_amount;
            
            // Transfer remaining balance (rent) to seller
            **seller_lamports += remaining_balance;
            
            // Close escrow account
            **escrow_lamports = 0;
        }

        // Now update state after all lamport operations complete
        let escrow_info = &mut ctx.accounts.escrow_info;
        escrow_info.state = State::Closed;

        msg!("Refunded {} lamports to buyer and {} lamports rent to seller", 
             refund_amount, remaining_balance);

        Ok(())
    }
}

// Context Structures
#[derive(Accounts)]
#[instruction(amount_in_lamports: u64, escrow_name: String)]
pub struct InitializeCtx<'info> {
    #[account(mut)]
    pub seller: Signer<'info>,
    
    /// CHECK: Used only for PDA derivation, not modified in this instruction
    pub buyer: AccountInfo<'info>,
    
    #[account(
        init,
        payer = seller,
        space = 8 + EscrowInfo::LEN,
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
    
    /// CHECK: Used for validation against escrow_info.seller
    pub seller: AccountInfo<'info>,
    
    #[account(
        mut,
        seeds = [escrow_name.as_ref(), seller.key().as_ref(), buyer.key().as_ref()],
        bump,
        constraint = escrow_info.seller == seller.key() @ EscrowError::InvalidSeller
    )]
    pub escrow_info: Account<'info, EscrowInfo>,
    
    pub system_program: Program<'info, System>,
}

#[derive(Accounts)]
#[instruction(escrow_name: String)]
pub struct PayCtx<'info> {
    pub buyer: Signer<'info>,
    
    /// CHECK: Validated against escrow_info.seller, receives payment
    #[account(mut)]
    pub seller: AccountInfo<'info>,
    
    #[account(
        mut,
        seeds = [escrow_name.as_ref(), seller.key().as_ref(), buyer.key().as_ref()],
        bump,
        constraint = escrow_info.seller == seller.key() @ EscrowError::InvalidSeller
    )]
    pub escrow_info: Account<'info, EscrowInfo>,
}

#[derive(Accounts)]
#[instruction(escrow_name: String)]
pub struct RefundCtx<'info> {
    #[account(mut)]
    pub seller: Signer<'info>,
    
    /// CHECK: Validated against escrow_info.buyer, receives refund
    #[account(mut)]
    pub buyer: AccountInfo<'info>,
    
    #[account(
        mut,
        seeds = [escrow_name.as_ref(), seller.key().as_ref(), buyer.key().as_ref()],
        bump,
        constraint = escrow_info.seller == seller.key() @ EscrowError::InvalidSeller,
        constraint = escrow_info.buyer == buyer.key() @ EscrowError::InvalidBuyer
    )]
    pub escrow_info: Account<'info, EscrowInfo>,
}

// Account Structure
#[account]
pub struct EscrowInfo {
    pub seller: Pubkey,
    pub buyer: Pubkey,
    pub amount_in_lamports: u64,
    pub state: State,
}

impl EscrowInfo {
    pub const LEN: usize = 32 + 32 + 8 + 1; // seller + buyer + amount + state
}

// State Enum
#[derive(AnchorSerialize, AnchorDeserialize, Clone, PartialEq, Eq)]
pub enum State {
    WaitDeposit,
    WaitRecipient,
    Closed,
}

// Error Types
#[error_code]
pub enum EscrowError {
    #[msg("Amount cannot be zero")]
    ZeroAmount,
    #[msg("Invalid escrow state for this operation")]
    InvalidState,
    #[msg("Unauthorized buyer")]
    UnauthorizedBuyer,
    #[msg("Unauthorized seller")]
    UnauthorizedSeller,
    #[msg("Invalid seller in escrow")]
    InvalidSeller,
    #[msg("Invalid buyer in escrow")]
    InvalidBuyer,
}