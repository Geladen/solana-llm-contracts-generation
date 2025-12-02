use anchor_lang::prelude::*;
use anchor_spl::associated_token::AssociatedToken;
use anchor_spl::token::{self, Mint, SetAuthority, Token, TokenAccount};
use anchor_spl::token::spl_token::instruction::AuthorityType;

declare_id!("2mh2FVWAjdbkb6VraZkx45PRd4BKPLzQJrkfPm5HGLWQ");

#[program]
pub mod token_transfer {
    use super::*;

    /// Initiates token escrow by transferring ownership of temp_ata to PDA
    pub fn deposit(ctx: Context<DepositCtx>) -> Result<()> {
        let temp_ata = &ctx.accounts.temp_ata;
        let deposit_info = &mut ctx.accounts.deposit_info;
        let atas_holder_pda_key = Pubkey::find_program_address(
            &[b"atas_holder"],
            ctx.program_id
        ).0;

        // Validate that temp_ata has tokens
        require!(temp_ata.amount > 0, ErrorCode::NoTokensToDeposit);

        // Validate that temp_ata is currently owned by sender
        require!(
            temp_ata.owner == ctx.accounts.sender.key(),
            ErrorCode::InvalidTokenAccountOwner
        );

        // Store escrow information
        deposit_info.temp_ata = temp_ata.key();
        deposit_info.recipient = ctx.accounts.recipient.key();

        // Transfer ownership of temp_ata to ATAs Holder PDA
        let cpi_accounts = SetAuthority {
            account_or_mint: ctx.accounts.temp_ata.to_account_info(),
            current_authority: ctx.accounts.sender.to_account_info(),
        };
        let cpi_program = ctx.accounts.token_program.to_account_info();
        let cpi_ctx = CpiContext::new(cpi_program, cpi_accounts);

        token::set_authority(
            cpi_ctx,
            AuthorityType::AccountOwner,
            Some(atas_holder_pda_key),
        )?;

        msg!("Deposit successful: {} tokens escrowed", temp_ata.amount);
        Ok(())
    }

    /// Allows recipient to withdraw tokens from escrow
    pub fn withdraw(ctx: Context<WithdrawCtx>, amount_to_withdraw: u64) -> Result<()> {
        let temp_ata = &ctx.accounts.temp_ata;
        let deposit_info = &ctx.accounts.deposit_info;
        let mint = &ctx.accounts.mint;

        // Validate withdrawal amount
        require!(amount_to_withdraw > 0, ErrorCode::InvalidWithdrawAmount);

        // Convert amount to raw token units (multiply by 10^decimals)
        let decimals = mint.decimals;
        let amount_with_decimals = amount_to_withdraw
            .checked_mul(10u64.pow(decimals as u32))
            .ok_or(ErrorCode::MathOverflow)?;

        require!(
            temp_ata.amount >= amount_with_decimals,
            ErrorCode::InsufficientBalance
        );

        // Validate that temp_ata matches deposit_info
        require!(
            temp_ata.key() == deposit_info.temp_ata,
            ErrorCode::InvalidTempAta
        );

        // Validate that recipient matches deposit_info
        require!(
            ctx.accounts.recipient.key() == deposit_info.recipient,
            ErrorCode::UnauthorizedRecipient
        );

        // Generate PDA signer seeds
        let seeds = &[b"atas_holder".as_ref(), &[ctx.bumps.atas_holder_pda]];
        let signer_seeds = &[&seeds[..]];

        // Transfer tokens from temp_ata to recipient_ata
        let cpi_accounts = token::Transfer {
            from: ctx.accounts.temp_ata.to_account_info(),
            to: ctx.accounts.recipient_ata.to_account_info(),
            authority: ctx.accounts.atas_holder_pda.to_account_info(),
        };
        let cpi_program = ctx.accounts.token_program.to_account_info();
        let cpi_ctx = CpiContext::new_with_signer(cpi_program, cpi_accounts, signer_seeds);

        token::transfer(cpi_ctx, amount_with_decimals)?;

        // Reload temp_ata to get updated balance
        ctx.accounts.temp_ata.reload()?;

        msg!("Withdrawal successful: {} tokens ({} raw units) transferred", amount_to_withdraw, amount_with_decimals);

        // If temp_ata is now empty, close both accounts
        if ctx.accounts.temp_ata.amount == 0 {
            // Close temp_ata account
            let cpi_accounts = token::CloseAccount {
                account: ctx.accounts.temp_ata.to_account_info(),
                destination: ctx.accounts.sender.to_account_info(),
                authority: ctx.accounts.atas_holder_pda.to_account_info(),
            };
            let cpi_program = ctx.accounts.token_program.to_account_info();
            let cpi_ctx = CpiContext::new_with_signer(cpi_program, cpi_accounts, signer_seeds);

            token::close_account(cpi_ctx)?;

            // Close deposit_info account and return rent to sender
            let deposit_info_account = ctx.accounts.deposit_info.to_account_info();
            let sender_account = ctx.accounts.sender.to_account_info();
            
            let dest_starting_lamports = sender_account.lamports();
            **sender_account.lamports.borrow_mut() = dest_starting_lamports
                .checked_add(deposit_info_account.lamports())
                .unwrap();
            **deposit_info_account.lamports.borrow_mut() = 0;

            msg!("Temp ATA and deposit info closed, all tokens withdrawn");
        }

        Ok(())
    }
}

#[derive(Accounts)]
pub struct DepositCtx<'info> {
    #[account(mut)]
    pub sender: Signer<'info>,

    /// CHECK: Recipient account, validated in business logic
    pub recipient: UncheckedAccount<'info>,

    pub mint: Account<'info, Mint>,

    #[account(
        mut,
        token::mint = mint,
        token::authority = sender,
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

    /// CHECK: Sender account to receive rent refund
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
        token::mint = mint,
        token::authority = atas_holder_pda,
    )]
    pub temp_ata: Account<'info, TokenAccount>,

    #[account(
        mut,
        seeds = [temp_ata.key().as_ref()],
        bump,
    )]
    pub deposit_info: Account<'info, DepositInfo>,

    /// CHECK: PDA that holds authority over temp_ata
    #[account(
        seeds = [b"atas_holder"],
        bump
    )]
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
    #[msg("No tokens to deposit")]
    NoTokensToDeposit,
    #[msg("Invalid token account owner")]
    InvalidTokenAccountOwner,
    #[msg("Invalid withdrawal amount")]
    InvalidWithdrawAmount,
    #[msg("Insufficient balance in temp_ata")]
    InsufficientBalance,
    #[msg("Invalid temp_ata account")]
    InvalidTempAta,
    #[msg("Unauthorized recipient")]
    UnauthorizedRecipient,
    #[msg("Math overflow during decimal conversion")]
    MathOverflow,
}