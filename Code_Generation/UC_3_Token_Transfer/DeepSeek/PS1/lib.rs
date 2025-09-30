
use anchor_lang::prelude::*;
use anchor_spl::{
    associated_token::AssociatedToken,
    token::{
        transfer, close_account, 
        Transfer, CloseAccount, 
        Token, TokenAccount, Mint
    },
};

declare_id!("FyPxqSXCggZdgi4DriTNmNVjAU5A8BYrwmZDQ1WAu3Xs");

#[program]
pub mod token_transfer {
    use super::*;

    pub fn deposit(ctx: Context<DepositCtx>, amount: u64) -> Result<()> {
        require!(amount > 0, ErrorCode::InvalidAmount);
        require!(
            ctx.accounts.sender_ata.amount >= amount,
            ErrorCode::InsufficientFunds
        );

        // Transfer tokens from sender's ATA to PDA-owned token account
        let cpi_accounts = Transfer {
            from: ctx.accounts.sender_ata.to_account_info(),
            to: ctx.accounts.temp_ata.to_account_info(),
            authority: ctx.accounts.sender.to_account_info(),
        };
        let cpi_program = ctx.accounts.token_program.to_account_info();
        let cpi_ctx = CpiContext::new(cpi_program, cpi_accounts);
        
        transfer(cpi_ctx, amount)?;

        // Store escrow information
        let deposit_info = &mut ctx.accounts.deposit_info;
        deposit_info.temp_ata = ctx.accounts.temp_ata.key();
        deposit_info.recipient = ctx.accounts.recipient.key();
        deposit_info.amount = amount;

        msg!(
            "Deposit created: {} tokens escrowed from {} to {}",
            amount,
            ctx.accounts.sender.key(),
            ctx.accounts.recipient.key()
        );

        Ok(())
    }

    pub fn withdraw(ctx: Context<WithdrawCtx>, amount_to_withdraw: u64) -> Result<()> {
        // Validate the deposit info matches
        require!(
            ctx.accounts.deposit_info.recipient == ctx.accounts.recipient.key(),
            ErrorCode::InvalidRecipient
        );
        require!(
            ctx.accounts.deposit_info.temp_ata == ctx.accounts.temp_ata.key(),
            ErrorCode::InvalidTempATA
        );

        // Calculate actual amount to withdraw (respecting token decimals)
        let decimals = ctx.accounts.mint.decimals;
        let adjusted_amount = amount_to_withdraw
            .checked_mul(10u64.pow(decimals as u32))
            .ok_or(ErrorCode::CalculationOverflow)?;

        require!(
            adjusted_amount <= ctx.accounts.temp_ata.amount,
            ErrorCode::InsufficientFunds
        );

        // Transfer tokens from temp_ata to recipient_ata using PDA as signer
        let bump = ctx.bumps.atas_holder_pda;
        let seeds: &[&[&[u8]]] = &[&[b"atas_holder", &[bump]]];

        let cpi_accounts = Transfer {
            from: ctx.accounts.temp_ata.to_account_info(),
            to: ctx.accounts.recipient_ata.to_account_info(),
            authority: ctx.accounts.atas_holder_pda.to_account_info(),
        };
        let cpi_program = ctx.accounts.token_program.to_account_info();
        let cpi_ctx = CpiContext::new_with_signer(cpi_program, cpi_accounts, seeds);
        
        transfer(cpi_ctx, adjusted_amount)?;

        msg!(
            "Withdrawn {} tokens to recipient {}",
            adjusted_amount,
            ctx.accounts.recipient.key()
        );

        // Close temp_ata account if fully withdrawn
        if ctx.accounts.temp_ata.amount == adjusted_amount {
            let close_seeds: &[&[&[u8]]] = &[&[b"atas_holder", &[bump]]];

            let close_accounts = CloseAccount {
                account: ctx.accounts.temp_ata.to_account_info(),
                destination: ctx.accounts.sender.to_account_info(),
                authority: ctx.accounts.atas_holder_pda.to_account_info(),
            };
            let cpi_program = ctx.accounts.token_program.to_account_info();
            let cpi_ctx = CpiContext::new_with_signer(cpi_program, close_accounts, close_seeds);
            
            close_account(cpi_ctx)?;

            msg!("Temporary token account closed");
        }

        Ok(())
    }
}

#[derive(Accounts)]
pub struct DepositCtx<'info> {
    #[account(mut)]
    pub sender: Signer<'info>,
    
    /// CHECK: This is the recipient who will be able to withdraw
    pub recipient: AccountInfo<'info>,
    
    pub mint: Account<'info, Mint>,
    
    #[account(
        mut,
        associated_token::mint = mint,
        associated_token::authority = sender
    )]
    pub sender_ata: Account<'info, TokenAccount>,
    
    #[account(
        init,
        payer = sender,
        token::mint = mint,
        token::authority = atas_holder_pda,
    )]
    pub temp_ata: Account<'info, TokenAccount>,
    
    #[account(
        init,
        payer = sender,
        space = DepositInfo::LEN,
        seeds = [b"deposit_info", temp_ata.key().as_ref()],
        bump
    )]
    pub deposit_info: Account<'info, DepositInfo>,
    
    #[account(
        seeds = [b"atas_holder"],
        bump
    )]
    /// CHECK: PDA that will own the temporary token account
    pub atas_holder_pda: AccountInfo<'info>,
    
    pub token_program: Program<'info, Token>,
    pub system_program: Program<'info, System>,
    pub associated_token_program: Program<'info, AssociatedToken>,
}

#[derive(Accounts)]
pub struct WithdrawCtx<'info> {
    #[account(mut)]
    pub recipient: Signer<'info>,
    
    /// CHECK: Original sender who deposited tokens - not required as signer
    pub sender: AccountInfo<'info>,
    
    pub mint: Account<'info, Mint>,
    
    #[account(
        mut,
        associated_token::mint = mint,
        associated_token::authority = recipient
    )]
    pub recipient_ata: Account<'info, TokenAccount>,
    
    #[account(
        mut,
        associated_token::mint = mint,
        associated_token::authority = atas_holder_pda
    )]
    pub temp_ata: Account<'info, TokenAccount>,
    
    #[account(
        mut,
        close = sender, // Send rent back to sender when closed
        seeds = [b"deposit_info", temp_ata.key().as_ref()],
        bump
    )]
    pub deposit_info: Account<'info, DepositInfo>,
    
    #[account(
        seeds = [b"atas_holder"],
        bump
    )]
    /// CHECK: PDA that owns the temporary token account
    pub atas_holder_pda: AccountInfo<'info>,
    
    pub token_program: Program<'info, Token>,
    pub system_program: Program<'info, System>,
    pub associated_token_program: Program<'info, AssociatedToken>,
    
    pub rent: Sysvar<'info, Rent>,
}

#[account]
pub struct DepositInfo {
    pub temp_ata: Pubkey,    // 32 bytes
    pub recipient: Pubkey,   // 32 bytes  
    pub amount: u64,         // 8 bytes
}

impl DepositInfo {
    pub const LEN: usize = 8 + // discriminator
        32 + // temp_ata
        32 + // recipient
        8;   // amount
}

#[error_code]
pub enum ErrorCode {
    #[msg("Unauthorized access")]
    Unauthorized,
    #[msg("Invalid recipient")]
    InvalidRecipient,
    #[msg("Invalid temporary token account")]
    InvalidTempATA,
    #[msg("Insufficient funds")]
    InsufficientFunds,
    #[msg("Invalid amount")]
    InvalidAmount,
    #[msg("Calculation overflow")]
    CalculationOverflow,
}