use anchor_lang::prelude::*;

declare_id!("5W5qJAay3CcokJDv31h4YrhQtHUFYituLtjwiZwDtTUb");

#[program]
pub mod escrow {
    use super::*;

    /// Initialize the escrow contract
    /// Must be called by the seller with the amount and escrow name
    pub fn initialize(
        ctx: Context<InitializeCtx>, 
        amount_in_lamports: u64, 
        escrow_name: String
    ) -> Result<()> {
        // Validate amount is not zero
        require!(amount_in_lamports > 0, EscrowError::InvalidAmount);
        
        // Validate escrow name length to prevent excessive storage costs
        require!(escrow_name.len() <= 50, EscrowError::EscrowNameTooLong);

        let escrow_info = &mut ctx.accounts.escrow_info;
        
        // Initialize escrow state
        escrow_info.seller = ctx.accounts.seller.key();
        escrow_info.buyer = ctx.accounts.buyer.key();
        escrow_info.amount_in_lamports = amount_in_lamports;
        escrow_info.state = State::WaitDeposit;

        msg!("Escrow initialized: {} lamports between {} and {}", 
             amount_in_lamports, 
             ctx.accounts.seller.key(), 
             ctx.accounts.buyer.key());

        Ok(())
    }

    /// Deposit funds into escrow
    /// Must be called by the buyer to deposit the exact amount
    pub fn deposit(ctx: Context<DepositCtx>, _escrow_name: String) -> Result<()> {
        // Validate state
        require!(
            ctx.accounts.escrow_info.state == State::WaitDeposit, 
            EscrowError::InvalidState
        );

        // Validate buyer matches
        require!(
            ctx.accounts.escrow_info.buyer == ctx.accounts.buyer.key(), 
            EscrowError::UnauthorizedBuyer
        );

        // Validate seller matches for additional security
        require!(
            ctx.accounts.escrow_info.seller == ctx.accounts.seller.key(), 
            EscrowError::UnauthorizedSeller
        );

        let amount = ctx.accounts.escrow_info.amount_in_lamports;
        let buyer_key = ctx.accounts.buyer.key();
        let escrow_key = ctx.accounts.escrow_info.key();

        // Transfer lamports from buyer to escrow PDA
        let ix = anchor_lang::solana_program::system_instruction::transfer(
            &buyer_key,
            &escrow_key,
            amount,
        );

        anchor_lang::solana_program::program::invoke(
            &ix,
            &[
                ctx.accounts.buyer.to_account_info(),
                ctx.accounts.escrow_info.to_account_info(),
            ],
        )?;

        // Update state - now we can borrow mutably
        ctx.accounts.escrow_info.state = State::WaitRecipient;

        msg!("Buyer deposited {} lamports into escrow", amount);

        Ok(())
    }

    /// Pay the seller
    /// Must be called by the buyer to release funds to seller
    pub fn pay(ctx: Context<PayCtx>, _escrow_name: String) -> Result<()> {
        // Validate state
        require!(
            ctx.accounts.escrow_info.state == State::WaitRecipient, 
            EscrowError::InvalidState
        );

        // Validate buyer authorization
        require!(
            ctx.accounts.escrow_info.buyer == ctx.accounts.buyer.key(), 
            EscrowError::UnauthorizedBuyer
        );

        // Validate seller matches
        require!(
            ctx.accounts.escrow_info.seller == ctx.accounts.seller.key(), 
            EscrowError::UnauthorizedSeller
        );

        // Get escrow balance before any mutable operations
        let escrow_balance = ctx.accounts.escrow_info.to_account_info().lamports();
        
        // Transfer all lamports from escrow PDA to seller
        **ctx.accounts.escrow_info.to_account_info().try_borrow_mut_lamports()? -= escrow_balance;
        **ctx.accounts.seller.to_account_info().try_borrow_mut_lamports()? += escrow_balance;

        // Update state to closed
        ctx.accounts.escrow_info.state = State::Closed;

        msg!("Payment of {} lamports sent to seller", escrow_balance);

        Ok(())
    }

