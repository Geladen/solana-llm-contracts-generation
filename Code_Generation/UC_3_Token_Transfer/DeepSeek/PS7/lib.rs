use anchor_lang::prelude::*;
use anchor_spl::associated_token::AssociatedToken;
use anchor_spl::token::{self, Mint, Token, TokenAccount, Transfer};

declare_id!("J13NAiGcKc9jwfHDxk6D3ZXxHbNDB7Ned7n3qTa6MYVS");

#[program]
pub mod token_transfer {
    use super::*;

    pub fn deposit(ctx: Context<Deposit>) -> Result<()> {
        // Transfer tokens from sender's ATA to temporary ATA
        let transfer_amount = ctx.accounts.sender_ata.amount;
        
        let cpi_accounts = Transfer {
            from: ctx.accounts.sender_ata.to_account_info(),
            to: ctx.accounts.temp_ata.to_account_info(),
            authority: ctx.accounts.sender.to_account_info(),
        };
        let cpi_ctx = CpiContext::new(ctx.accounts.token_program.to_account_info(), cpi_accounts);
        token::transfer(cpi_ctx, transfer_amount)?;

        // Create a PDA-owned token account instead of transferring ownership
        // This is the proper way to have PDA-controlled tokens
        let seeds = &[b"atas_holder".as_ref(), &[ctx.bumps.atas_holder_pda]];
        let signer_seeds = &[&seeds[..]];

        // Close the original temp_ata and create a new PDA-owned one
        let close_ctx = CpiContext::new(
            ctx.accounts.token_program.to_account_info(),
            token::CloseAccount {
                account: ctx.accounts.temp_ata.to_account_info(),
                destination: ctx.accounts.sender.to_account_info(),
                authority: ctx.accounts.sender.to_account_info(),
            },
        );
        token::close_account(close_ctx)?;

        // Create new PDA-owned token account
        let create_ctx = CpiContext::new_with_signer(
            ctx.accounts.associated_token_program.to_account_info(),
            anchor_spl::associated_token::Create {
                payer: ctx.accounts.sender.to_account_info(),
                associated_token: ctx.accounts.pda_temp_ata.to_account_info(),
                authority: ctx.accounts.atas_holder_pda.to_account_info(),
                mint: ctx.accounts.mint.to_account_info(),
                system_program: ctx.accounts.system_program.to_account_info(),
                token_program: ctx.accounts.token_program.to_account_info(),
            },
            signer_seeds,
        );
        anchor_spl::associated_token::create(create_ctx)?;

        // Transfer tokens to PDA-owned account
        let cpi_accounts = Transfer {
            from: ctx.accounts.sender_ata.to_account_info(),
            to: ctx.accounts.pda_temp_ata.to_account_info(),
            authority: ctx.accounts.sender.to_account_info(),
        };
        let cpi_ctx = CpiContext::new(ctx.accounts.token_program.to_account_info(), cpi_accounts);
        token::transfer(cpi_ctx, transfer_amount)?;

        // Initialize deposit info
        let deposit_info = &mut ctx.accounts.deposit_info;
        deposit_info.temp_ata = ctx.accounts.pda_temp_ata.key();
        deposit_info.recipient = ctx.accounts.recipient.key();
        deposit_info.sender = ctx.accounts.sender.key();
        deposit_info.mint = ctx.accounts.mint.key();
        deposit_info.amount = transfer_amount;

        msg!("Deposit completed: {} tokens escrowed", transfer_amount);
        Ok(())
    }

