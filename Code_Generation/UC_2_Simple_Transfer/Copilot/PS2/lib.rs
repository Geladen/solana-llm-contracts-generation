use anchor_lang::prelude::*;
use anchor_lang::system_program::{self, Transfer};

declare_id!("7Esh6BPKDq34MyeZXYYaSdGeAWK4v6vS719VZYcJ9Wn9");


#[program]
pub mod simple_copilot {
    use super::*;

    /// Owner deposits `amount` lamports into our PDA.  
    /// First call creates & initializes it; subsequent calls simply top it up.
    pub fn deposit(ctx: Context<Deposit>, amount: u64) -> Result<()> {
        // 1) Reject zero
        require!(amount > 0, Error::AmountMustBeGreaterThanZero);

        // 2) Update PDA state
        let pda = &mut ctx.accounts.balance_holder_pda;
        pda.sender = ctx.accounts.sender.key();
        pda.recipient = ctx.accounts.recipient.key();
        pda.amount = pda.amount.checked_add(amount).unwrap();

        // 3) Transfer lamports from sender → PDA
        let cpi_accounts = anchor_lang::system_program::Transfer {
            from: ctx.accounts.sender.to_account_info(),
            to: pda.to_account_info(),
        };
        let cpi_ctx = CpiContext::new(
            ctx.accounts.system_program.to_account_info(),
            cpi_accounts,
        );
        anchor_lang::system_program::transfer(cpi_ctx, amount)?;

        Ok(())
    }

    /// Recipient can withdraw up to `amount`.  
    /// When drained (amount → 0), PDA is closed and rent sent back to sender.
    pub fn withdraw(ctx: Context<Withdraw>, amount: u64) -> Result<()> {
        // 1) Reject zero
        require!(amount > 0, Error::AmountMustBeGreaterThanZero);

        let pda = &mut ctx.accounts.balance_holder_pda;

        // 2) Only designated recipient may call
        require!(
            pda.recipient == ctx.accounts.recipient.key(),
            Error::Unauthorized
        );

        // 3) Must have enough lamports
        require!(pda.amount >= amount, Error::InsufficientFunds);

        // 4) Move lamports out
        **pda.to_account_info().try_borrow_mut_lamports()? -= amount;
        **ctx.accounts
            .recipient
            .to_account_info()
            .try_borrow_mut_lamports()? += amount;

        // 5) Update PDA state
        pda.amount = pda.amount.checked_sub(amount).unwrap();

        // 6) If drained, Anchor closes PDA and refunds rent to `sender`
        //    because of `close = sender` in the account constraint below.
        Ok(())
    }
}

#[derive(Accounts)]
#[instruction(amount: u64)]
pub struct Deposit<'info> {
    /// The wallet depositing lamports
    #[account(mut)]
    pub sender: Signer<'info>,

    /// CHECK: only used as a PDA seed, we do not read or write data here
    pub recipient: UncheckedAccount<'info>,

    /// PDA holding (sender, recipient, amount).
    /// - init: creates the account on first deposit  
    /// - payer = sender: sender pays rent+fees  
    /// - space = discriminator(8) + Pubkey(32) + Pubkey(32) + u64(8)  
    /// - seeds = [ recipient, sender ], bump auto  
    #[account(
        init,
        payer = sender,
        space = BalanceHolderPDA::LEN,
        seeds = [recipient.key().as_ref(), sender.key().as_ref()],
        bump
    )]
    pub balance_holder_pda: Account<'info, BalanceHolderPDA>,

    /// For the system-program CPI transfer
    pub system_program: Program<'info, System>,
}

#[derive(Accounts)]
#[instruction(amount: u64)]
pub struct Withdraw<'info> {
    /// Must match `balance_holder_pda.recipient`
    #[account(mut)]
    pub recipient: Signer<'info>,

    /// CHECK: only used as a PDA seed and close refund target
    pub sender: UncheckedAccount<'info>,

    /// Mutable PDA with close-on-drain
    /// - seeds = [ recipient, sender ], bump auto  
    /// - has_one ensures the PDA’s fields match these signers  
    /// - close = sender refunds rent to sender when amount hits zero  
    #[account(
        mut,
        seeds = [recipient.key().as_ref(), sender.key().as_ref()],
        bump,
        has_one = recipient,
        has_one = sender,
        close = sender
    )]
    pub balance_holder_pda: Account<'info, BalanceHolderPDA>,

    /// Required for `close = sender`
    pub system_program: Program<'info, System>,
}

#[account]
pub struct BalanceHolderPDA {
    pub sender:    Pubkey,
    pub recipient: Pubkey,
    pub amount:    u64,
}

impl BalanceHolderPDA {
    /// 8 discriminator + 32 + 32 + 8
    pub const LEN: usize = 8 + 32 + 32 + 8;
}

#[error_code]
pub enum Error {
    #[msg("Amount must be greater than zero")]
    AmountMustBeGreaterThanZero,
    #[msg("You are not authorized to perform this action")]
    Unauthorized,
    #[msg("Insufficient funds in PDA")]
    InsufficientFunds,
    #[msg("PDA derivation mismatch")]
    InvalidPda,
    #[msg("Math overflow")]
    Overflow,
}

