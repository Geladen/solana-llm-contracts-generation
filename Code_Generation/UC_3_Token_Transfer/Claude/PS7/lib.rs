use anchor_lang::prelude::*;
use anchor_spl::associated_token::AssociatedToken;
use anchor_spl::token::{self, spl_token::instruction::AuthorityType, Mint, SetAuthority, Token, TokenAccount};

declare_id!("9Bgvyps4SKpKkUy9AU313xZbRs8uVEYiBa4m7zEpMSdW");

#[program]
pub mod token_escrow {
    use super::*;

    /// Initiates token escrow by transferring temp_ata ownership to PDA
    /// and storing escrow information
    pub fn deposit(ctx: Context<DepositCtx>) -> Result<()> {
        let deposit_info = &mut ctx.accounts.deposit_info;
        
        // Store escrow information
        deposit_info.temp_ata = ctx.accounts.temp_ata.key();
        deposit_info.recipient = ctx.accounts.recipient.key();
        
        // Transfer ownership of temp_ata to ATAs Holder PDA
        let cpi_accounts = SetAuthority {
            current_authority: ctx.accounts.sender.to_account_info(),
            account_or_mint: ctx.accounts.temp_ata.to_account_info(),
        };
        
        let cpi_program = ctx.accounts.token_program.to_account_info();
        let cpi_ctx = CpiContext::new(cpi_program, cpi_accounts);
        
        // Get ATAs Holder PDA key
        let (atas_holder_pda, _bump) = Pubkey::find_program_address(
            &[b"atas_holder"],
            ctx.program_id
        );
        
        token::set_authority(
            cpi_ctx,
            AuthorityType::AccountOwner,
            Some(atas_holder_pda),
        )?;
        
        msg!("Deposit successful. Tokens escrowed for recipient: {}", deposit_info.recipient);
        
        Ok(())
    }

    /// Allows recipient to withdraw tokens from escrow
    /// Closes accounts when fully withdrawn
    pub fn withdraw(ctx: Context<WithdrawCtx>, amount_to_withdraw: u64) -> Result<()> {
        let deposit_info = &ctx.accounts.deposit_info;
        
        // Validate that the recipient matches
        require!(
            deposit_info.recipient == ctx.accounts.recipient.key(),
            ErrorCode::UnauthorizedRecipient
        );
        
        // Validate that temp_ata matches
        require!(
            deposit_info.temp_ata == ctx.accounts.temp_ata.key(),
            ErrorCode::InvalidTempAta
        );
        
        // Get mint decimals and calculate actual amount with decimals
        let decimals = ctx.accounts.mint.decimals;
        let decimal_multiplier = 10u64.pow(decimals as u32);
        let actual_amount = amount_to_withdraw
            .checked_mul(decimal_multiplier)
            .ok_or(ErrorCode::MathOverflow)?;
        
        // Get current balance of temp_ata
        let temp_ata_balance = ctx.accounts.temp_ata.amount;
        
        msg!("Temp ATA balance: {}", temp_ata_balance);
        msg!("Amount to withdraw (with decimals): {}", actual_amount);
        
        // Validate withdrawal amount
        require!(
            actual_amount <= temp_ata_balance,
            ErrorCode::InsufficientBalance
        );
        
        // Prepare PDA signer seeds
        let seeds = &[b"atas_holder".as_ref(), &[ctx.bumps.atas_holder_pda]];
        let signer = &[&seeds[..]];
        
        // Transfer tokens from temp_ata to recipient_ata
        let cpi_accounts = token::Transfer {
            from: ctx.accounts.temp_ata.to_account_info(),
            to: ctx.accounts.recipient_ata.to_account_info(),
            authority: ctx.accounts.atas_holder_pda.to_account_info(),
        };
        
        let cpi_program = ctx.accounts.token_program.to_account_info();
        let cpi_ctx = CpiContext::new_with_signer(cpi_program, cpi_accounts, signer);
        
        token::transfer(cpi_ctx, actual_amount)?;
        
        msg!("Transferred {} tokens to recipient", actual_amount);
        
        // Check if all tokens have been withdrawn
        let remaining_balance = temp_ata_balance.checked_sub(actual_amount).unwrap();
        
        msg!("Remaining balance: {}", remaining_balance);
        
        if remaining_balance == 0 {
            msg!("Closing temp_ata account...");
            
            // Close temp_ata account
            let close_accounts = token::CloseAccount {
                account: ctx.accounts.temp_ata.to_account_info(),
                destination: ctx.accounts.sender.to_account_info(),
                authority: ctx.accounts.atas_holder_pda.to_account_info(),
            };
            
            let close_program = ctx.accounts.token_program.to_account_info();
            let close_ctx = CpiContext::new_with_signer(close_program, close_accounts, signer);
            
            token::close_account(close_ctx)?;
            
            msg!("Temp ATA closed successfully.");
        } else {
            msg!("Partial withdrawal. Remaining balance: {}", remaining_balance);
        }
        
        Ok(())
    }
}

#[derive(Accounts)]
pub struct DepositCtx<'info> {
    #[account(mut)]
    pub sender: Signer<'info>,
    
    /// CHECK: Recipient address validated in business logic
    pub recipient: UncheckedAccount<'info>,
    
    pub mint: Account<'info, Mint>,
    
    #[account(
        mut,
        associated_token::mint = mint,
        associated_token::authority = sender,
    )]
    pub temp_ata: Account<'info, TokenAccount>,
    
    #[account(
        init,
        payer = sender,
        space = 8 + DepositInfo::INIT_SPACE,
        seeds = [temp_ata.key().as_ref()],
        bump
    )]
    pub deposit_info: Account<'info, DepositInfo>,
    
    pub token_program: Program<'info, Token>,
    pub system_program: Program<'info, System>,
    pub associated_token_program: Program<'info, AssociatedToken>,
}

#[derive(Accounts)]
pub struct WithdrawCtx<'info> {
    #[account(mut)]
    pub recipient: Signer<'info>,
    
    /// CHECK: Sender address used for returning rent
    #[account(mut)]
    pub sender: UncheckedAccount<'info>,
    
    pub mint: Account<'info, Mint>,
    
    #[account(
        init_if_needed,
        payer = recipient,
        associated_token::mint = mint,
        associated_token::authority = recipient,
    )]
    pub recipient_ata: Account<'info, TokenAccount>,
    
    #[account(
        mut,
        constraint = temp_ata.mint == mint.key() @ ErrorCode::InvalidMint,
        constraint = temp_ata.owner == atas_holder_pda.key() @ ErrorCode::InvalidAuthority,
    )]
    pub temp_ata: Account<'info, TokenAccount>,
    
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
    /// CHECK: PDA used as authority for token operations
    pub atas_holder_pda: UncheckedAccount<'info>,
    
    pub token_program: Program<'info, Token>,
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
pub enum ErrorCode {
    #[msg("Unauthorized recipient attempting to withdraw")]
    UnauthorizedRecipient,
    #[msg("Invalid temporary token account")]
    InvalidTempAta,
    #[msg("Insufficient balance in escrow")]
    InsufficientBalance,
    #[msg("Invalid mint for token account")]
    InvalidMint,
    #[msg("Invalid authority for token account")]
    InvalidAuthority,
    #[msg("Math operation overflow")]
    MathOverflow,
}