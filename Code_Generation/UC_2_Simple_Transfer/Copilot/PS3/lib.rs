use anchor_lang::prelude::*;
use anchor_lang::system_program;                  // for Transfer & transfer()
use anchor_lang::solana_program::program::invoke_signed;
use anchor_lang::solana_program::system_instruction;

declare_id!("DLykriTeJXQ73Vr1DxTUAvm5AfQVFHAkFHvMy3ay3VWy");

#[program]
pub mod simple_copilot {
    use super::*;

    /// Deposit lamports into the PDA. If it’s brand‐new, CPI‐create it (rent+deposit) + init state.
    pub fn deposit(ctx: Context<DepositCtx>, amount_to_deposit: u64) -> Result<()> {
        let sender     = &ctx.accounts.sender;
        let recipient  = &ctx.accounts.recipient;
        let pda_info   = &ctx.accounts.balance_holder_pda.to_account_info();
        let system     = &ctx.accounts.system_program;
        let rent       = &ctx.accounts.rent;
        let program_id = ctx.program_id; // &Pubkey

        // Disallow zero
        require!(amount_to_deposit > 0, ErrorCode::InvalidAmount);

        // Seeds & bump
        let bump     = ctx.bumps.balance_holder_pda;
        let rec_key  = recipient.key();
        let send_key = sender.key();
        let bump_arr = [bump];
        let seeds: &[&[u8]] = &[
            rec_key.as_ref(),
            send_key.as_ref(),
            bump_arr.as_ref(),
        ];

        // Rent‐exempt lamports + space
        let space         = 8 + 32 + 32 + 8;
        let rent_lamports = rent.minimum_balance(space);

        if pda_info.owner != program_id {
            // FIRST deposit: create account with rent+deposit in one CPI
            let lamports_to_allocate = rent_lamports
                .checked_add(amount_to_deposit)
                .unwrap();

            let ix = system_instruction::create_account(
                &send_key,
                &pda_info.key(),
                lamports_to_allocate,
                space as u64,
                &program_id,
            );
            invoke_signed(
                &ix,
                &[
                    sender.to_account_info(),
                    pda_info.clone(),
                    system.to_account_info(),
                ],
                &[seeds],
            )?;

            // Now write initial state
            let mut data = pda_info.try_borrow_mut_data()?;
            let state = BalanceHolderPDA {
                sender:    send_key,
                recipient: rec_key,
                amount:    amount_to_deposit,
            };
            state.try_serialize(&mut *data)?;
        } else {
            // TOP‐UP: just transfer then bump on‐chain state
            let cpi_ctx = CpiContext::new(
                system.to_account_info(),
                system_program::Transfer {
                    from: sender.to_account_info(),
                    to:   pda_info.clone(),
                },
            );
            system_program::transfer(cpi_ctx, amount_to_deposit)?;

            let mut data = pda_info.try_borrow_mut_data()?;
            let mut slice: &[u8] = &*data;
            let mut slice_ref: &mut &[u8] = &mut slice;
            let mut state = BalanceHolderPDA::try_deserialize(&mut slice_ref)?;
            require_keys_eq!(state.sender,    send_key, ErrorCode::InvalidPDAData);
            require_keys_eq!(state.recipient, rec_key,  ErrorCode::InvalidPDAData);
            state.amount = state
                .amount
                .checked_add(amount_to_deposit)
                .ok_or(error!(ErrorCode::Overflow))?;
            state.try_serialize(&mut *data)?;
        }

        Ok(())
    }

    /// Withdraw lamports by hand‐moving lamports from the PDA → recipient.
    /// Anchor will auto‐close the PDA (rent → sender) once `amount` hits zero.
    pub fn withdraw(ctx: Context<WithdrawCtx>, amount_to_withdraw: u64) -> Result<()> {
    // 1) Pull out AccountInfos up front (no borrows of the `Account<...>` remain)
    let recipient_info = ctx.accounts.recipient.to_account_info();
    let sender_info    = ctx.accounts.sender.to_account_info();
    let pda_info       = ctx.accounts.balance_holder_pda.to_account_info();

    // 2) Now get a mutable reference to the PDA state
    let vault = &mut ctx.accounts.balance_holder_pda;

    // 3) State‐based guards
    require!(amount_to_withdraw > 0, ErrorCode::InvalidAmount);
    require!(vault.amount >= amount_to_withdraw, ErrorCode::InsufficientFunds);

    // 4) Update the on‐chain `amount`
    vault.amount = vault.amount.checked_sub(amount_to_withdraw).unwrap();

    // 5) Move lamports: debit PDA, credit recipient
    **recipient_info.lamports.borrow_mut() += amount_to_withdraw;
    **pda_info.lamports.borrow_mut()       -= amount_to_withdraw;

    // 6) If fully drained, refund the remaining rent reserve back to sender
    if vault.amount == 0 {
        let remaining = **pda_info.lamports.borrow();
        **sender_info.lamports.borrow_mut() += remaining;
        **pda_info.lamports.borrow_mut() = 0;
    }

    Ok(())
}


}

//
// ACCOUNTS
//

#[derive(Accounts)]
pub struct DepositCtx<'info> {
    #[account(mut)]
    pub sender: Signer<'info>,

    /// CHECK: only used as a seed, no data read or written
    pub recipient: UncheckedAccount<'info>,

    /// CHECK: PDA is manually created via CPI in `deposit`
    #[account(
        mut,
        seeds = [recipient.key().as_ref(), sender.key().as_ref()],
        bump,
    )]
    pub balance_holder_pda: UncheckedAccount<'info>,

    pub system_program: Program<'info, System>,
    pub rent:           Sysvar<'info, Rent>,
}

#[derive(Accounts)]
pub struct WithdrawCtx<'info> {
    #[account(mut)]
    pub recipient: Signer<'info>,

    /// CHECK: only used for seed derivation & `has_one` validation
    /// We mark it `mut` so the PDA can refund rent to it on close.
    #[account(mut)]
    pub sender: UncheckedAccount<'info>,

    #[account(
        mut,
        seeds = [recipient.key().as_ref(), sender.key().as_ref()],
        bump,
        has_one = sender,
        has_one = recipient
    )]
    pub balance_holder_pda: Account<'info, BalanceHolderPDA>,

    pub system_program: Program<'info, System>,
}

//
// STATE & ERRORS
//

#[account]
#[derive(Default)]
pub struct BalanceHolderPDA {
    pub sender:    Pubkey,
    pub recipient: Pubkey,
    pub amount:    u64,
}

#[error_code]
pub enum ErrorCode {
    #[msg("Amount must be greater than zero")]
    InvalidAmount,

    #[msg("Insufficient funds available for withdrawal")]
    InsufficientFunds,

    #[msg("PDA state does not match provided sender/recipient")]
    InvalidPDAData,

    #[msg("Overflow in balance calculation")]
    Overflow,
}

