use anchor_lang::prelude::*;

declare_id!("6gnE2sbs9nyGLzw2MzZnHCwvD5YTu1LcoypRkxfqKZc2"); // Replace with your program id when deploying

#[program]
pub mod simple_gpt {
    use super::*;

    pub fn deposit(ctx: Context<Deposit>, amount_to_deposit: u64) -> Result<()> {
    require!(amount_to_deposit > 0, ErrorCode::InvalidAmount);

    let sender = &ctx.accounts.sender;
    let recipient = &ctx.accounts.recipient;
    let pda_account = &mut ctx.accounts.balance_holder_pda;

    // ✅ Use system_program transfer instead of direct lamport mutation
    let ix = anchor_lang::solana_program::system_instruction::transfer(
        &sender.key(),
        &pda_account.key(),
        amount_to_deposit,
    );
    anchor_lang::solana_program::program::invoke(
        &ix,
        &[
            sender.to_account_info(),
            pda_account.to_account_info(),
            ctx.accounts.system_program.to_account_info(),
        ],
    )?;

    // Initialize or update state
    if pda_account.amount == 0 && pda_account.sender == Pubkey::default() {
        // First time: initialize
        pda_account.sender = sender.key();
        pda_account.recipient = recipient.key();
        pda_account.amount = amount_to_deposit;
    } else {
        require!(pda_account.sender == sender.key(), ErrorCode::InvalidSender);
        require!(pda_account.recipient == recipient.key(), ErrorCode::InvalidRecipient);

        pda_account.amount = pda_account
            .amount
            .checked_add(amount_to_deposit)
            .ok_or(ErrorCode::Overflow)?;
    }

    Ok(())
}


// ---------- Withdraw handler ----------
pub fn withdraw(ctx: Context<Withdraw>, amount_to_withdraw: u64) -> Result<()> {
    require!(amount_to_withdraw > 0, ErrorCode::ZeroAmount);

    // 1) Bind AccountInfos up-front so temporaries do not get dropped while borrows are active.
    let pda_ai = ctx.accounts.balance_holder_pda.to_account_info();
    let recipient_ai = ctx.accounts.recipient.to_account_info();
    let sender_ai = ctx.accounts.sender.to_account_info();

    // 2) Read PDA state into locals (immutable access only)
    let pda_snapshot = {
        // read-only borrow of account data (cheap, small)
        let acc = &ctx.accounts.balance_holder_pda;
        (acc.sender, acc.recipient, acc.amount)
    };
    let (pda_sender_key, pda_recipient_key, pda_amount) = pda_snapshot;

    // 3) Validate identity and funds
    require!(pda_recipient_key == ctx.accounts.recipient.key(), ErrorCode::InvalidRecipient);
    require!(pda_sender_key == ctx.accounts.sender.key(), ErrorCode::InvalidSender);
    require!(pda_amount >= amount_to_withdraw, ErrorCode::InsufficientFunds);

    // Calculate remaining logical amount
    let remaining_amount = pda_amount
        .checked_sub(amount_to_withdraw)
        .ok_or(ErrorCode::Underflow)?;

    // 4) Move lamports using short-lived borrows (must happen before we mutably borrow the PDA Account).
    {
        // Borrow lamports refs (these are RefMut<&mut u64>; they must be deref'd with **)
        let mut pda_lams = pda_ai.try_borrow_mut_lamports()?;
        let mut recipient_lams = recipient_ai.try_borrow_mut_lamports()?;

        // sanity check: actual lamports available on PDA
        require!(**pda_lams >= amount_to_withdraw, ErrorCode::InsufficientFunds);

        // move requested amount to recipient
        **pda_lams = (**pda_lams)
            .checked_sub(amount_to_withdraw)
            .ok_or(ErrorCode::Underflow)?;
        **recipient_lams = (**recipient_lams)
            .checked_add(amount_to_withdraw)
            .ok_or(ErrorCode::Overflow)?;

        // If this withdrawal will fully empty the logical amount, also drain any remaining lamports
        // (rent-exempt reserve or dust) to the sender now — still inside the same short scope so borrows drop afterwards.
        if remaining_amount == 0 {
            // NOTE: sender_ai is bound above; borrow its lamports now
            let mut sender_lams = sender_ai.try_borrow_mut_lamports()?;

            if **pda_lams > 0 {
                let moving = **pda_lams;
                **pda_lams = (**pda_lams)
                    .checked_sub(moving)
                    .ok_or(ErrorCode::Underflow)?;
                **sender_lams = (**sender_lams)
                    .checked_add(moving)
                    .ok_or(ErrorCode::Overflow)?;
            }
            // After this block, pda_lams == 0 and remaining lamports moved to sender.
        }
    } // <-- pda_lams / recipient_lams / sender_lams dropped here

    // 5) Mutably update the PDA state (no overlapping borrow with lamport borrows)
    {
        let pda_account = &mut ctx.accounts.balance_holder_pda;
        pda_account.amount = remaining_amount;

        if remaining_amount == 0 {
            // zero the account data so it's inert (practically closed)
            // We can zero the account's data buffer by writing zeros into it.
            let mut data = pda_ai.try_borrow_mut_data()?;
            for b in data.iter_mut() {
                *b = 0;
            }
        }
    }

    Ok(())
}


}

#[derive(Accounts)]
pub struct Deposit<'info> {
    #[account(mut)]
    pub sender: Signer<'info>,

    /// CHECK: only used for PDA derivation
    pub recipient: UncheckedAccount<'info>,

    #[account(
        init,
        payer = sender,
        space = 8 + 32 + 32 + 8,
        seeds = [recipient.key().as_ref(), sender.key().as_ref()],
        bump
    )]
    pub balance_holder_pda: Account<'info, BalanceHolderPDA>,

    pub system_program: Program<'info, System>,
}

// ---------- Accounts struct for Withdraw ----------
#[derive(Accounts)]
pub struct Withdraw<'info> {
    /// recipient (signer, mut) — must sign the withdraw txn
    #[account(mut)]
    pub recipient: Signer<'info>,

    /// CHECK: sender is only used as a PDA seed and to receive refunded lamports on close.
    /// Not a signer here.
    #[account(mut)]
    pub sender: UncheckedAccount<'info>,

    /// balance_holder_pda — PDA owned by this program storing BalanceHolderPDA state.
    /// Seeds: [recipient.key(), sender.key()]
    #[account(mut, seeds = [ recipient.key().as_ref(), sender.key().as_ref() ], bump)]
    pub balance_holder_pda: Account<'info, BalanceHolderPDA>,

    /// rent (included per your spec — not used for CPI here)
    pub rent: Sysvar<'info, Rent>,
}



#[account]
pub struct BalanceHolderPDA {
    pub sender: Pubkey,
    pub recipient: Pubkey,
    pub amount: u64,
}

#[error_code]
pub enum ErrorCode {
    #[msg("Amount must be greater than zero")]
    InvalidAmount,
    #[msg("Insufficient funds")]
    InsufficientFunds,
    #[msg("Overflow occurred")]
    Overflow,
    #[msg("Underflow occurred")]
    Underflow,
    #[msg("Invalid sender")]
    InvalidSender,
    #[msg("Invalid recipient")]
    InvalidRecipient,
    #[msg("Amount must be greater than zero.")]
    ZeroAmount,
}
