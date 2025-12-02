use anchor_lang::prelude::*;
use anchor_spl::token_interface::{self, TokenAccount, Mint, TokenInterface, TransferChecked, SetAuthority, CloseAccount};
use anchor_spl::associated_token::AssociatedToken;

declare_id!("EYPSnjhpTwJXT7zYFNBFUgusWgmmovFCdcaJfbmSBZYF");

#[program]
pub mod token_transfer {
    use super::*;

    /// Deposit tokens into escrow by transferring temp_ata ownership to PDA
    pub fn deposit(ctx: Context<DepositCtx>) -> Result<()> {
        let deposit_info = &mut ctx.accounts.deposit_info;
        
        // Store escrow information
        deposit_info.temp_ata = ctx.accounts.temp_ata.key();
        deposit_info.recipient = ctx.accounts.recipient.key();
        
        // Transfer ownership of temp_ata to ATAs Holder PDA
        let (atas_holder_pda, _bump) = Pubkey::find_program_address(
            &[b"atas_holder"],
            ctx.program_id
        );
        
        let cpi_accounts = SetAuthority {
            account_or_mint: ctx.accounts.temp_ata.to_account_info(),
            current_authority: ctx.accounts.sender.to_account_info(),
        };
        
        let cpi_program = ctx.accounts.token_program.to_account_info();
        let cpi_ctx = CpiContext::new(cpi_program, cpi_accounts);
        
        token_interface::set_authority(
            cpi_ctx,
            anchor_spl::token_interface::spl_token_2022::instruction::AuthorityType::AccountOwner,
            Some(atas_holder_pda),
        )?;
        
        msg!("Deposit successful. Temp ATA: {}, Recipient: {}", 
             ctx.accounts.temp_ata.key(), 
             ctx.accounts.recipient.key());
        
        Ok(())
    }

    /// Withdraw tokens from escrow to recipient
    pub fn withdraw(ctx: Context<WithdrawCtx>, amount_to_withdraw: u64) -> Result<()> {
        let temp_ata = &ctx.accounts.temp_ata;
        let deposit_info = &ctx.accounts.deposit_info;
        let mint = &ctx.accounts.mint;
        
        // Verify recipient matches stored info
        require!(
            deposit_info.recipient == ctx.accounts.recipient.key(),
            EscrowError::InvalidRecipient
        );
        
        // Verify temp_ata matches stored info
        require!(
            deposit_info.temp_ata == temp_ata.key(),
            EscrowError::InvalidTempAta
        );
        
        // Calculate actual amount with decimals
        let actual_amount = amount_to_withdraw
            .checked_mul(10u64.pow(mint.decimals as u32))
            .ok_or(EscrowError::MathOverflow)?;
        
        // Verify temp_ata has sufficient balance
        require!(
            temp_ata.amount >= actual_amount,
            EscrowError::InsufficientBalance
        );
        
        // Generate PDA signer seeds
        let seeds = &[
            b"atas_holder".as_ref(),
            &[ctx.bumps.atas_holder_pda],
        ];
        let signer_seeds = &[&seeds[..]];
        
        // Transfer tokens from temp_ata to recipient_ata using transfer_checked
        let transfer_accounts = TransferChecked {
            from: ctx.accounts.temp_ata.to_account_info(),
            to: ctx.accounts.recipient_ata.to_account_info(),
            authority: ctx.accounts.atas_holder_pda.to_account_info(),
            mint: ctx.accounts.mint.to_account_info(),
        };
        
        let transfer_ctx = CpiContext::new_with_signer(
            ctx.accounts.token_program.to_account_info(),
            transfer_accounts,
            signer_seeds,
        );
        
        token_interface::transfer_checked(transfer_ctx, actual_amount, mint.decimals)?;
        
        msg!("Transferred {} tokens ({} with decimals) to recipient", amount_to_withdraw, actual_amount);
        
        // Reload temp_ata to check remaining balance
        ctx.accounts.temp_ata.reload()?;
        
        // If temp_ata is fully withdrawn, close accounts
        if ctx.accounts.temp_ata.amount == 0 {
            // Close temp_ata and return rent to sender
            let close_accounts = CloseAccount {
                account: ctx.accounts.temp_ata.to_account_info(),
                destination: ctx.accounts.sender.to_account_info(),
                authority: ctx.accounts.atas_holder_pda.to_account_info(),
            };
            
            let close_ctx = CpiContext::new_with_signer(
                ctx.accounts.token_program.to_account_info(),
                close_accounts,
                signer_seeds,
            );
            
            token_interface::close_account(close_ctx)?;
            
            msg!("Temp ATA closed. Escrow fully withdrawn.");
        }
        
        Ok(())
    }
}

#[derive(Accounts)]
pub struct DepositCtx<'info> {
    #[account(mut)]
    pub sender: Signer<'info>,
    
    /// CHECK: Recipient address stored in deposit_info
    pub recipient: UncheckedAccount<'info>,
    
    #[account(
        mint::token_program = token_program
    )]
    pub mint: InterfaceAccount<'info, Mint>,
    
    #[account(
        mut,
        token::mint = mint,
        token::authority = sender,
        token::token_program = token_program
    )]
    pub temp_ata: InterfaceAccount<'info, TokenAccount>,
    
    #[account(
        init,
        payer = sender,
        space = 8 + DepositInfo::INIT_SPACE,
        seeds = [temp_ata.key().as_ref()],
        bump
    )]
    pub deposit_info: Account<'info, DepositInfo>,
    
    pub token_program: Interface<'info, TokenInterface>,
    pub system_program: Program<'info, System>,
    pub associated_token_program: Program<'info, AssociatedToken>,
}

#[derive(Accounts)]
pub struct WithdrawCtx<'info> {
    #[account(mut)]
    pub recipient: Signer<'info>,
    
    /// CHECK: Sender receives rent refund when accounts are closed
    #[account(mut)]
    pub sender: UncheckedAccount<'info>,
    
    #[account(
        mint::token_program = token_program
    )]
    pub mint: InterfaceAccount<'info, Mint>,
    
    #[account(
        init_if_needed,
        payer = recipient,
        associated_token::mint = mint,
        associated_token::authority = recipient,
        associated_token::token_program = token_program
    )]
    pub recipient_ata: InterfaceAccount<'info, TokenAccount>,
    
    #[account(
        mut,
        token::mint = mint,
        token::token_program = token_program
    )]
    pub temp_ata: InterfaceAccount<'info, TokenAccount>,
    
    #[account(
        mut,
        seeds = [temp_ata.key().as_ref()],
        bump,
        close = sender,
    )]
    pub deposit_info: Account<'info, DepositInfo>,
    
    #[account(
        seeds = [b"atas_holder"],
        bump
    )]
    /// CHECK: PDA that owns the temp_ata
    pub atas_holder_pda: UncheckedAccount<'info>,
    
    pub token_program: Interface<'info, TokenInterface>,
    pub system_program: Program<'info, System>,
    pub associated_token_program: Program<'info, AssociatedToken>,
    pub rent: Sysvar<'info, Rent>,
}

#[account]
#[derive(InitSpace)]
pub struct DepositInfo {
    pub temp_ata: Pubkey,
    pub recipient: Pubkey,
}

#[error_code]
pub enum EscrowError {
    #[msg("Invalid recipient")]
    InvalidRecipient,
    #[msg("Invalid temp ATA")]
    InvalidTempAta,
    #[msg("Insufficient balance in temp ATA")]
    InsufficientBalance,
    #[msg("Math overflow")]
    MathOverflow,
}