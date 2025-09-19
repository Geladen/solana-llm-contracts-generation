use anchor_lang::prelude::*;
use anchor_lang::solana_program::{
    program::{invoke, invoke_signed},
    system_instruction,
};
use anchor_lang::cpi::system_program::close_account;
use anchor_lang::cpi::system_program::accounts::CloseAccount;
use borsh::{BorshDeserialize, BorshSerialize};

declare_id!("E3A3T3EAc3Pxo9ndX1Jy39ZUKp5Rn2svGsEmsa1gAodR");

/// On‐chain PDA state: exactly three fields
#[derive(BorshSerialize, BorshDeserialize)]
pub struct BalanceHolderPDA {
    pub sender:    Pubkey,
    pub recipient: Pubkey,
    pub amount:    u64,
}

#[program]
pub mod simple_copilot {
    use super::*;

    /// Owner deposits lamports into a PDA keyed by (recipient, sender).
    /// • First call: creates the PDA (rent-exempt + initial deposit).
    /// • Later calls: simply tops up the balance.
    pub fn deposit(ctx: Context<Deposit>, amount: u64) -> Result<()> {
        require!(amount > 0, ErrorCode::InvalidAmount);

        let sender    = &ctx.accounts.sender;
        let recipient = &ctx.accounts.recipient;
        let pda_acc   = &ctx.accounts.balance_holder_pda;
        let sys_prog  = &ctx.accounts.system_program;

        // Derive PDA address & bump
        let (expected_pda, bump) = Pubkey::find_program_address(
            &[recipient.key().as_ref(), sender.key().as_ref()],
            ctx.program_id,
        );
        require_keys_eq!(expected_pda, *pda_acc.key, ErrorCode::InvalidPDA);

        if pda_acc.owner != ctx.program_id {
            // Create PDA: rent-exempt + deposit
            let rent     = Rent::get()?;
            let space    = 8 + std::mem::size_of::<BalanceHolderPDA>() as u64;
            let lamports = rent.minimum_balance(space as usize) + amount;

            let ix = system_instruction::create_account(
                sender.key,
                pda_acc.key,
                lamports,
                space,
                ctx.program_id,
            );
            invoke_signed(
                &ix,
                &[
                    sender.to_account_info(),
                    pda_acc.to_account_info(),
                    sys_prog.to_account_info(),
                ],
                &[&[recipient.key().as_ref(), sender.key().as_ref(), &[bump]]],
            )?;

            // Initialize PDA state
            let mut data = pda_acc.try_borrow_mut_data()?;
            let state = BalanceHolderPDA {
                sender: *sender.key,
                recipient: *recipient.key,
                amount,
            };
            state.serialize(&mut &mut data[..])?;
        } else {
            // Top-up: transfer lamports and update state
            let ix = system_instruction::transfer(sender.key, pda_acc.key, amount);
            invoke(
                &ix,
                &[
                    sender.to_account_info(),
                    pda_acc.to_account_info(),
                    sys_prog.to_account_info(),
                ],
            )?;

            // Read-modify-write
            let mut data  = pda_acc.try_borrow_mut_data()?;
            let mut state = BalanceHolderPDA::try_from_slice(&data)?;
            require_keys_eq!(state.sender,    *sender.key,    ErrorCode::InvalidSender);
            require_keys_eq!(state.recipient, *recipient.key, ErrorCode::InvalidRecipient);

            state.amount = state.amount.checked_add(amount).unwrap();
            state.serialize(&mut &mut data[..])?;
        }

        Ok(())
    }

    /// Recipient withdraws arbitrary fractions until PDA is empty.
    /// When balance hits zero, closes the PDA and refunds rent to the sender.
    pub fn withdraw(ctx: Context<Withdraw>, amount: u64) -> Result<()> {
        require!(amount > 0, ErrorCode::InvalidAmount);

        let sender    = &ctx.accounts.sender;
        let recipient = &ctx.accounts.recipient;
        let pda_acc   = &ctx.accounts.balance_holder_pda;
        let sys_prog  = &ctx.accounts.system_program;

        // Derive PDA address & bump
        let (expected_pda, bump) = Pubkey::find_program_address(
            &[recipient.key().as_ref(), sender.key().as_ref()],
            ctx.program_id,
        );
        require_keys_eq!(expected_pda, *pda_acc.key, ErrorCode::InvalidPDA);

        // Read & validate
        let mut data  = pda_acc.try_borrow_mut_data()?;
        let mut state = BalanceHolderPDA::try_from_slice(&data)?;
        require_keys_eq!(state.sender,    *sender.key,    ErrorCode::InvalidSender);
        require_keys_eq!(state.recipient, *recipient.key, ErrorCode::InvalidRecipient);
        require!(state.amount >= amount, ErrorCode::InsufficientFunds);

        // Build PDA signer seeds
        let seeds: &[&[u8]]       = &[
            recipient.key().as_ref(),
            sender.key().as_ref(),
            &[bump],
        ];
        let signer_seeds: &[&[&[u8]]] = &[seeds];

        // Transfer out
        let ix = system_instruction::transfer(pda_acc.key, recipient.key, amount);
        invoke_signed(
            &ix,
            &[
                pda_acc.to_account_info(),
                recipient.to_account_info(),
                sys_prog.to_account_info(),
            ],
            signer_seeds,
        )?;

        // Update on-chain balance
        state.amount = state.amount.checked_sub(amount).unwrap();
        state.serialize(&mut &mut data[..])?;

        // If empty, close and refund rent
        if state.amount == 0 {
            let cpi_ctx = CpiContext::new_with_signer(
                sys_prog.to_account_info(),
                CloseAccount {
                    account:     pda_acc.to_account_info(),
                    destination: sender.to_account_info(),
                },
                signer_seeds,
            );
            close_account(cpi_ctx)?;
        }

        Ok(())
    }
}

//
// Accounts for `deposit`
//
#[derive(Accounts)]
pub struct Deposit<'info> {
    /// Owner sending lamports
    #[account(mut, signer)]
    pub sender: Signer<'info>,

    /// Designated recipient (PDA seed only)
    /// CHECK: no data read
    pub recipient: UncheckedAccount<'info>,

    /// PDA storing lamports & state
    #[account(mut)]
    pub balance_holder_pda: UncheckedAccount<'info>,

    /// System program for CPI
    pub system_program: Program<'info, System>,
}

//
// Accounts for `withdraw`
//
#[derive(Accounts)]
pub struct Withdraw<'info> {
    /// Recipient extracting lamports
    #[account(mut, signer)]
    pub recipient: Signer<'info>,

    /// Original owner (PDA seed only)
    /// CHECK: no data read
    pub sender: UncheckedAccount<'info>,

    /// PDA storing lamports & state
    #[account(mut)]
    pub balance_holder_pda: UncheckedAccount<'info>,

    /// Required for closing the PDA
    pub rent: Sysvar<'info, Rent>,

    /// System program for CPI
    pub system_program: Program<'info, System>,
}

//
// Custom errors
//
#[error_code]
pub enum ErrorCode {
    #[msg("Amount must be > 0.")]
    InvalidAmount,
    #[msg("Insufficient funds.")]
    InsufficientFunds,
    #[msg("PDA derivation mismatch.")]
    InvalidPDA,
    #[msg("Sender mismatch in PDA state.")]
    InvalidSender,
    #[msg("Recipient mismatch in PDA state.")]
    InvalidRecipient,
}

