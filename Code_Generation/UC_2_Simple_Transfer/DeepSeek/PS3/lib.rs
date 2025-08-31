#![allow(unexpected_cfgs)]
use anchor_lang::prelude::*;
use anchor_lang::system_program;

declare_id!("H8xcZLcJbeVQWEXuGFYAGT1rxaDMMuLrsnmZMTcJ1SZ8");

#[program]
pub mod transfer_contract {
    use super::*;

    pub fn deposit(ctx: Context<DepositCtx>, amount_to_deposit: u64) -> Result<()> {
        // Validate amount
        require!(amount_to_deposit > 0, TransferError::InvalidAmount);

        // Check if PDA needs initialization by checking if it has data
        if ctx.accounts.balance_holder_pda.data_is_empty() {
            // This is a new PDA, we need to initialize it
            // Calculate required rent
            let rent = Rent::get()?;
            let space = 8 + 32 + 32 + 8; // Discriminator + Pubkey + Pubkey + u64
            let lamports_required = rent.minimum_balance(space);
            
            // Transfer rent to PDA
            let cpi_context = CpiContext::new(
                ctx.accounts.system_program.to_account_info(),
                system_program::Transfer {
                    from: ctx.accounts.sender.to_account_info(),
                    to: ctx.accounts.balance_holder_pda.to_account_info(),
                },
            );
            system_program::transfer(cpi_context, lamports_required)?;
            
            // Initialize the PDA data
            let mut data = ctx.accounts.balance_holder_pda.try_borrow_mut_data()?;
            
            // Write account discriminator (first 8 bytes)
            // We'll use a simple discriminator for this example
            let disc = [1, 2, 3, 4, 5, 6, 7, 8]; // Simple discriminator
            data[0..8].copy_from_slice(&disc);
            
            // Write sender public key
            data[8..40].copy_from_slice(ctx.accounts.sender.key().as_ref());
            
            // Write recipient public key
            data[40..72].copy_from_slice(ctx.accounts.recipient.key().as_ref());
            
            // Write amount
            data[72..80].copy_from_slice(&amount_to_deposit.to_le_bytes());
        } else {
            // PDA already exists, update the amount
            let mut data = ctx.accounts.balance_holder_pda.try_borrow_mut_data()?;
            
            // Read current amount
            let mut amount_bytes = [0u8; 8];
            amount_bytes.copy_from_slice(&data[72..80]);
            let current_amount = u64::from_le_bytes(amount_bytes);
            
            // Update amount
            let new_amount = current_amount + amount_to_deposit;
            data[72..80].copy_from_slice(&new_amount.to_le_bytes());
        }

        // Transfer deposit amount to PDA
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
        require!(amount_to_withdraw > 0, TransferError::InvalidAmount);
        
        // Read current amount from PDA
        let data = ctx.accounts.balance_holder_pda.try_borrow_data()?;
        
        // Verify sender and recipient
        let sender_in_pda = Pubkey::new_from_array(data[8..40].try_into().unwrap());
        let recipient_in_pda = Pubkey::new_from_array(data[40..72].try_into().unwrap());
        
        require!(
            sender_in_pda == *ctx.accounts.sender.key,
            TransferError::InvalidSender
        );
        require!(
            recipient_in_pda == *ctx.accounts.recipient.key,
            TransferError::InvalidRecipient
        );
        
        // Read current amount
        let mut amount_bytes = [0u8; 8];
        amount_bytes.copy_from_slice(&data[72..80]);
        let current_amount = u64::from_le_bytes(amount_bytes);
        
        require!(
            amount_to_withdraw <= current_amount,
            TransferError::InsufficientFunds
        );

        // Update PDA state
        drop(data); // Release borrow before borrowing mutably
        let mut data = ctx.accounts.balance_holder_pda.try_borrow_mut_data()?;
        
        let new_amount = current_amount - amount_to_withdraw;
        data[72..80].copy_from_slice(&new_amount.to_le_bytes());

        // Transfer lamports to recipient
        **ctx.accounts.balance_holder_pda.try_borrow_mut_lamports()? = ctx
            .accounts
            .balance_holder_pda
            .lamports()
            .checked_sub(amount_to_withdraw)
            .unwrap();
        **ctx
            .accounts
            .recipient
            .to_account_info()
            .try_borrow_mut_lamports()? = ctx
            .accounts
            .recipient
            .to_account_info()
            .lamports()
            .checked_add(amount_to_withdraw)
            .unwrap();

        // Close PDA if balance is zero
        if new_amount == 0 {
            let rent = Rent::get()?;
            let rent_lamports = rent.minimum_balance(ctx.accounts.balance_holder_pda.data_len());
            let remaining_lamports = ctx.accounts.balance_holder_pda.lamports() - rent_lamports;

            **ctx.accounts.balance_holder_pda.try_borrow_mut_lamports()? = rent_lamports;
            **ctx
                .accounts
                .sender
                .to_account_info()
                .try_borrow_mut_lamports()? = ctx
                .accounts
                .sender
                .to_account_info()
                .lamports()
                .checked_add(remaining_lamports)
                .unwrap();
        }

        Ok(())
    }
}

#[derive(Accounts)]
pub struct DepositCtx<'info> {
    #[account(mut)]
    sender: Signer<'info>,
    /// CHECK: Used for PDA derivation only, no data access
    recipient: AccountInfo<'info>,
    /// CHECK: PDA account that will be initialized if needed
    #[account(
        mut,
        seeds = [recipient.key().as_ref(), sender.key().as_ref()],
        bump,
    )]
    balance_holder_pda: AccountInfo<'info>,
    system_program: Program<'info, System>,
}

#[derive(Accounts)]
pub struct WithdrawCtx<'info> {
    #[account(mut)]
    recipient: Signer<'info>,
    /// CHECK: Used for PDA derivation and validation only, no data access
    sender: AccountInfo<'info>,
    /// CHECK: PDA account that stores the balance
    #[account(
        mut,
        seeds = [recipient.key().as_ref(), sender.key().as_ref()],
        bump,
    )]
    balance_holder_pda: AccountInfo<'info>,
    rent: Sysvar<'info, Rent>,
}

#[error_code]
pub enum TransferError {
    #[msg("Amount must be greater than zero")]
    InvalidAmount,
    #[msg("Insufficient funds for withdrawal")]
    InsufficientFunds,
    #[msg("Invalid sender")]
    InvalidSender,
    #[msg("Invalid recipient")]
    InvalidRecipient,
}