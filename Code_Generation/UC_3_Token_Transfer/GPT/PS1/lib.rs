use anchor_lang::prelude::*;
use anchor_spl::associated_token::AssociatedToken;
use anchor_spl::token::{self, CloseAccount, SetAuthority, Transfer, Token, TokenAccount};
use anchor_spl::token::spl_token::instruction::AuthorityType;

declare_id!("3vX3hXhmk1C2UEcUGapMCk28QzepkEEdbG43c7MkkX8U");

#[program]
pub mod token_transfer {
    use super::*;

    pub fn deposit(ctx: Context<DepositCtx>) -> Result<()> {
        let deposit_info = &mut ctx.accounts.deposit_info;

        // Ensure temp_ata is owned by sender
        require!(
            ctx.accounts.temp_ata.owner == ctx.accounts.sender.key(),
            EscrowError::InvalidTempAtaOwner
        );

        // Transfer ownership of temp_ata to PDA (atas_holder)
        let cpi_accounts = SetAuthority {
            account_or_mint: ctx.accounts.temp_ata.to_account_info(),
            current_authority: ctx.accounts.sender.to_account_info(),
        };
        let cpi_program = ctx.accounts.token_program.to_account_info();
        let cpi_ctx = CpiContext::new(cpi_program.clone(), cpi_accounts);

        token::set_authority(
            cpi_ctx,
            AuthorityType::AccountOwner,
            Some(ctx.accounts.atas_holder.key()),
        )?;

        // Initialize deposit_info
        deposit_info.temp_ata = ctx.accounts.temp_ata.key();
        deposit_info.recipient = ctx.accounts.recipient.key();

        Ok(())
    }

    pub fn withdraw(ctx: Context<WithdrawCtx>, amount_to_withdraw: u64) -> Result<()> {
        let deposit_info = &ctx.accounts.deposit_info;

        // Only recipient can withdraw
        require!(
            deposit_info.recipient == ctx.accounts.recipient.key(),
            EscrowError::UnauthorizedRecipient
        );

        // PDA signer seeds
        let (atas_holder_pda, bump) =
            Pubkey::find_program_address(&[b"atas_holder"], ctx.program_id);
        require!(
            atas_holder_pda == ctx.accounts.atas_holder_pda.key(),
            EscrowError::InvalidPDA
        );

        let bump_bytes = &[bump];
        let signer_seeds_single: &[&[u8]] = &[b"atas_holder", bump_bytes];
        let signer_seeds: &[&[&[u8]]] = &[signer_seeds_single];

        // Get temp_ata balance
        let temp_ata_balance = ctx.accounts.temp_ata.amount;
        require!(
            amount_to_withdraw <= temp_ata_balance,
            EscrowError::InsufficientFunds
        );

        // Transfer tokens to recipient
        let cpi_accounts = Transfer {
            from: ctx.accounts.temp_ata.to_account_info(),
            to: ctx.accounts.recipient_ata.to_account_info(),
            authority: ctx.accounts.atas_holder_pda.to_account_info(),
        };
        let cpi_program = ctx.accounts.token_program.to_account_info();
        let cpi_ctx = CpiContext::new_with_signer(cpi_program.clone(), cpi_accounts, signer_seeds);
        token::transfer(cpi_ctx, amount_to_withdraw)?;

        // Close temp_ata if full balance withdrawn
        if amount_to_withdraw == temp_ata_balance {
            let cpi_accounts = CloseAccount {
                account: ctx.accounts.temp_ata.to_account_info(),
                destination: ctx.accounts.sender.to_account_info(),
                authority: ctx.accounts.atas_holder_pda.to_account_info(),
            };
            let cpi_ctx = CpiContext::new_with_signer(cpi_program.clone(), cpi_accounts, signer_seeds);
            token::close_account(cpi_ctx)?;

            // Close deposit_info and return lamports
            **ctx.accounts.sender.lamports.borrow_mut() += ctx.accounts.deposit_info.to_account_info().lamports();
            **ctx.accounts.deposit_info.to_account_info().lamports.borrow_mut() = 0;
        }

        Ok(())
    }
}

#[derive(Accounts)]
pub struct DepositCtx<'info> {
    #[account(mut)]
    pub sender: Signer<'info>,

    /// CHECK: Recipient can be any account
    pub recipient: UncheckedAccount<'info>,

    pub mint: Account<'info, token::Mint>,

    #[account(mut)]
    pub temp_ata: Account<'info, TokenAccount>,

    #[account(
        init,
        payer = sender,
        space = 8 + 32 + 32,
        seeds = [temp_ata.key().as_ref()],
        bump
    )]
    pub deposit_info: Account<'info, DepositInfo>,

    #[account(
        seeds = [b"atas_holder"],
        bump
    )]
    /// CHECK: PDA holding temp_ata
    pub atas_holder: UncheckedAccount<'info>,

    pub token_program: Program<'info, Token>,
    pub system_program: Program<'info, System>,
    pub associated_token_program: Program<'info, AssociatedToken>,
}


#[derive(Accounts)]
pub struct WithdrawCtx<'info> {
    #[account(mut)]
    pub recipient: Signer<'info>,

    #[account(mut)]
    /// CHECK: Sender receives lamports when deposit_info and temp ATA are closed
    pub sender: UncheckedAccount<'info>,

    pub mint: Account<'info, token::Mint>,

    #[account(mut)]
    pub recipient_ata: Account<'info, TokenAccount>,

    #[account(mut)]
    pub temp_ata: Account<'info, TokenAccount>,

    #[account(
        mut,
        seeds = [temp_ata.key().as_ref()],
        bump,
        close = sender
    )]
    pub deposit_info: Account<'info, DepositInfo>,

    #[account(
        seeds = [b"atas_holder"],
        bump
    )]
    /// CHECK: PDA authority for temp_ata; used as signer for transfers
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
}

#[error_code]
pub enum EscrowError {
    #[msg("Temporary token account is not owned by the sender.")]
    InvalidTempAtaOwner,

    #[msg("Only the designated recipient can withdraw.")]
    UnauthorizedRecipient,

    #[msg("Invalid PDA supplied.")]
    InvalidPDA,

    #[msg("Not enough tokens in the escrow to withdraw the requested amount.")]
    InsufficientFunds,
}
