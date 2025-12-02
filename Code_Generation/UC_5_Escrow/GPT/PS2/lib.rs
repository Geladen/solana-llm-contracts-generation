use anchor_lang::prelude::*;
use anchor_lang::solana_program::{program::invoke_signed, program::invoke, system_instruction};
use anchor_lang::solana_program::keccak::hash;
use borsh::{BorshDeserialize, BorshSerialize};

declare_id!("3TarCgEN7W8FUoQMvRNoSrfcx8scLAH23Ub9VikYHeF5");

#[program]
pub mod escrow {
    use super::*;

    /// Seller initializes the escrow PDA and sets expected deposit amount.
    /// Accounts:
    /// - seller (signer, mutable)
    /// - buyer (reference)
    /// - escrow_info (PDA account pubkey passed in; will be created by this instruction)
    /// - system_program
    /// - rent
    pub fn initialize(
        ctx: Context<InitializeCtx>,
        amount_in_lamports: u64,
        escrow_name: String,
    ) -> Result<()> {
        msg!("Instruction: Initialize");

        // INPUT VALIDATION
        if amount_in_lamports == 0 {
            return err!(EscrowError::ZeroAmount);
        }
        let name_bytes = escrow_name.as_bytes();
        if name_bytes.is_empty() || name_bytes.len() > 32 {
            return err!(EscrowError::InvalidEscrowName);
        }

        // Keys
        let seller_key = ctx.accounts.seller.key();
        let buyer_key = ctx.accounts.buyer.key();

        // Derive PDA with exact seeds [escrow_name, seller, buyer]
        let (pda, bump) =
            Pubkey::find_program_address(&[name_bytes, seller_key.as_ref(), buyer_key.as_ref()], ctx.program_id);

        // Verify provided escrow_info account matches PDA
        if ctx.accounts.escrow_info.key() != pda {
            return err!(EscrowError::InvalidEscrowPDA);
        }

        // Compute space and rent
        let space: usize = EscrowInfo::LEN;
        let rent = &ctx.accounts.rent;
        let lamports_required = rent.minimum_balance(space);

        // Create account at PDA (payer = seller)
        let create_ix = system_instruction::create_account(
            &seller_key,
            &pda,
            lamports_required,
            space as u64,
            ctx.program_id,
        );

        // Build stable signer seeds (avoid temporaries)
        let bump_arr = [bump];
        let signer_seeds: [&[u8]; 4] = [
            name_bytes,
            seller_key.as_ref(),
            buyer_key.as_ref(),
            &bump_arr,
        ];

        invoke_signed(
            &create_ix,
            &[
                ctx.accounts.seller.to_account_info(),
                ctx.accounts.escrow_info.to_account_info(),
                ctx.accounts.system_program.to_account_info(),
            ],
            &[&signer_seeds],
        )?;

        msg!("PDA created. Writing account data (discriminator + struct)");

        // Prepare the struct and directly serialize into the account buffer (no Vec allocation)
        let escrow_struct = EscrowInfo {
            seller: seller_key,
            buyer: buyer_key,
            amount_in_lamports,
            state: State::WaitDeposit,
        };

        // borrow the account data and write discriminator + serialized struct
        let mut data = ctx.accounts.escrow_info.try_borrow_mut_data()?;

        // compute Anchor-style discriminator for "account:EscrowInfo"
        let disc = hash("account:EscrowInfo".as_bytes());
        data[..8].copy_from_slice(&disc.0[..8]);

        // serialize struct directly into the buffer slice after discriminator
        if escrow_struct
            .serialize(&mut &mut data[8..])
            .is_err()
        {
            return err!(EscrowError::InitializationFailed);
        }

        msg!("Initialize: done");
        Ok(())
    }