    pub fn withdraw(ctx: Context<Withdraw>, amount_to_withdraw: u64) -> Result<()> {
        // Verify deposit info matches
        require_keys_eq!(
            ctx.accounts.deposit_info.temp_ata,
            ctx.accounts.temp_ata.key(),
            ErrorCode::InvalidDepositInfo
        );
        require_keys_eq!(
            ctx.accounts.deposit_info.recipient,
            ctx.accounts.recipient.key(),
            ErrorCode::InvalidRecipient
        );

        // Calculate adjusted amount considering token decimals
        let decimals = ctx.accounts.mint.decimals;
        let adjusted_amount = amount_to_withdraw
            .checked_mul(10u64.pow(decimals as u32))
            .ok_or(ErrorCode::AmountOverflow)?;

        // Verify the temp_ata has sufficient balance
        require!(
            ctx.accounts.temp_ata.amount >= adjusted_amount,
            ErrorCode::InsufficientBalance
        );

        // Transfer tokens using PDA as signer
        let seeds = &[b"atas_holder".as_ref(), &[ctx.bumps.atas_holder_pda]];
        let signer_seeds = &[&seeds[..]];
        
        let cpi_accounts = Transfer {
            from: ctx.accounts.temp_ata.to_account_info(),
            to: ctx.accounts.recipient_ata.to_account_info(),
            authority: ctx.accounts.atas_holder_pda.to_account_info(),
        };
        
        let cpi_ctx = CpiContext::new_with_signer(
            ctx.accounts.token_program.to_account_info(),
            cpi_accounts,
            signer_seeds,
        );
        
        token::transfer(cpi_ctx, adjusted_amount)?;

        // Update deposit info with remaining amount
        let deposit_info = &mut ctx.accounts.deposit_info;
        deposit_info.amount = deposit_info.amount.checked_sub(adjusted_amount)
            .ok_or(ErrorCode::AmountOverflow)?;

        // Close temp_ata if fully withdrawn
        if deposit_info.amount == 0 {
            // Close PDA-owned temp_ata account
            let seeds = &[b"atas_holder".as_ref(), &[ctx.bumps.atas_holder_pda]];
            let signer_seeds = &[&seeds[..]];
            
            let close_ctx = CpiContext::new_with_signer(
                ctx.accounts.token_program.to_account_info(),
                token::CloseAccount {
                    account: ctx.accounts.temp_ata.to_account_info(),
                    destination: ctx.accounts.sender.to_account_info(),
                    authority: ctx.accounts.atas_holder_pda.to_account_info(),
                },
                signer_seeds,
            );
            
            token::close_account(close_ctx)?;
            
            // Close deposit_info account and return rent to sender
            let deposit_info_account = &mut ctx.accounts.deposit_info;
            let deposit_info_lamports = deposit_info_account.to_account_info().lamports();
            **deposit_info_account.to_account_info().lamports.borrow_mut() = 0;
            **ctx.accounts.sender.lamports.borrow_mut() = ctx.accounts.sender.lamports()
                .checked_add(deposit_info_lamports)
                .ok_or(ErrorCode::AmountOverflow)?;
        }

        msg!("Withdrawal completed: {} tokens transferred", adjusted_amount);
        Ok(())
    }
}

#[derive(Accounts)]
pub struct Deposit<'info> {
    #[account(mut)]
    pub sender: Signer<'info>,
    
    /// CHECK: This is the recipient's public key, not an account
    pub recipient: UncheckedAccount<'info>,
    
    pub mint: Account<'info, Mint>,
    
    #[account(
        mut,
        associated_token::mint = mint,
        associated_token::authority = sender
    )]
    pub sender_ata: Account<'info, TokenAccount>,
    
    #[account(
        init_if_needed,
        payer = sender,
        associated_token::mint = mint,
        associated_token::authority = sender
    )]
    pub temp_ata: Account<'info, TokenAccount>,
    
    /// CHECK: This will be initialized as PDA-owned
    #[account(
        mut,
        associated_token::mint = mint,
        associated_token::authority = atas_holder_pda
    )]
    pub pda_temp_ata: Account<'info, TokenAccount>,
    
    #[account(
        init,
        payer = sender,
        space = DepositInfo::LEN,
        seeds = [b"deposit", temp_ata.key().as_ref()],
        bump
    )]
    pub deposit_info: Account<'info, DepositInfo>,
    
    #[account(
        seeds = [b"atas_holder"],
        bump
    )]
    /// CHECK: PDA that owns the temp ATA
    pub atas_holder_pda: UncheckedAccount<'info>,
    
    pub token_program: Program<'info, Token>,
    pub system_program: Program<'info, System>,
    pub associated_token_program: Program<'info, AssociatedToken>,
}

#[derive(Accounts)]
pub struct Withdraw<'info> {
    #[account(mut)]
    pub recipient: Signer<'info>,
    
    /// CHECK: Sender account for rent return
    #[account(mut)]
    pub sender: UncheckedAccount<'info>,
    
    pub mint: Account<'info, Mint>,
    
    #[account(
        init_if_needed,
        payer = recipient,
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
        seeds = [b"deposit", temp_ata.key().as_ref()],
        bump,
        constraint = deposit_info.recipient == recipient.key() @ ErrorCode::InvalidRecipient,
        constraint = deposit_info.temp_ata == temp_ata.key() @ ErrorCode::InvalidDepositInfo
    )]
    pub deposit_info: Account<'info, DepositInfo>,
    
    #[account(
        seeds = [b"atas_holder"],
        bump
    )]
    /// CHECK: PDA that owns the temp ATA
    pub atas_holder_pda: UncheckedAccount<'info>,
    
    pub token_program: Program<'info, Token>,
    pub system_program: Program<'info, System>,
    pub associated_token_program: Program<'info, AssociatedToken>,
    pub rent: Sysvar<'info, Rent>,
}

#[account]
pub struct DepositInfo {
    pub temp_ata: Pubkey,
    pub recipient: Pubkey,
    pub sender: Pubkey,
    pub mint: Pubkey,
    pub amount: u64,
}

impl DepositInfo {
    pub const LEN: usize = 8 + // discriminator
        32 + // temp_ata
        32 + // recipient
        32 + // sender
        32 + // mint
        8;   // amount
}

#[error_code]
pub enum ErrorCode {
    #[msg("Invalid deposit info")]
    InvalidDepositInfo,
    #[msg("Invalid recipient")]
    InvalidRecipient,
    #[msg("Amount overflow")]
    AmountOverflow,
    #[msg("Insufficient balance")]
    InsufficientBalance,
}