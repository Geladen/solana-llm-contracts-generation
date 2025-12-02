use anchor_lang::prelude::*;
use anchor_spl::{
    associated_token::AssociatedToken,
    token::{self, Mint, Token, TokenAccount, Transfer, SetAuthority, CloseAccount},
};

declare_id!("7Wv4MSBAA7GphUz2xcNJ7Q6uXCw9FVLC3BKQKwb5Eugp");

#[program]
pub mod token_transfer {
    use super::*;

    pub fn deposit(ctx: Context<Deposit>) -> Result<()> {
        // Validate that temp_ata has positive balance
        require!(
            ctx.accounts.temp_ata.amount > 0,
            ErrorCode::InsufficientBalance
        );

        // Store deposit info
        let deposit_info = &mut ctx.accounts.deposit_info;
        deposit_info.temp_ata = ctx.accounts.temp_ata.key();
        deposit_info.recipient = ctx.accounts.recipient.key();
        deposit_info.sender = ctx.accounts.sender.key();
        deposit_info.mint = ctx.accounts.mint.key();
        deposit_info.initial_amount = ctx.accounts.temp_ata.amount;

        // Transfer ownership of temp_ata to ATAs Holder PDA
        let cpi_accounts = SetAuthority {
            current_authority: ctx.accounts.sender.to_account_info(),
            account_or_mint: ctx.accounts.temp_ata.to_account_info(),
        };
        let cpi_program = ctx.accounts.token_program.to_account_info();
        let cpi_ctx = CpiContext::new(cpi_program, cpi_accounts);
        
        token::set_authority(
            cpi_ctx,
            anchor_spl::token::spl_token::instruction::AuthorityType::AccountOwner,
            Some(ctx.accounts.atas_holder_pda.key()),
        )?;

        msg!("Deposit successful - Temp ATA owner: {:?}", ctx.accounts.temp_ata.owner);
        
        Ok(())
    }

    pub fn withdraw(ctx: Context<Withdraw>, amount_to_withdraw: u64) -> Result<()> {
        // Validate withdrawal amount
        require!(
            amount_to_withdraw > 0,
            ErrorCode::InvalidWithdrawalAmount
        );

        // Validate all accounts match deposit info
        require!(
            ctx.accounts.recipient.key() == ctx.accounts.deposit_info.recipient,
            ErrorCode::InvalidRecipient
        );

        require!(
            ctx.accounts.sender.key() == ctx.accounts.deposit_info.sender,
            ErrorCode::InvalidSender
        );

        require!(
            ctx.accounts.mint.key() == ctx.accounts.deposit_info.mint,
            ErrorCode::InvalidMint
        );

        require!(
            ctx.accounts.temp_ata.key() == ctx.accounts.deposit_info.temp_ata,
            ErrorCode::InvalidTempAta
        );

        // Check sufficient balance
        require!(
            ctx.accounts.temp_ata.amount >= amount_to_withdraw,
            ErrorCode::InsufficientBalance
        );

        // Create signer seeds for PDA - FIXED: Use the bump from context
        let signer_seeds: &[&[&[u8]]] = &[&[b"atas_holder", &[ctx.bumps.atas_holder_pda]]];

        msg!("Withdrawing {} tokens", amount_to_withdraw);
        msg!("Temp ATA balance before: {}", ctx.accounts.temp_ata.amount);
        msg!("Temp ATA owner: {:?}", ctx.accounts.temp_ata.owner);

        // Transfer tokens from temp_ata (owned by PDA) to recipient_ata
        let cpi_accounts = Transfer {
            from: ctx.accounts.temp_ata.to_account_info(),
            to: ctx.accounts.recipient_ata.to_account_info(),
            authority: ctx.accounts.atas_holder_pda.to_account_info(),
        };
        
        token::transfer(
            CpiContext::new_with_signer(
                ctx.accounts.token_program.to_account_info(),
                cpi_accounts,
                signer_seeds,
            ),
            amount_to_withdraw,
        )?;

        msg!("Transfer completed - processing account closure");

        // Calculate remaining balance
        let remaining_balance = ctx.accounts.temp_ata.amount.checked_sub(amount_to_withdraw)
            .ok_or(ErrorCode::CalculationError)?;

        // If full withdrawal, close the temp_ata account
        if remaining_balance == 0 {
            msg!("Full withdrawal - closing temp ATA account");
            
            let close_accounts = CloseAccount {
                account: ctx.accounts.temp_ata.to_account_info(),
                destination: ctx.accounts.sender.to_account_info(),
                authority: ctx.accounts.atas_holder_pda.to_account_info(),
            };
            
            token::close_account(
                CpiContext::new_with_signer(
                    ctx.accounts.token_program.to_account_info(),
                    close_accounts,
                    signer_seeds,
                ),
            )?;

            msg!("Temp ATA closed");
        } else {
            msg!("Partial withdrawal - {} tokens remaining", remaining_balance);
        }

        Ok(())
    }
}

