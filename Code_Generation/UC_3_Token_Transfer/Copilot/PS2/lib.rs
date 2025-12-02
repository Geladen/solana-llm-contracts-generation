use anchor_lang::prelude::*;
use anchor_lang::solana_program::pubkey::Pubkey;
use anchor_spl::associated_token::AssociatedToken;
use anchor_spl::token::{self, TokenAccount, Token, Mint, SetAuthority, Transfer, CloseAccount};
use anchor_spl::token::spl_token::instruction::AuthorityType;

declare_id!("ECFxAiBrTTgnpkZfAe2wfQAFaAcfi6sGa8Eq4hdjMjjG");

#[program]
pub mod token_transfer {
    use super::*;

    // deposit (sender signs)
    pub fn deposit(ctx: Context<DepositCtx>) -> Result<()> {
        let temp_ata = &ctx.accounts.temp_ata;
        let sender = &ctx.accounts.sender;
        let mint = &ctx.accounts.mint;

        // Pre-CPI checks (read-only; safe)
        require!(temp_ata.mint == mint.key(), EscrowError::TempAtaMintMismatch);
        require!(temp_ata.owner == sender.key(), EscrowError::TempAtaNotOwnedBySender);
        require!(temp_ata.amount > 0, EscrowError::TempAtaEmpty);

        // Initialize deposit info
        let deposit_info = &mut ctx.accounts.deposit_info;
        deposit_info.temp_ata = temp_ata.key();
        deposit_info.recipient = ctx.accounts.recipient.key();

        // Perform CPI to set the token account owner to the PDA
        let cpi_program = ctx.accounts.token_program.to_account_info();
        let cpi_accounts = SetAuthority {
            account_or_mint: ctx.accounts.temp_ata.to_account_info(),
            current_authority: ctx.accounts.sender.to_account_info(),
        };
        let cpi_ctx = CpiContext::new(cpi_program, cpi_accounts);
        token::set_authority(cpi_ctx, AuthorityType::AccountOwner, Some(ctx.accounts.atas_holder_pda.key()))?;

        // Do not attempt to read temp_ata.owner here: CPI state changes won't be reflected in
        // the already-deserialized `temp_ata` Account<T>. Tests / clients should fetch the token
        // account after the transaction to observe the new owner.

        Ok(())
    }


    pub fn withdraw(ctx: Context<WithdrawCtx>, amount_to_withdraw: u64) -> Result<()> {
        // validate deposit mapping and basic account invariants
        let deposit_info = &ctx.accounts.deposit_info;
        require!(deposit_info.temp_ata == ctx.accounts.temp_ata.key(), EscrowError::DepositInfoTempAtaMismatch);
        require!(deposit_info.recipient == ctx.accounts.recipient.key(), EscrowError::DepositInfoRecipientMismatch);

        let temp_ata = &ctx.accounts.temp_ata;
        let mint = &ctx.accounts.mint;
        require!(temp_ata.mint == mint.key(), EscrowError::TempAtaMintMismatch);

        let recipient_ata = &ctx.accounts.recipient_ata;
        require!(recipient_ata.mint == mint.key(), EscrowError::RecipientAtaMintMismatch);
        require!(recipient_ata.owner == ctx.accounts.recipient.key(), EscrowError::RecipientAtaOwnerMismatch);

        // derive expected atas_holder PDA and bump
        let (expected_pda, atas_holder_bump) = Pubkey::find_program_address(&[b"atas_holder"], ctx.program_id);
        require!(expected_pda == ctx.accounts.atas_holder_pda.key(), EscrowError::InvalidPda);

        require!(temp_ata.owner == ctx.accounts.atas_holder_pda.key(), EscrowError::TempAtaNotOwnedByPda);

        // Convert user-facing amount_to_withdraw into raw token base units using mint.decimals
        // Safe conversion: amount_to_withdraw * 10^decimals, checking overflow
        let decimals = mint.decimals as u32;
        let mut multiplier: u128 = 1;
        for _ in 0..decimals {
            multiplier = multiplier.checked_mul(10).ok_or(EscrowError::AmountOverflow)?;
        }
        let raw_amount128 = (amount_to_withdraw as u128)
            .checked_mul(multiplier)
            .ok_or(EscrowError::AmountOverflow)?;
        let raw_amount: u64 = raw_amount128
            .try_into()
            .map_err(|_| EscrowError::AmountOverflow)?;

        // snapshot pre-transfer balance (base units)
        let pre_balance = temp_ata.amount;
        require!(pre_balance >= raw_amount, EscrowError::InsufficientFunds);

        // prepare signer seeds for PDA
        let seeds: &[&[u8]] = &[b"atas_holder", &[atas_holder_bump]];
        let signer = &[seeds];

        // Transfer raw_amount (base units) from temp_ata to recipient_ata, signed by PDA
        let cpi_accounts = Transfer {
            from: ctx.accounts.temp_ata.to_account_info(),
            to: ctx.accounts.recipient_ata.to_account_info(),
            authority: ctx.accounts.atas_holder_pda.to_account_info(),
        };
        let cpi_program = ctx.accounts.token_program.to_account_info();
        let cpi_ctx = CpiContext::new_with_signer(cpi_program, cpi_accounts, signer);
        token::transfer(cpi_ctx, raw_amount)?;

        // If fully drained (based on pre-transfer base-unit balance), close the temp ATA (PDA signs)
        if pre_balance == raw_amount {
            let cpi_accounts_close = CloseAccount {
                account: ctx.accounts.temp_ata.to_account_info(),
                destination: ctx.accounts.sender.to_account_info(),
                authority: ctx.accounts.atas_holder_pda.to_account_info(),
            };
            let cpi_program_close = ctx.accounts.token_program.to_account_info();
            let cpi_ctx_close = CpiContext::new_with_signer(cpi_program_close, cpi_accounts_close, signer);
            token::close_account(cpi_ctx_close)?;
        }

        Ok(())
    }
}

