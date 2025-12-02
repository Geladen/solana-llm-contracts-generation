use anchor_lang::prelude::*;
use anchor_spl::associated_token::AssociatedToken;
use anchor_spl::token::{self, CloseAccount, Mint, SetAuthority, Token, TokenAccount, Transfer, spl_token};

declare_id!("HEh6WxHNNRRBFQ8D72qKqjphXwtkjRh2WufaDVWB4ueQ");

#[program]
pub mod token_transfer {
    use super::*;

    /// Sender deposits tokens into escrow
    pub fn deposit(ctx: Context<DepositCtx>) -> Result<()> {
        let sender = &ctx.accounts.sender;
        let temp_ata = &ctx.accounts.temp_ata;
        let mint = &ctx.accounts.mint;

        require!(
            temp_ata.mint == mint.key(),
            EscrowError::TempAtaMintMismatch
        );
        require!(
            temp_ata.owner == sender.key(),
            EscrowError::TempAtaNotOwnedBySender
        );
        require!(temp_ata.amount > 0, EscrowError::TempAtaEmpty);

        let (atas_holder_pda, _atas_bump) =
            Pubkey::find_program_address(&[b"atas_holder"], ctx.program_id);

        let cpi_accounts = SetAuthority {
            account_or_mint: temp_ata.to_account_info(),
            current_authority: sender.to_account_info(),
        };
        let cpi_program = ctx.accounts.token_program.to_account_info();
        let cpi_ctx = CpiContext::new(cpi_program, cpi_accounts);

        // Fully-qualified AuthorityType
        token::set_authority(
            cpi_ctx,
            spl_token::instruction::AuthorityType::AccountOwner,
            Some(atas_holder_pda),
        )?;

        ctx.accounts.deposit_info.temp_ata = temp_ata.key();
        ctx.accounts.deposit_info.recipient = ctx.accounts.recipient.key();
        ctx.accounts.deposit_info.bump = ctx.bumps.deposit_info;

        Ok(())
    }

    /// Recipient withdraws tokens from escrow
    pub fn withdraw(ctx: Context<WithdrawCtx>, amount_to_withdraw: u64) -> Result<()> {
        let recipient = &ctx.accounts.recipient;
        let sender = &ctx.accounts.sender;
        let temp_ata = &ctx.accounts.temp_ata;
        let mint = &ctx.accounts.mint;

        require!(
            ctx.accounts.deposit_info.temp_ata == temp_ata.key(),
            EscrowError::DepositInfoMismatch
        );
        require!(
            ctx.accounts.deposit_info.recipient == recipient.key(),
            EscrowError::UnauthorizedRecipient
        );
        require!(temp_ata.mint == mint.key(), EscrowError::TempAtaMintMismatch);

        let decimals = mint.decimals as u32;
        let factor: u128 = 10u128
            .checked_pow(decimals)
            .ok_or(EscrowError::DecimalsOverflow)?;
        let amount_raw_u128: u128 = (amount_to_withdraw as u128)
            .checked_mul(factor)
            .ok_or(EscrowError::AmountOverflow)?;
        let amount_to_transfer: u64 = amount_raw_u128
            .try_into()
            .map_err(|_| error!(EscrowError::AmountOverflow))?;

        require!(amount_to_transfer > 0, EscrowError::InvalidWithdrawAmount);
        require!(
            amount_to_transfer <= temp_ata.amount,
            EscrowError::InsufficientEscrowBalance
        );

        let atas_bump = ctx.bumps.atas_holder_pda;
        let signer_seeds: &[&[&[u8]]] = &[&[b"atas_holder".as_ref(), &[atas_bump]]];

        let cpi_accounts = Transfer {
            from: temp_ata.to_account_info(),
            to: ctx.accounts.recipient_ata.to_account_info(),
            authority: ctx.accounts.atas_holder_pda.to_account_info(),
        };
        let cpi_program = ctx.accounts.token_program.to_account_info();
        let cpi_ctx =
            CpiContext::new_with_signer(cpi_program, cpi_accounts, signer_seeds);
        token::transfer(cpi_ctx, amount_to_transfer)?;

        if amount_to_transfer == temp_ata.amount {
            let cpi_accounts_close = CloseAccount {
                account: temp_ata.to_account_info(),
                destination: sender.to_account_info(),
                authority: ctx.accounts.atas_holder_pda.to_account_info(),
            };
            let cpi_program_close = ctx.accounts.token_program.to_account_info();
            let cpi_ctx_close = CpiContext::new_with_signer(
                cpi_program_close,
                cpi_accounts_close,
                signer_seeds,
            );
            token::close_account(cpi_ctx_close)?;

            let deposit_info_ai = ctx.accounts.deposit_info.to_account_info();
            let sender_ai = sender.to_account_info();

            let lamports_to_move = **deposit_info_ai.lamports.borrow();
            **sender_ai.lamports.borrow_mut() = sender_ai
                .lamports()
                .checked_add(lamports_to_move)
                .ok_or(EscrowError::AmountOverflow)?;
            **deposit_info_ai.lamports.borrow_mut() = 0;

            let mut data = deposit_info_ai.data.borrow_mut();
            for byte in data.iter_mut() {
                *byte = 0;
            }
        }

        Ok(())
    }
}

