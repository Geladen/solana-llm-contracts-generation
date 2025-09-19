use anchor_lang::prelude::*;
use borsh::{BorshDeserialize, BorshSerialize};

declare_id!("7emMvrBu97UpXYpgVH7Nyvd2Wnkguze75c6TSPwGUWbw");

#[program]
pub mod escrow_contract {
    use super::*;

    pub fn initialize(
        ctx: Context<InitializeCtx>,
        amount_in_lamports: u64,
        escrow_name: String,
    ) -> Result<()> {
        require!(amount_in_lamports > 0, EscrowError::InvalidAmount);
        
        let escrow_info = &mut ctx.accounts.escrow_info;
        escrow_info.seller = ctx.accounts.seller.key();
        escrow_info.buyer = ctx.accounts.buyer.key();
        escrow_info.amount_in_lamports = amount_in_lamports;
        escrow_info.state = 0; // WaitDeposit

        Ok(())
    }

    pub fn deposit(ctx: Context<DepositCtx>, escrow_name: String) -> Result<()> {
        require!(
            ctx.accounts.escrow_info.state == 0, // WaitDeposit
            EscrowError::InvalidState
        );
        require!(
            ctx.accounts.escrow_info.buyer == ctx.accounts.buyer.key(),
            EscrowError::InvalidBuyer
        );

        let transfer_instruction = anchor_lang::system_program::Transfer {
            from: ctx.accounts.buyer.to_account_info(),
            to: ctx.accounts.escrow_info.to_account_info(),
        };

        anchor_lang::system_program::transfer(
            CpiContext::new(
                ctx.accounts.system_program.to_account_info(),
                transfer_instruction,
            ),
            ctx.accounts.escrow_info.amount_in_lamports,
        )?;

        ctx.accounts.escrow_info.state = 1; // WaitRecipient

        Ok(())
    }

    pub fn pay(ctx: Context<PayCtx>, escrow_name: String) -> Result<()> {
        require!(
            ctx.accounts.escrow_info.state == 1, // WaitRecipient
            EscrowError::InvalidState
        );

        let escrow_balance = ctx.accounts.escrow_info.to_account_info().lamports();
        
        // Transfer entire balance to seller
        **ctx.accounts.escrow_info.to_account_info().try_borrow_mut_lamports()? = 0;
        **ctx.accounts.seller.try_borrow_mut_lamports()? += escrow_balance;

        ctx.accounts.escrow_info.state = 2; // Closed

        Ok(())
    }

    pub fn refund(ctx: Context<RefundCtx>, escrow_name: String) -> Result<()> {
        require!(
            ctx.accounts.escrow_info.state == 1, // WaitRecipient
            EscrowError::InvalidState
        );

        let escrow_balance = ctx.accounts.escrow_info.to_account_info().lamports();
        
        // Transfer entire balance to buyer (including rent)
        **ctx.accounts.escrow_info.to_account_info().try_borrow_mut_lamports()? = 0;
        **ctx.accounts.buyer.try_borrow_mut_lamports()? += escrow_balance;

        ctx.accounts.escrow_info.state = 2; // Closed

        Ok(())
    }
}

#[derive(Accounts)]
#[instruction(amount_in_lamports: u64, escrow_name: String)]
pub struct InitializeCtx<'info> {
    #[account(mut)]
    pub seller: Signer<'info>,
    /// CHECK: Used for PDA derivation
    pub buyer: AccountInfo<'info>,
    #[account(
        init,
        payer = seller,
        space = 8 + std::mem::size_of::<EscrowInfo>(),
        seeds = [
            escrow_name.as_bytes(),
            seller.key().as_ref(),
            buyer.key().as_ref()
        ],
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
    /// CHECK: Used for validation
    pub seller: AccountInfo<'info>,
    #[account(
        mut,
        seeds = [
            escrow_name.as_bytes(),
            seller.key().as_ref(),
            buyer.key().as_ref()
        ],
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
    /// CHECK: Recipient of funds
    pub seller: AccountInfo<'info>,
    #[account(
        mut,
        seeds = [
            escrow_name.as_bytes(),
            seller.key().as_ref(),
            buyer.key().as_ref()
        ],
        bump
    )]
    pub escrow_info: Account<'info, EscrowInfo>,
}

#[derive(Accounts)]
#[instruction(escrow_name: String)]
pub struct RefundCtx<'info> {
    pub seller: Signer<'info>,
    #[account(mut)]
    /// CHECK: Recipient of refund
    pub buyer: AccountInfo<'info>,
    #[account(
        mut,
        seeds = [
            escrow_name.as_bytes(),
            seller.key().as_ref(),
            buyer.key().as_ref()
        ],
        bump
    )]
    pub escrow_info: Account<'info, EscrowInfo>,
}

#[account]
#[derive(Default)]
pub struct EscrowInfo {
    pub seller: Pubkey,
    pub buyer: Pubkey,
    pub amount_in_lamports: u64,
    pub state: u8, // 0 = WaitDeposit, 1 = WaitRecipient, 2 = Closed
}

#[error_code]
pub enum EscrowError {
    #[msg("Invalid amount must be greater than zero")]
    InvalidAmount,
    #[msg("Invalid state for operation")]
    InvalidState,
    #[msg("Invalid buyer account")]
    InvalidBuyer,
}