#[account]
pub struct DepositInfo {
    pub temp_ata: Pubkey,
    pub recipient: Pubkey,
}

#[derive(Accounts)]
pub struct DepositCtx<'info> {
    /// CHECK: signer and payer; we validate `temp_ata.owner == sender.key()` at runtime
    #[account(mut, signer)]
    pub sender: AccountInfo<'info>,

    /// CHECK: recipient pubkey is stored in DepositInfo and validated during withdraw
    pub recipient: UncheckedAccount<'info>,

    pub mint: Account<'info, Mint>,

    /// Temporary token account owned by sender; validated by constraint
    #[account(mut, constraint = temp_ata.mint == mint.key() @ EscrowError::TempAtaMintMismatch)]
    pub temp_ata: Account<'info, TokenAccount>,

    /// DepositInfo PDA storing temp_ata -> recipient mapping. Created here.
    /// PDA seeds = [temp_ata.key().as_ref()]
    #[account(
        init,
        payer = sender,
        space = 8 + 32 + 32,
        seeds = [temp_ata.key().as_ref()],
        bump
    )]
    pub deposit_info: Account<'info, DepositInfo>,

    /// CHECK: atas_holder_pda is a PDA used as the new owner/authority of temp_ata.
    /// It holds no data and is only used as an authority; ownership and bump are validated on withdraw.
    #[account(mut, seeds = [b"atas_holder"], bump)]
    pub atas_holder_pda: UncheckedAccount<'info>,

    pub token_program: Program<'info, Token>,
    pub system_program: Program<'info, System>,
    pub associated_token_program: Program<'info, AssociatedToken>,
}

#[derive(Accounts)]
pub struct WithdrawCtx<'info> {
    /// CHECK: recipient must sign; recipient ATA owner is validated at runtime
    #[account(mut, signer)]
    pub recipient: AccountInfo<'info>,

    /// CHECK: sender is the lamports recipient when deposit_info is closed; verified by deposit_info.close = sender
    #[account(mut)]
    pub sender: UncheckedAccount<'info>,

    pub mint: Account<'info, Mint>,

    /// Recipient's token account to receive tokens (must match mint)
    #[account(mut, constraint = recipient_ata.mint == mint.key() @ EscrowError::RecipientAtaMintMismatch)]
    pub recipient_ata: Account<'info, TokenAccount>,

    /// Temporary token account holding escrowed tokens (owned by atas_holder_pda)
    #[account(mut, constraint = temp_ata.mint == mint.key() @ EscrowError::TempAtaMintMismatch)]
    pub temp_ata: Account<'info, TokenAccount>,

    /// DepositInfo PDA to close (seeds = [temp_ata.key().as_ref()])
    /// Closed to sender on withdraw completion so rent goes to sender
    #[account(
        mut,
        seeds = [temp_ata.key().as_ref()],
        bump,
        close = sender
    )]
    pub deposit_info: Account<'info, DepositInfo>,

    /// CHECK: atas_holder_pda is PDA authority for temp_ata; its bump is derived at runtime and ownership is validated
    #[account(mut, seeds = [b"atas_holder"], bump)]
    pub atas_holder_pda: UncheckedAccount<'info>,

    pub token_program: Program<'info, Token>,
    pub system_program: Program<'info, System>,
    pub associated_token_program: Program<'info, AssociatedToken>,
    pub rent: Sysvar<'info, Rent>,
}

#[error_code]
pub enum EscrowError {
    #[msg("temp_ata mint does not match provided mint")]
    TempAtaMintMismatch,
    #[msg("temp_ata is not owned by the sender")]
    TempAtaNotOwnedBySender,
    #[msg("temp_ata is empty")]
    TempAtaEmpty,
    #[msg("deposit_info temp_ata mismatch")]
    DepositInfoTempAtaMismatch,
    #[msg("deposit_info recipient mismatch")]
    DepositInfoRecipientMismatch,
    #[msg("recipient ATA mint mismatch")]
    RecipientAtaMintMismatch,
    #[msg("recipient ATA owner mismatch")]
    RecipientAtaOwnerMismatch,
    #[msg("missing PDA bump")]
    MissingPdaBump,
    #[msg("invalid PDA")]
    InvalidPda,
    #[msg("temp_ata not owned by atas_holder PDA")]
    TempAtaNotOwnedByPda,
    #[msg("insufficient funds in temp_ata")]
    InsufficientFunds,
    #[msg("amount conversion overflow")]
    AmountOverflow,
}
