use anchor_lang::prelude::*;
use anchor_lang::system_program;

declare_id!("ABHkYugQeLho88hFg7iuStareXtNofPVCnoj1K3jDTa4");

// Program: simple_transfer
// PDA seeds: [recipient.key().as_ref(), sender.key().as_ref()]
// PDA state struct: BalanceHolderPDA { sender, recipient, amount }

#[program]
pub mod simple_transfer {
    use super::*;

    /// Deposit lamports from `sender` into the PDA associated with (recipient, sender).
    /// Initializes the PDA if needed. Requires `sender` signature.
    pub fn deposit(ctx: Context<DepositCtx>, amount_to_deposit: u64) -> Result<()> {
        require!(amount_to_deposit > 0, ErrorCode::InvalidAmount);

        // Transfer lamports from sender to PDA via system program CPI
        let cpi_accounts = system_program::Transfer {
            from: ctx.accounts.sender.to_account_info(),
            to: ctx.accounts.balance_holder_pda.to_account_info(),
        };
        let cpi_ctx = CpiContext::new(ctx.accounts.system_program.to_account_info(), cpi_accounts);
        system_program::transfer(cpi_ctx, amount_to_deposit)?;

        // Initialize PDA state
        let pda = &mut ctx.accounts.balance_holder_pda;
        pda.sender = ctx.accounts.sender.key();
        pda.recipient = ctx.accounts.recipient.key();
        pda.amount = pda
            .amount
            .checked_add(amount_to_deposit)
            .ok_or(ErrorCode::Overflow)?;

        Ok(())
    }

    /// Withdraw lamports from PDA to the designated recipient.
    /// Requires recipient signature. If the withdrawal reduces the stored amount to 0,
    /// the PDA account is closed and remaining lamports (rent) are returned to `sender`.
    pub fn withdraw(ctx: Context<WithdrawCtx>, amount_to_withdraw: u64) -> Result<()> {
    require!(amount_to_withdraw > 0, ErrorCode::InvalidAmount);

    // First, extract account infos we’ll need for lamports transfer
    let pda_info = ctx.accounts.balance_holder_pda.to_account_info();
    let recipient_info = ctx.accounts.recipient.to_account_info();

    // Then borrow the PDA state mutably
    let pda = &mut ctx.accounts.balance_holder_pda;

    // Validate that PDA state matches accounts
    require!(pda.recipient == ctx.accounts.recipient.key(), ErrorCode::Unauthorized);
    require!(pda.sender == ctx.accounts.sender.key(), ErrorCode::InvalidPDAState);
    require!(pda.amount >= amount_to_withdraw, ErrorCode::InsufficientFunds);

    // Perform lamports transfer
    {
        let mut pda_lamports = pda_info.try_borrow_mut_lamports()?;
        let mut recipient_lamports = recipient_info.try_borrow_mut_lamports()?;

        let new_recipient = (*recipient_lamports)
            .checked_add(amount_to_withdraw)
            .ok_or(ErrorCode::Overflow)?;
        let new_pda = (*pda_lamports)
            .checked_sub(amount_to_withdraw)
            .ok_or(ErrorCode::InsufficientFunds)?;

        **recipient_lamports = new_recipient;
        **pda_lamports = new_pda;
    }

    // Update PDA logical amount
    pda.amount = pda
        .amount
        .checked_sub(amount_to_withdraw)
        .ok_or(ErrorCode::Overflow)?;

    // If balance zero, close PDA and return rent to sender
    if pda.amount == 0 {
        ctx.accounts
            .balance_holder_pda
            .close(ctx.accounts.sender.to_account_info())?;
    }

    Ok(())
}

}

// ------------------------- Accounts Contexts -------------------------

#[derive(Accounts)]
pub struct DepositCtx<'info> {
    #[account(mut)]
    pub sender: Signer<'info>,

    /// CHECK: We only use the pubkey for PDA derivation and store it in the PDA.
    pub recipient: UncheckedAccount<'info>,

    #[account(
        init,
        payer = sender,
        space = BalanceHolderPDA::LEN,
        seeds = [recipient.key().as_ref(), sender.key().as_ref()],
        bump
    )]
    pub balance_holder_pda: Account<'info, BalanceHolderPDA>,

    pub system_program: Program<'info, System>,
}


#[derive(Accounts)]
pub struct WithdrawCtx<'info> {
    /// Recipient must sign to withdraw.
    #[account(mut)]
    pub recipient: Signer<'info>,

    /// CHECK: This is only used to receive lamports when closing the PDA.
    /// We don’t read or write any data from this account.
    #[account(mut)]
    pub sender: UncheckedAccount<'info>,

    /// PDA derived with seeds [recipient.key(), sender.key()]
    #[account(mut, seeds = [recipient.key().as_ref(), sender.key().as_ref()], bump)]
    pub balance_holder_pda: Account<'info, BalanceHolderPDA>,

    /// Rent sysvar included per specification.
    pub rent: Sysvar<'info, Rent>,
}


// ------------------------- PDA State -------------------------

// Must be named exactly `BalanceHolderPDA` and contain exactly three fields with these names.
#[account]
pub struct BalanceHolderPDA {
    pub sender: Pubkey,
    pub recipient: Pubkey,
    pub amount: u64,
}

impl BalanceHolderPDA {
    // discriminator (8) + 32 + 32 + 8
    pub const LEN: usize = 8 + 32 + 32 + 8;
}

// ------------------------- Errors -------------------------

#[error_code]
pub enum ErrorCode {
    #[msg("Amount must be > 0")]
    InvalidAmount,

    #[msg("PDA does not have enough funds")]
    InsufficientFunds,

    #[msg("PDA state mismatch (sender/recipient mismatch)")]
    InvalidPDAState,

    #[msg("Unauthorized: only designated recipient may withdraw")]
    Unauthorized,

    #[msg("Integer overflow")]
    Overflow,
}

