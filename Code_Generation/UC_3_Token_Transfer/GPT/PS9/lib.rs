use anchor_lang::prelude::*;
use anchor_spl::associated_token::AssociatedToken;
use anchor_spl::token::{self, CloseAccount, Mint, SetAuthority, Token, TokenAccount, Transfer};
use anchor_spl::token::spl_token::instruction::AuthorityType; // correct path

declare_id!("B97MchJirjk9Kopz1DgKWwe5qSk8hqSQUAi6zCacqHAN");

#[program]
pub mod token_transfer {
    use super::*;

    /// Sender deposits: transfer ownership of temp_ata to atas_holder PDA and create deposit_info PDA.
    pub fn deposit(ctx: Context<DepositCtx>) -> Result<()> {
        // Basic sanity checks
        require!(
            ctx.accounts.temp_ata.mint == ctx.accounts.mint.key(),
            ErrorCode::MintMismatch
        );
        require!(
            ctx.accounts.temp_ata.owner == ctx.accounts.sender.key(),
            ErrorCode::TempAtaNotOwnedBySender
        );

        // Derive atas_holder PDA (and bump)
        let (atas_holder_pda, _bump) =
            Pubkey::find_program_address(&[b"atas_holder"], ctx.program_id);

        // Change authority of the token account (temp_ata) to the PDA
        let cpi_accounts = SetAuthority {
            account_or_mint: ctx.accounts.temp_ata.to_account_info(),
            current_authority: ctx.accounts.sender.to_account_info(),
        };
        let cpi_program = ctx.accounts.token_program.to_account_info();

        token::set_authority(
            CpiContext::new(cpi_program, cpi_accounts),
            AuthorityType::AccountOwner,
            Some(atas_holder_pda),
        )?;

        // Initialize deposit_info fields
        let deposit_info = &mut ctx.accounts.deposit_info;
        deposit_info.temp_ata = ctx.accounts.temp_ata.key();
        deposit_info.recipient = ctx.accounts.recipient.key();

        Ok(())
    }

    /// Recipient withdraws `amount_to_withdraw` whole units (converted by mint.decimals).
    pub fn withdraw(ctx: Context<WithdrawCtx>, amount_to_withdraw: u64) -> Result<()> {
        // Validate deposit info references
        require!(
            ctx.accounts.deposit_info.temp_ata == ctx.accounts.temp_ata.key(),
            ErrorCode::DepositInfoTempAtaMismatch
        );
        require!(
            ctx.accounts.deposit_info.recipient == ctx.accounts.recipient.key(),
            ErrorCode::InvalidRecipient
        );
        require!(
            ctx.accounts.temp_ata.mint == ctx.accounts.mint.key(),
            ErrorCode::MintMismatch
        );

        // Derive expected ATAs holder PDA and bump
        let (expected_atas_holder, atas_bump) =
            Pubkey::find_program_address(&[b"atas_holder"], ctx.program_id);
        require!(
            expected_atas_holder == ctx.accounts.atas_holder_pda.key(),
            ErrorCode::InvalidAtasHolderPda
        );

        // Ensure temp_ata is owned by the PDA
        require!(
            ctx.accounts.temp_ata.owner == expected_atas_holder,
            ErrorCode::TempAtaNotOwnedByPda
        );

        // Convert whole units -> base units using mint.decimals
        let decimals = ctx.accounts.mint.decimals as u32;
        let multiplier = 10u128
            .checked_pow(decimals)
            .ok_or(error!(ErrorCode::AmountOverflow))?;
        let amount_base_u128 = (amount_to_withdraw as u128)
            .checked_mul(multiplier)
            .ok_or(error!(ErrorCode::AmountOverflow))?;
        let amount: u64 = amount_base_u128
            .try_into()
            .map_err(|_| error!(ErrorCode::AmountOverflow))?;

        let initial_balance = ctx.accounts.temp_ata.amount;
        require!(initial_balance >= amount, ErrorCode::InsufficientFunds);

        // Build PDA signer seeds correctly:
        // seeds slice must be &[&[u8]] and new_with_signer expects &[&[&[u8]]]
        let seed_bytes: &[u8] = &b"atas_holder"[..];
        let seeds: &[&[u8]] = &[seed_bytes, &[atas_bump]];
        let signer_seeds: &[&[&[u8]]] = &[seeds];

        // Transfer tokens from temp_ata (PDA-owned) to recipient_ata
        let cpi_accounts = Transfer {
            from: ctx.accounts.temp_ata.to_account_info(),
            to: ctx.accounts.recipient_ata.to_account_info(),
            authority: ctx.accounts.atas_holder_pda.to_account_info(),
        };
        let cpi_program = ctx.accounts.token_program.to_account_info();

        token::transfer(
            CpiContext::new_with_signer(cpi_program, cpi_accounts, signer_seeds),
            amount,
        )?;

        // If emptied, close temp_ata and deposit_info
        if initial_balance == amount {
            // Close the temp_ata (rent goes to sender)
            let cpi_close = CloseAccount {
                account: ctx.accounts.temp_ata.to_account_info(),
                destination: ctx.accounts.sender.to_account_info(),
                authority: ctx.accounts.atas_holder_pda.to_account_info(),
            };
            token::close_account(
                CpiContext::new_with_signer(
                    ctx.accounts.token_program.to_account_info(),
                    cpi_close,
                    signer_seeds,
                ),
            )?;

            // Close deposit_info and return lamports to sender
            ctx.accounts
                .deposit_info
                .close(ctx.accounts.sender.to_account_info())?;
        }

        Ok(())
    }
}

