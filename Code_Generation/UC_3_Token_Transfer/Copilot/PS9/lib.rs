use anchor_lang::prelude::*;
use anchor_spl::associated_token::AssociatedToken;
use anchor_spl::token::{self, Token, TokenAccount, Mint, SetAuthority, CloseAccount, Transfer};
use anchor_spl::token::spl_token::instruction::AuthorityType as SplAuthorityType;

declare_id!("DbnfKkh7HWgUQMEnLLiyjn92apQe4gn5rGbkED7SEWVe");

#[program]
pub mod token_transfer {
    use super::*;

    pub fn deposit(ctx: Context<DepositCtx>) -> Result<()> {
        // sender must sign
        require!(ctx.accounts.sender.is_signer, EscrowError::MissingSenderSignature);

        // temp_ata must be owned by sender before deposit
        require!(
            ctx.accounts.temp_ata.owner == ctx.accounts.sender.key(),
            EscrowError::TempAtaOwnerMismatch
        );

        // Derive ATAs holder PDA inside the program using the same seed as withdraw expects
        let (ata_holder_pda, _bump) =
            Pubkey::find_program_address(&[b"atas_holder".as_ref()], ctx.program_id);

        // Set authority of the token account to the derived PDA pubkey
        let cpi_accounts = SetAuthority {
            account_or_mint: ctx.accounts.temp_ata.to_account_info(),
            current_authority: ctx.accounts.sender.to_account_info(),
        };
        let cpi_program = ctx.accounts.token_program.to_account_info();
        let cpi_ctx = CpiContext::new(cpi_program, cpi_accounts);

        token::set_authority(
            cpi_ctx,
            SplAuthorityType::AccountOwner,
            Some(ata_holder_pda),
        )?;

        // Initialize deposit_info
        let deposit_info = &mut ctx.accounts.deposit_info;
        deposit_info.temp_ata = ctx.accounts.temp_ata.key();
        deposit_info.recipient = ctx.accounts.recipient.key();

        Ok(())
    }

    pub fn withdraw(ctx: Context<WithdrawCtx>, amount_to_withdraw: u64) -> Result<()> {
        // recipient must sign
        require!(ctx.accounts.recipient.is_signer, EscrowError::MissingRecipientSignature);

        // deposit_info must match temp_ata and recipient
        let deposit_info = &ctx.accounts.deposit_info;
        require!(
            deposit_info.temp_ata == ctx.accounts.temp_ata.key(),
            EscrowError::DepositInfoMismatch
        );
        require!(
            deposit_info.recipient == ctx.accounts.recipient.key(),
            EscrowError::InvalidRecipient
        );

        // ensure temp_ata is owned by atas_holder_pda
        require!(
            ctx.accounts.temp_ata.owner == ctx.accounts.atas_holder_pda.key(),
            EscrowError::TempAtaNotOwnedByPda
        );

        // validate recipient_ata: correct mint and owner
        require!(
            ctx.accounts.recipient_ata.mint == ctx.accounts.mint.key(),
            EscrowError::InvalidRecipientAta
        );
        require!(
            ctx.accounts.recipient_ata.owner == ctx.accounts.recipient.key(),
            EscrowError::InvalidRecipientAta
        );

        // convert human amount to smallest units using mint.decimals
        let decimals = ctx.accounts.mint.decimals as u32;
        let pow = 10u128
            .checked_pow(decimals)
            .ok_or(EscrowError::MathOverflow)?;
        let amount128 = (amount_to_withdraw as u128)
            .checked_mul(pow)
            .ok_or(EscrowError::MathOverflow)?;
        let amount: u64 = amount128.try_into().map_err(|_| EscrowError::MathOverflow)?;

        // check temp_ata balance
        let pre_balance = ctx.accounts.temp_ata.amount;
        require!(pre_balance >= amount, EscrowError::InsufficientFunds);

        // compute PDA bump and signer seeds so PDA can sign CPIs
        let (_pda, bump) = Pubkey::find_program_address(&[b"atas_holder".as_ref()], ctx.program_id);
        let seeds = &[b"atas_holder".as_ref(), &[bump]];
        let signer_seeds = &[&seeds[..]];

        // transfer tokens (signed by PDA)
        let cpi_accounts = Transfer {
            from: ctx.accounts.temp_ata.to_account_info(),
            to: ctx.accounts.recipient_ata.to_account_info(),
            authority: ctx.accounts.atas_holder_pda.to_account_info(),
        };
        let cpi_program = ctx.accounts.token_program.to_account_info();
        let cpi_ctx = CpiContext::new_with_signer(cpi_program, cpi_accounts, signer_seeds);
        token::transfer(cpi_ctx, amount)?;

        // if fully withdrawn, close temp_ata (lamports go to sender)
        if pre_balance == amount {
            let cpi_accounts_close = CloseAccount {
                account: ctx.accounts.temp_ata.to_account_info(),
                destination: ctx.accounts.sender.to_account_info(),
                authority: ctx.accounts.atas_holder_pda.to_account_info(),
            };
            let cpi_program_close = ctx.accounts.token_program.to_account_info();
            let cpi_ctx_close = CpiContext::new_with_signer(cpi_program_close, cpi_accounts_close, signer_seeds);
            token::close_account(cpi_ctx_close)?;
            // deposit_info is annotated with close = sender and will be closed by Anchor
        }

        Ok(())
    }
}