#[derive(Accounts)]
pub struct Deposit<'info> {
    #[account(mut)]
    pub sender: Signer<'info>,
    
    /// CHECK: Recipient public key (not a signer in deposit)
    pub recipient: AccountInfo<'info>,
    
    pub mint: Account<'info, Mint>,
    
    #[account(
        mut,
        associated_token::mint = mint,
        associated_token::authority = sender
    )]
    pub temp_ata: Account<'info, TokenAccount>,
    
    #[account(
        init,
        payer = sender,
        space = DepositInfo::LEN,
        seeds = [temp_ata.key().as_ref()],
        bump
    )]
    pub deposit_info: Account<'info, DepositInfo>,
    
    /// CHECK: PDA that will hold the temporary token account
    #[account(
        seeds = [b"atas_holder"],
        bump
    )]
    pub atas_holder_pda: AccountInfo<'info>,
    
    pub token_program: Program<'info, Token>,
    pub system_program: Program<'info, System>,
    pub associated_token_program: Program<'info, AssociatedToken>,
}

#[derive(Accounts)]
pub struct Withdraw<'info> {
    #[account(mut)]
    pub recipient: Signer<'info>,
    
    /// CHECK: Original sender for rent return
    #[account(mut)]
    pub sender: AccountInfo<'info>,
    
    pub mint: Account<'info, Mint>,
    
    #[account(
        mut,
        associated_token::mint = mint,
        associated_token::authority = recipient
    )]
    pub recipient_ata: Account<'info, TokenAccount>,
    
    #[account(mut)]
    pub temp_ata: Account<'info, TokenAccount>,
    
    #[account(
        mut,
        seeds = [temp_ata.key().as_ref()],
        bump,
        close = sender // Close deposit_info and send rent to sender
    )]
    pub deposit_info: Account<'info, DepositInfo>,
    
    /// CHECK: PDA that holds the temporary token account
    #[account(
        seeds = [b"atas_holder"],
        bump
    )]
    pub atas_holder_pda: AccountInfo<'info>,
    
    pub token_program: Program<'info, Token>,
    pub system_program: Program<'info, System>,
    pub associated_token_program: Program<'info, AssociatedToken>,
    pub rent: Sysvar<'info, Rent>,
}

#[account]
pub struct DepositInfo {
    pub temp_ata: Pubkey,      // The temporary token account in escrow
    pub recipient: Pubkey,     // Who can withdraw the tokens
    pub sender: Pubkey,        // Who deposited the tokens
    pub mint: Pubkey,          // The token mint
    pub initial_amount: u64,   // Initial amount deposited
}

impl DepositInfo {
    pub const LEN: usize = 8 + // discriminator
        32 + // temp_ata
        32 + // recipient  
        32 + // sender
        32 + // mint
        8;   // initial_amount
}

#[error_code]
pub enum ErrorCode {
    #[msg("Invalid recipient for this escrow")]
    InvalidRecipient,
    #[msg("Invalid sender for this escrow")]
    InvalidSender,
    #[msg("Invalid mint for this escrow")]
    InvalidMint,
    #[msg("Invalid temporary token account")]
    InvalidTempAta,
    #[msg("Insufficient balance in escrow")]
    InsufficientBalance,
    #[msg("Invalid withdrawal amount")]
    InvalidWithdrawalAmount,
    #[msg("Calculation error")]
    CalculationError,
}