    /// deposit: buyer transfers the exact escrow.amount_in_lamports into the PDA
    /// Accounts required: buyer (signer, mut), seller (reference), escrow_info (PDA typed, mut), system_program
    pub fn deposit(ctx: Context<DepositCtx>, _escrow_name: String) -> Result<()> {
        msg!("Instruction: Deposit");

        // Read needed values before mutable borrow
        let amount = ctx.accounts.escrow_info.amount_in_lamports;
        let escrow_key = ctx.accounts.escrow_info.key();
        let buyer_key = ctx.accounts.buyer.key();

        // Validate matching parties/state using copies (no long borrow)
        if ctx.accounts.escrow_info.state != State::WaitDeposit {
            return err!(EscrowError::InvalidState);
        }
        if ctx.accounts.escrow_info.buyer != buyer_key {
            return err!(EscrowError::Unauthorized);
        }
        if ctx.accounts.escrow_info.seller != ctx.accounts.seller.key() {
            return err!(EscrowError::InvalidEscrowSeller);
        }

        // Transfer lamports buyer -> PDA (buyer is signer)
        let ix = system_instruction::transfer(&buyer_key, &escrow_key, amount);
        invoke(
            &ix,
            &[
                ctx.accounts.buyer.to_account_info(),
                ctx.accounts.escrow_info.to_account_info(),
                ctx.accounts.system_program.to_account_info(),
            ],
        )?;

        // Now update state (take a mutable borrow)
        let escrow = &mut ctx.accounts.escrow_info;
        escrow.state = State::WaitRecipient;

        msg!("Deposit: done");
        Ok(())
    }

    /// pay: buyer authorizes transfer of the entire PDA balance to seller by closing PDA to seller
    /// Accounts: buyer (signer), seller (mut), escrow_info (mut, close = seller)
    pub fn pay(ctx: Context<PayCtx>, _escrow_name: String) -> Result<()> {
        msg!("Instruction: Pay");

        // Read required checks without holding mutable borrow
        let escrow_buyer = ctx.accounts.escrow_info.buyer;
        let escrow_seller = ctx.accounts.escrow_info.seller;
        let escrow_state = ctx.accounts.escrow_info.state.clone();

        if escrow_state != State::WaitRecipient {
            return err!(EscrowError::InvalidState);
        }
        if escrow_buyer != ctx.accounts.buyer.key() {
            return err!(EscrowError::Unauthorized);
        }
        if escrow_seller != ctx.accounts.seller.key() {
            return err!(EscrowError::InvalidEscrowSeller);
        }

        // Mutate state — Anchor will close the account to seller at the end (close = seller)
        let escrow = &mut ctx.accounts.escrow_info;
        escrow.state = State::Closed;

        msg!("Pay: done (account will be closed to seller)");
        Ok(())
    }

    /// refund: seller refunds buyer the deposited amount (then close PDA to seller to return rent)
    /// Accounts: seller (signer), buyer (mut), escrow_info (mut, close = seller), system_program
    pub fn refund(ctx: Context<RefundCtx>, escrow_name: String) -> Result<()> {
        msg!("Instruction: Refund");

        // read required values before mutable borrow
        let amount = ctx.accounts.escrow_info.amount_in_lamports;
        let escrow_info_key = ctx.accounts.escrow_info.key();

        // validate state & participants (reading copies)
        if ctx.accounts.escrow_info.state != State::WaitRecipient {
            return err!(EscrowError::InvalidState);
        }
        if ctx.accounts.escrow_info.seller != ctx.accounts.seller.key() {
            return err!(EscrowError::Unauthorized);
        }
        if ctx.accounts.escrow_info.buyer != ctx.accounts.buyer.key() {
            return err!(EscrowError::InvalidEscrowBuyer);
        }

        // Validate PDA against seeds (defensive)
        let seller_key = ctx.accounts.seller.key();
        let buyer_key = ctx.accounts.buyer.key();
        let name_bytes = escrow_name.as_bytes();

        let (pda, bump) = Pubkey::find_program_address(&[name_bytes, seller_key.as_ref(), buyer_key.as_ref()], ctx.program_id);
        if pda != escrow_info_key {
            return err!(EscrowError::InvalidEscrowPDA);
        }

        // Build signer seeds
        let bump_arr = [bump];
        let signer_seeds: [&[u8]; 4] = [
            name_bytes,
            seller_key.as_ref(),
            buyer_key.as_ref(),
            &bump_arr,
        ];

        // Transfer only the recorded deposit amount from PDA -> buyer using PDA as signer
        let ix = system_instruction::transfer(&escrow_info_key, &buyer_key, amount);
        invoke_signed(
            &ix,
            &[
                ctx.accounts.escrow_info.to_account_info(),
                ctx.accounts.buyer.to_account_info(),
                ctx.accounts.system_program.to_account_info(),
            ],
            &[&signer_seeds],
        )?;

        // Now update state; account will be closed to seller automatically at instruction end.
        let escrow = &mut ctx.accounts.escrow_info;
        escrow.state = State::Closed;

        msg!("Refund: done (deposit returned; account will be closed to seller)");
        Ok(())
    }
}