#[derive(Accounts)]
pub struct DepositCtx<'info> {
    /// Sender must sign and will pay for deposit_info init
    #[account(mut)]
    pub sender: Signer<'info>,

    /// Recipient of the escrow
    /// CHECK: safe because this account is only stored in DepositInfo and validated on withdraw
    pub recipient: UncheckedAccount<'info>,

    /// Mint of token being escrowed
    pub mint: Account<'info, Mint>,

    /// Temporary token account currently owned by sender
    #[account(mut)]
    pub temp_ata: Account<'info, TokenAccount>,

    /// DepositInfo PDA stores escrow state
    #[account(
        init,
        payer = sender,
        space = 8 + DepositInfo::SIZE,
        seeds = [temp_ata.key().as_ref()],
        bump
    )]
    pub deposit_info: Account<'info, DepositInfo>,

    pub token_program: Program<'info, Token>,

    pub system_program: Program<'info, System>,
    pub associated_token_program: Program<'info, AssociatedToken>,
    pub rent: Sysvar<'info, Rent>,
}

#[derive(Accounts)]
pub struct WithdrawCtx<'info> {
    /// Recipient must sign
    #[account(mut)]
    pub recipient: Signer<'info>,

    /// Sender receives rent and lamports from closed accounts
    /// CHECK: validated against DepositInfo.temp_ata when closing deposit_info
    #[account(mut)]
    pub sender: UncheckedAccount<'info>,

    pub mint: Account<'info, Mint>,

    /// Recipient's associated token account for mint
    #[account(mut)]
    pub recipient_ata: Account<'info, TokenAccount>,

    /// Temporary token account holding escrowed tokens (owned by atas_holder_pda)
    #[account(mut)]
    pub temp_ata: Account<'info, TokenAccount>,

    /// Deposit info PDA derived from temp_ata; closed to sender
    #[account(
        mut,
        seeds = [temp_ata.key().as_ref()],
        bump,
        close = sender
    )]
    pub deposit_info: Account<'info, DepositInfo>,

    /// ATAs Holder PDA (authority over temp_ata)
    /// CHECK: PDA derived from seed [b"atas_holder"; program_id]; used as signer for token CPI
    #[account(seeds = [b"atas_holder".as_ref()], bump)]
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

impl DepositInfo {
    pub const SIZE: usize = 32 + 32;
}

#[error_code]
pub enum EscrowError {
    #[msg("Sender signature missing")]
    MissingSenderSignature,
    #[msg("Recipient signature missing")]
    MissingRecipientSignature,
    #[msg("Temp ATA owner mismatch; expected sender to be owner before deposit")]
    TempAtaOwnerMismatch,
    #[msg("DepositInfo does not match provided temp_ata")]
    DepositInfoMismatch,
    #[msg("Invalid recipient for this deposit")]
    InvalidRecipient,
    #[msg("Insufficient funds in temp ATA")]
    InsufficientFunds,
    #[msg("Math overflow when computing token amounts")]
    MathOverflow,
    #[msg("Temp ATA is not owned by the expected PDA")]
    TempAtaNotOwnedByPda,
    #[msg("Invalid recipient ATA")]
    InvalidRecipientAta,
    #[msg("Missing PDA bump")]
    PdaBumpMissing,
}