    /// Refund the buyer
    /// Must be called by the seller to return funds to buyer
    pub fn refund(ctx: Context<RefundCtx>, _escrow_name: String) -> Result<()> {
        // Validate state
        require!(
            ctx.accounts.escrow_info.state == State::WaitRecipient, 
            EscrowError::InvalidState
        );

        // Validate seller authorization
        require!(
            ctx.accounts.escrow_info.seller == ctx.accounts.seller.key(), 
            EscrowError::UnauthorizedSeller
        );

        // Validate buyer matches
        require!(
            ctx.accounts.escrow_info.buyer == ctx.accounts.buyer.key(), 
            EscrowError::UnauthorizedBuyer
        );

        let deposited_amount = ctx.accounts.escrow_info.amount_in_lamports;
        // Get escrow balance before any mutable operations
        let escrow_balance = ctx.accounts.escrow_info.to_account_info().lamports();
        
        // Calculate rent exempt amount (approximate PDA account rent)
        let rent_lamports = escrow_balance - deposited_amount;

        // Transfer deposited amount back to buyer
        **ctx.accounts.escrow_info.to_account_info().try_borrow_mut_lamports()? -= deposited_amount;
        **ctx.accounts.buyer.to_account_info().try_borrow_mut_lamports()? += deposited_amount;

        // Transfer remaining rent lamports to seller
        if rent_lamports > 0 {
            **ctx.accounts.escrow_info.to_account_info().try_borrow_mut_lamports()? -= rent_lamports;
            **ctx.accounts.seller.to_account_info().try_borrow_mut_lamports()? += rent_lamports;
        }

        // Update state to closed
        ctx.accounts.escrow_info.state = State::Closed;

        msg!("Refunded {} lamports to buyer, {} lamports rent to seller", 
             deposited_amount, rent_lamports);

        Ok(())
    }
}

// Context structs for each instruction

#[derive(Accounts)]
#[instruction(amount_in_lamports: u64, escrow_name: String)]
pub struct InitializeCtx<'info> {
    #[account(mut)]
    pub seller: Signer<'info>,
    
    /// CHECK: buyer account for PDA derivation
    pub buyer: AccountInfo<'info>,
    
    #[account(
        init,
        payer = seller,
        space = EscrowInfo::LEN,
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
    
    /// CHECK: seller account for validation
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
    /// CHECK: seller account to receive funds
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
    /// CHECK: buyer account to receive refund
    pub buyer: AccountInfo<'info>,
    
    #[account(
        mut,
        seeds = [escrow_name.as_ref(), seller.key().as_ref(), buyer.key().as_ref()],
        bump
    )]
    pub escrow_info: Account<'info, EscrowInfo>,
}

// Account data structure

#[account]
pub struct EscrowInfo {
    pub seller: Pubkey,
    pub buyer: Pubkey,
    pub amount_in_lamports: u64,
    pub state: State,
}

impl EscrowInfo {
    pub const LEN: usize = 8 + 32 + 32 + 8 + 1; // discriminator + seller + buyer + amount + state
}

// State enum

#[derive(AnchorSerialize, AnchorDeserialize, Clone, Copy, Debug, PartialEq)]
pub enum State {
    WaitDeposit,
    WaitRecipient,
    Closed,
}

// Custom error types

#[error_code]
pub enum EscrowError {
    #[msg("Invalid amount: must be greater than zero")]
    InvalidAmount,
    
    #[msg("Invalid state for this operation")]
    InvalidState,
    
    #[msg("Unauthorized buyer")]
    UnauthorizedBuyer,
    
    #[msg("Unauthorized seller")]
    UnauthorizedSeller,
    
    #[msg("Escrow name too long")]
    EscrowNameTooLong,
}