/* -------------------------
   Account Contexts
   ------------------------- */

#[derive(Accounts)]
#[instruction(escrow_name: String)]
pub struct Initialize<'info> {
    #[account(init, payer = seller, space = 8 + 32 + 32 + 8 + 1)]
    pub escrow_info: Account<'info, EscrowInfo>,
    
    /// CHECK: This account is only used to pay for initialization, no data is read or written.
    #[account(mut)]
    pub seller: UncheckedAccount<'info>,
    
    /// CHECK: This account is only stored as a Pubkey reference in EscrowInfo.
    pub buyer: UncheckedAccount<'info>,
    
    pub system_program: Program<'info, System>,
}

#[derive(Accounts)]
#[instruction(escrow_name: String)]
pub struct DepositCtx<'info> {
    /// Buyer must sign and be mutable (payer)
    #[account(mut, signer)]
    pub buyer: Signer<'info>,

    /// CHECK: Seller reference used only for PDA derivation + validation
    pub seller: UncheckedAccount<'info>,

    /// PDA typed account; validated and mutable
    #[account(
        mut,
        seeds = [escrow_name.as_bytes(), seller.key().as_ref(), buyer.key().as_ref()],
        bump
    )]
    pub escrow_info: Account<'info, EscrowInfo>,

    pub system_program: Program<'info, System>,
}

#[derive(Accounts)]
#[instruction(escrow_name: String)]
pub struct PayCtx<'info> {
    /// Buyer must sign
    #[account(signer)]
    pub buyer: Signer<'info>,

    /// Seller receives lamports when PDA closed
    #[account(mut)]
    pub seller: AccountInfo<'info>,

    /// PDA account to be closed to seller; seeds validated
    #[account(
        mut,
        seeds = [escrow_name.as_bytes(), seller.key().as_ref(), buyer.key().as_ref()],
        bump,
        close = seller
    )]
    pub escrow_info: Account<'info, EscrowInfo>,
}

#[derive(Accounts)]
#[instruction(escrow_name: String)]
pub struct RefundCtx<'info> {
    /// Seller must sign to authorize refund
    #[account(signer)]
    pub seller: Signer<'info>,

    /// Buyer receives refund lamports (mutable)
    #[account(mut)]
    pub buyer: AccountInfo<'info>,

    /// PDA to refund from and then close to seller
    #[account(
        mut,
        seeds = [escrow_name.as_bytes(), seller.key().as_ref(), buyer.key().as_ref()],
        bump,
        close = seller
    )]
    pub escrow_info: Account<'info, EscrowInfo>,

    pub system_program: Program<'info, System>,
}

/* -------------------------
   State & Account struct
   ------------------------- */

/// Escrow account storing seller, buyer, expected amount, and state.
/// We implement Borsh serialization so we can write directly into the account buffer.
/// Escrow account storing seller, buyer, expected amount, and state.
#[account]
pub struct EscrowInfo {
    /// CHECK: This is a raw pubkey stored for reference only.
    pub seller: Pubkey,
    /// CHECK: This is a raw pubkey stored for reference only.
    pub buyer: Pubkey,
    pub amount_in_lamports: u64,
    pub state: State,
}

impl EscrowInfo {
    /// Anchor discriminator (8) + fields
    pub const LEN: usize = 8 + 32 + 32 + 8 + 1;
}

/// State enum used in EscrowInfo
#[derive(AnchorSerialize, AnchorDeserialize, Clone, PartialEq, Eq)]
pub enum State {
    WaitDeposit,
    WaitRecipient,
    Closed,
}

/* -------------------------
   Errors
   ------------------------- */

#[error_code]
pub enum EscrowError {
    #[msg("Amount must be non-zero.")]
    ZeroAmount,

    #[msg("Invalid escrow state for this operation.")]
    InvalidState,

    #[msg("Unauthorized signer for this action.")]
    Unauthorized,

    #[msg("Escrow seller mismatch.")]
    InvalidEscrowSeller,

    #[msg("Escrow buyer mismatch.")]
    InvalidEscrowBuyer,

    #[msg("Insufficient funds or CPI failed.")]
    TransferFailed,

    #[msg("Escrow name invalid (must be 1..=32 bytes).")]
    InvalidEscrowName,

    #[msg("Provided PDA account does not match derived PDA.")]
    InvalidEscrowPDA,

    #[msg("Failed to initialize PDA account data.")]
    InitializationFailed,
}