#[derive(Accounts)]
pub struct DepositCtx<'info> {
    /// CHECK: sender must sign to authorize token authority transfer
    #[account(mut)]
    pub sender: Signer<'info>,

    /// CHECK: recipient will receive the escrowed tokens; validated in deposit instruction
    pub recipient: UncheckedAccount<'info>,

    pub mint: Account<'info, Mint>,

    #[account(mut)]
    pub temp_ata: Account<'info, TokenAccount>,

    #[account(
        init,
        payer = sender,
        space = 8 + 32 + 32 + 1,
        seeds = [ temp_ata.key().as_ref() ],
        bump
    )]
    pub deposit_info: Account<'info, DepositInfo>,

    pub token_program: Program<'info, Token>,
    pub system_program: Program<'info, System>,
    pub associated_token_program: Program<'info, AssociatedToken>,
}

#[derive(Accounts)]
pub struct WithdrawCtx<'info> {
    /// CHECK: recipient must sign to withdraw tokens
    #[account(mut)]
    pub recipient: Signer<'info>,

    /// CHECK: original sender of the escrow; used as recipient when closing accounts
    #[account(mut)]
    pub sender: UncheckedAccount<'info>,

    pub mint: Account<'info, Mint>,

    /// CHECK: recipient's ATA; init_if_needed ensures it's valid
    #[account(
        init_if_needed,
        payer = recipient,
        associated_token::mint = mint,
        associated_token::authority = recipient
    )]
    pub recipient_ata: Account<'info, TokenAccount>,

    #[account(mut)]
    pub temp_ata: Account<'info, TokenAccount>,

    #[account(
        mut,
        seeds = [ temp_ata.key().as_ref() ],
        bump = deposit_info.bump,
        has_one = recipient,
        has_one = temp_ata
    )]
    pub deposit_info: Account<'info, DepositInfo>,

    /// CHECK: PDA that owns the temp_ata; signer via seeds
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
    pub bump: u8,
}

#[error_code]
pub enum EscrowError {
    #[msg("Deposit: temp ATA mint doesn't match provided mint.")]
    TempAtaMintMismatch,
    #[msg("Deposit: temp ATA is not owned by sender.")]
    TempAtaNotOwnedBySender,
    #[msg("Deposit: temp ATA has no tokens to escrow.")]
    TempAtaEmpty,
    #[msg("Withdraw: deposit info doesn't match the provided temp ATA.")]
    DepositInfoMismatch,
    #[msg("Withdraw: caller is not the recipient specified in deposit_info.")]
    UnauthorizedRecipient,
    #[msg("Decimals overflow when computing conversion factor.")]
    DecimalsOverflow,
    #[msg("Amount overflow after decimals conversion.")]
    AmountOverflow,
    #[msg("Invalid withdraw amount (zero).")]
    InvalidWithdrawAmount,
    #[msg("Insufficient escrow balance for requested withdraw.")]
    InsufficientEscrowBalance,
}
