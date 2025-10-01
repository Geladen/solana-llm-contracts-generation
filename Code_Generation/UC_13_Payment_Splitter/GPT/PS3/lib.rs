// Standard Anchor imports
use anchor_lang::prelude::*;


declare_id!("5vzYCTK92hzxcTLcrniASAVBtsvWcKALBixs98aLrCgF");

#[program]
pub mod playment_gpt {
    use super::*;

    pub fn initialize(
        ctx: Context<Initialize>,
        payees: Vec<Pubkey>,
        shares_amounts: Vec<u64>,
    ) -> Result<()> {
        let ps_info = &mut ctx.accounts.ps_info;

        if payees.is_empty() {
            return err!(ErrorCode::NoPayees);
        }
        if payees.len() != shares_amounts.len() {
            return err!(ErrorCode::PayeesSharesLengthMismatch);
        }
        if shares_amounts.iter().sum::<u64>() == 0 {
            return err!(ErrorCode::InvalidSharesTotal);
        }

        ps_info.current_lamports = **ctx.accounts.ps_info.to_account_info().lamports.borrow();
        ps_info.payees = payees;
        ps_info.shares_amounts = shares_amounts;
        ps_info.released_amounts = vec![0; ps_info.payees.len()];

        Ok(())
    }

pub fn release(ctx: Context<Release>) -> Result<()> {
    // Step 1: read lamports into a temporary variable BEFORE taking a mutable borrow
    let ps_lamports_val = **ctx.accounts.ps_info.to_account_info().lamports.borrow();

    // Step 2: now borrow the account mutably
    let ps_info = &mut ctx.accounts.ps_info;

    // Step 3: assign the lamports value safely
    ps_info.current_lamports = ps_lamports_val;

    Ok(())
}
}

#[derive(Accounts)]
#[instruction(payees: Vec<Pubkey>, shares_amounts: Vec<u64>)]
pub struct Initialize<'info> {
    #[account(mut, signer)]
    pub initializer: AccountInfo<'info>,
    #[account(init, payer = initializer, space = 8 + 1024, seeds = [b"payment_splitter", initializer.key().as_ref()], bump)]
    pub ps_info: Account<'info, PaymentSplitterInfo>,
    pub system_program: Program<'info, System>,
}

#[derive(Accounts)]
pub struct Release<'info> {
    #[account(signer)]
    pub payee: AccountInfo<'info>,
    #[account(mut, seeds = [b"payment_splitter", initializer.key().as_ref()], bump)]
    pub ps_info: Account<'info, PaymentSplitterInfo>,
    /// CHECK: Reference only
    pub initializer: AccountInfo<'info>,
    pub system_program: Program<'info, System>,
}

#[account]
pub struct PaymentSplitterInfo {
    pub current_lamports: u64,
    pub payees: Vec<Pubkey>,
    pub shares_amounts: Vec<u64>,
    pub released_amounts: Vec<u64>,
}

#[error_code]
pub enum ErrorCode {
    #[msg("No payees provided")]
    NoPayees,
    #[msg("Payees and shares length mismatch")]
    PayeesSharesLengthMismatch,
    #[msg("Invalid total shares")]
    InvalidSharesTotal,
    #[msg("Payee not found")]
    PayeeNotFound,
    #[msg("Nothing to release")]
    NothingToRelease,
    #[msg("Arithmetic overflow")]
    ArithmeticOverflow,
}
