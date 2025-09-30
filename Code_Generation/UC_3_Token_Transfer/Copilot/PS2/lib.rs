use anchor_lang::prelude::*;
use anchor_spl::associated_token::AssociatedToken;
use anchor_spl::token::{self, Mint, Token, TokenAccount, CloseAccount, SetAuthority, Transfer};
use anchor_spl::token::spl_token::instruction::AuthorityType;

declare_id!("CYpuXcPLZjhu5t9fN2wb5wBwuQbcCBZsePstmMVh3ikg");

#[program]
pub mod token_transfer {
    use super::*;

    pub fn deposit(ctx: Context<DepositCtx>) -> Result<()> {
        let temp_ata = &ctx.accounts.temp_ata;
        let sender = &ctx.accounts.sender;
        let mint = &ctx.accounts.mint;

        // Ensure token account currently belongs to the sender and mint matches
        require!(
            temp_ata.owner == sender.key(),
            EscrowError::TempAtaNotOwnedBySender
        );
        require!(temp_ata.mint == mint.key(), EscrowError::TempAtaMintMismatch);

        // Derive ATAs holder PDA pubkey deterministically
        let (atas_pubkey, _atas_bump) = Pubkey::find_program_address(&[b"atas_holder"], &crate::ID);

        // Use SetAuthority CPI: current_authority = sender (signer)
        let cpi_accounts = SetAuthority {
            account_or_mint: ctx.accounts.temp_ata.to_account_info(),
            current_authority: ctx.accounts.sender.to_account_info(),
        };
        let cpi_ctx = CpiContext::new(ctx.accounts.token_program.to_account_info(), cpi_accounts);
        token::set_authority(cpi_ctx, AuthorityType::AccountOwner, Some(atas_pubkey))?;

        // Initialize deposit_info (Anchor `init` did allocation). Store fields.
        let deposit_info = &mut ctx.accounts.deposit_info;
        deposit_info.temp_ata = ctx.accounts.temp_ata.key();
        deposit_info.recipient = ctx.accounts.recipient.key();

        Ok(())
    }

    pub fn withdraw(ctx: Context<WithdrawCtx>, amount_to_withdraw: u64) -> Result<()> {
        let deposit_info = &ctx.accounts.deposit_info;
        let temp_ata = &ctx.accounts.temp_ata;
        let recipient = &ctx.accounts.recipient;
        let mint = &ctx.accounts.mint;

        // Validate deposit info matches
        require!(
            deposit_info.temp_ata == temp_ata.key(),
            EscrowError::DepositInfoTempAtaMismatch
        );
        require!(
            deposit_info.recipient == recipient.key(),
            EscrowError::DepositInfoRecipientMismatch
        );
        require!(temp_ata.mint == mint.key(), EscrowError::TempAtaMintMismatch);

        // Convert amount_to_withdraw interpreted as whole tokens -> base units
        let decimals = mint.decimals as u32;
        let factor = 10u128
            .checked_pow(decimals)
            .ok_or(EscrowError::DecimalsOverflow)?;
        let amount_base = (amount_to_withdraw as u128)
            .checked_mul(factor)
            .ok_or(EscrowError::AmountOverflow)?;
        let amount: u64 = amount_base.try_into().map_err(|_| EscrowError::AmountOverflow)?;

        // Ensure sufficient balance
        require!(temp_ata.amount >= amount, EscrowError::InsufficientFundsInTempAta);

        // Derive ATAs holder PDA and bump for signer seeds
        let (atas_pubkey, atas_bump) = Pubkey::find_program_address(&[b"atas_holder"], &crate::ID);
        require!(
            atas_pubkey == ctx.accounts.atas_holder.key(),
            EscrowError::InvalidAtasHolder
        );

        // Stable bindings for signer seeds to avoid temporary-borrow issues
        let atas_seed: &[u8] = b"atas_holder";
        let atas_bump_arr: [u8; 1] = [atas_bump];
        let atas_seeds: &[&[u8]] = &[atas_seed, &atas_bump_arr];
        let signer_seeds: &[&[&[u8]]] = &[atas_seeds];

        // Transfer tokens from temp_ata (owned by PDA) to recipient_ata
        let cpi_accounts = Transfer {
            from: temp_ata.to_account_info(),
            to: ctx.accounts.recipient_ata.to_account_info(),
            authority: ctx.accounts.atas_holder.to_account_info(),
        };
        let cpi_ctx = CpiContext::new_with_signer(
            ctx.accounts.token_program.to_account_info(),
            cpi_accounts,
            signer_seeds,
        );
        token::transfer(cpi_ctx, amount)?;

        // If temp_ata had exactly `amount` before transfer, it is now empty: close it
        if temp_ata.amount == amount {
            let cpi_close = CloseAccount {
                account: ctx.accounts.temp_ata.to_account_info(),
                destination: ctx.accounts.sender.to_account_info(),
                authority: ctx.accounts.atas_holder.to_account_info(),
            };
            let cpi_ctx_close = CpiContext::new_with_signer(
                ctx.accounts.token_program.to_account_info(),
                cpi_close,
                signer_seeds,
            );
            token::close_account(cpi_ctx_close)?;
        }

        // deposit_info account has `close = sender` in account validation; Anchor will handle lamports return
        Ok(())
    }
}

#[derive(Accounts)]
pub struct DepositCtx<'info> {
    #[account(mut)]
    pub sender: Signer<'info>,

    /// CHECK: recipient is stored in DepositInfo and validated in withdraw; no direct type checks here
    pub recipient: UncheckedAccount<'info>,

    pub mint: Account<'info, Mint>,

    #[account(mut)]
    pub temp_ata: Account<'info, TokenAccount>,

    /// CHECK: atas_holder is a PDA used only as an authority signer for token CPIs; its address is derived and validated in instructions
    #[account(mut, seeds = [b"atas_holder"], bump)]
    pub atas_holder: UncheckedAccount<'info>,

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
}

#[derive(Accounts)]
pub struct WithdrawCtx<'info> {
    #[account(mut)]
    pub recipient: Signer<'info>,

    /// CHECK: sender is the rent destination when deposit_info is closed and the original owner of temp_ata; treated as a key only
    #[account(mut)]
    pub sender: UncheckedAccount<'info>,

    pub mint: Account<'info, Mint>,

    #[account(
        mut,
        associated_token::mint = mint,
        associated_token::authority = recipient,
    )]
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

    /// CHECK: atas_holder PDA is validated at runtime and used as signer for token CPIs
    #[account(seeds = [b"atas_holder"], bump)]
    pub atas_holder: UncheckedAccount<'info>,

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
    #[msg("Temporary token account is not owned by the sender.")]
    TempAtaNotOwnedBySender,
    #[msg("Temporary token account mint does not match provided mint.")]
    TempAtaMintMismatch,
    #[msg("DepositInfo temp_ata does not match provided temp_ata.")]
    DepositInfoTempAtaMismatch,
    #[msg("DepositInfo recipient does not match provided recipient.")]
    DepositInfoRecipientMismatch,
    #[msg("Insufficient funds in temporary token account.")]
    InsufficientFundsInTempAta,
    #[msg("Overflow converting decimals.")]
    DecimalsOverflow,
    #[msg("Overflow converting amount.")]
    AmountOverflow,
    #[msg("Invalid ATAs holder PDA passed.")]
    InvalidAtasHolder,
}