/// Deposit context (called by sender)
#[derive(Accounts)]
pub struct DepositCtx<'info> {
    /// Sender (must sign)
    #[account(mut)]
    pub sender: Signer<'info>,

    /// Recipient (only pubkey stored, not accessed in deposit)
    /// CHECK: We only record the recipient pubkey into deposit_info, no data access.
    pub recipient: UncheckedAccount<'info>,

    /// Token mint
    pub mint: Account<'info, Mint>,

    /// Temporary token account to be transferred to PDA
    #[account(
        mut,
        constraint = temp_ata.mint == mint.key() @ ErrorCode::MintMismatch,
        constraint = temp_ata.owner == sender.key() @ ErrorCode::TempAtaNotOwnedBySender
    )]
    pub temp_ata: Account<'info, TokenAccount>,

    /// DepositInfo PDA created here
    #[account(init, payer = sender, space = DepositInfo::LEN, seeds = [temp_ata.key().as_ref()], bump)]
    pub deposit_info: Account<'info, DepositInfo>,

    /// Programs
    pub token_program: Program<'info, Token>,
    pub system_program: Program<'info, System>,
    pub associated_token_program: Program<'info, AssociatedToken>,
}

/// Withdraw context (called by recipient)
#[derive(Accounts)]
pub struct WithdrawCtx<'info> {
    /// Recipient (must sign)
    #[account(mut)]
    pub recipient: Signer<'info>,

    /// Sender (lamports recipient when accounts are closed)
    /// CHECK: We only transfer lamports back when closing accounts, no other use.
    #[account(mut)]
    pub sender: UncheckedAccount<'info>,

    /// Token mint
    pub mint: Account<'info, Mint>,

    /// Recipient ATA
    #[account(
        init_if_needed,
        payer = recipient,
        associated_token::mint = mint,
        associated_token::authority = recipient
    )]
    pub recipient_ata: Account<'info, TokenAccount>,

    /// Temp token account held in escrow
    #[account(mut, constraint = temp_ata.mint == mint.key() @ ErrorCode::MintMismatch)]
    pub temp_ata: Account<'info, TokenAccount>,

    /// DepositInfo PDA related to temp_ata
    #[account(mut, seeds = [temp_ata.key().as_ref()], bump)]
    pub deposit_info: Account<'info, DepositInfo>,

    /// ATAs Holder PDA (seed = [b"atas_holder"])
    /// CHECK: PDA is derived and only used as authority for token CPIs.
    #[account(seeds = [b"atas_holder"], bump)]
    pub atas_holder_pda: UncheckedAccount<'info>,

    /// Programs & sysvars
    pub token_program: Program<'info, Token>,
    pub system_program: Program<'info, System>,
    pub associated_token_program: Program<'info, AssociatedToken>,
    pub rent: Sysvar<'info, Rent>,
}


/// DepositInfo PDA structure
#[account]
pub struct DepositInfo {
    pub temp_ata: Pubkey,
    pub recipient: Pubkey,
}
impl DepositInfo {
    // discriminator (8) + temp_ata (32) + recipient (32) = 72
    pub const LEN: usize = 8 + 32 + 32;
}

/// Errors
#[error_code]
pub enum ErrorCode {
    #[msg("Provided token account mint doesn't match the expected mint.")]
    MintMismatch,
    #[msg("Temporary token account is not owned by the sender.")]
    TempAtaNotOwnedBySender,
    #[msg("DepositInfo temp ATA mismatch.")]
    DepositInfoTempAtaMismatch,
    #[msg("Recipient does not match deposit info.")]
    InvalidRecipient,
    #[msg("ATAs holder PDA provided does not match program-derived address.")]
    InvalidAtasHolderPda,
    #[msg("Temporary token account is not owned by the expected PDA.")]
    TempAtaNotOwnedByPda,
    #[msg("Insufficient funds in escrowed token account.")]
    InsufficientFunds,
    #[msg("Amount overflow while converting decimals.")]
    AmountOverflow,
}
