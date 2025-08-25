#![allow(unexpected_cfgs)]
use anchor_lang::prelude::*;
use anchor_lang::system_program;                       // bring in the CPI module
use anchor_lang::solana_program::sysvar::clock::Clock; // Clock via Anchor’s re-export
use anchor_lang::solana_program::system_instruction;
use anchor_lang::solana_program::program::invoke_signed;

declare_id!("28H1btqWd9eRPpQepxHMpRcYyw2nenGWusNpzXbYoxyB");

#[program]
pub mod two_party_betting {
    use super::*;

    pub fn join(ctx: Context<JoinCtx>, delay: u64, wager: u64) -> Result<()> {
        let clock = Clock::get()?;
        require!(wager > 0, BetError::InvalidWager);

        let p1 = &ctx.accounts.participant1.to_account_info();
        let p2 = &ctx.accounts.participant2.to_account_info();

        require!(p1.lamports() >= wager, BetError::InsufficientFunds);
        require!(p2.lamports() >= wager, BetError::InsufficientFunds);

        // Transfer from participant1 → PDA
        {
            let cpi_accounts = system_program::Transfer {
                from: p1.clone(),
                to: ctx.accounts.bet_info.to_account_info(),
            };
            let cpi_ctx = CpiContext::new(
                ctx.accounts.system_program.to_account_info(),
                cpi_accounts,
            );
            system_program::transfer(cpi_ctx, wager)?;
        }

        // Transfer from participant2 → PDA
        {
            let cpi_accounts = system_program::Transfer {
                from: p2.clone(),
                to: ctx.accounts.bet_info.to_account_info(),
            };
            let cpi_ctx = CpiContext::new(
                ctx.accounts.system_program.to_account_info(),
                cpi_accounts,
            );
            system_program::transfer(cpi_ctx, wager)?;
        }

        let bet = &mut ctx.accounts.bet_info;
        bet.participant1 = ctx.accounts.participant1.key();
        bet.participant2 = ctx.accounts.participant2.key();
        bet.oracle       = ctx.accounts.oracle.key();
        bet.wager        = wager;
        bet.deadline     = clock.slot.checked_add(delay).unwrap();
        bet.state        = BetState::Active;
        bet.bump         = ctx.bumps.bet_info;

        Ok(())
    }

    pub fn win(ctx: Context<WinCtx>) -> Result<()> {
        let clock = Clock::get()?;
        let bet_info_account = ctx.accounts.bet_info.to_account_info();

        msg!("Raw bet_info data: {:?}", ctx.accounts.bet_info.to_account_info().data.borrow());
        // Manually deserialize BetInfo from account data
        let mut bet_data = BetInfo::try_from_slice(
            &ctx.accounts.bet_info.to_account_info().data.borrow()
        ).map_err(|_| BetError::InvalidBetAccount)?;


        // Validation checks
        require!(bet_data.state == BetState::Active, BetError::AlreadyResolved);
        require!(ctx.accounts.oracle.key() == bet_data.oracle, BetError::UnauthorizedOracle);
        require!(clock.slot <= bet_data.deadline, BetError::DeadlinePassed);

        let winner_key = ctx.accounts.winner.key();
        require!(
            winner_key == bet_data.participant1 || winner_key == bet_data.participant2,
            BetError::InvalidWinner
        );

        // Calculate pot
        let pot = bet_data.wager.checked_mul(2).ok_or(BetError::ArithmeticOverflow)?;

        // Prepare transfer instruction
        let ix = system_instruction::transfer(
            &bet_info_account.key(),
            &ctx.accounts.winner.key(),
            pot,
        );

        let seeds = &[
            bet_data.participant1.as_ref(),
            bet_data.participant2.as_ref(),
            &[bet_data.bump],
        ];
        let signer = &[&seeds[..]];

        // Perform transfer using PDA signer
        invoke_signed(
            &ix,
            &[
                bet_info_account.clone(),
                ctx.accounts.winner.to_account_info(),
                ctx.accounts.system_program.to_account_info(),
            ],
            signer,
        )?;
        // Update state manually
        bet_data.state = BetState::Resolved;

        // Serialize updated BetInfo back into account
        bet_data.serialize(&mut &mut bet_info_account.data.borrow_mut()[..])?;

        Ok(())
    }

    pub fn timeout(ctx: Context<TimeoutCtx>) -> Result<()> {
        let clock = Clock::get()?;
        let bet   = &mut ctx.accounts.bet_info;

        require!(bet.state == BetState::Active, BetError::AlreadyResolved);
        require!(clock.slot > bet.deadline, BetError::DeadlineNotReached);

        let p1_signed = ctx.accounts.participant1.is_signer;
        let p2_signed = ctx.accounts.participant2.is_signer;
        require!(p1_signed || p2_signed, BetError::UnauthorizedParticipant);

        let wager = bet.wager;
        let seeds = &[
            bet.participant1.as_ref(),
            bet.participant2.as_ref(),
            &[bet.bump],
        ];
        let signer = &[&seeds[..]];

        // Refund participant1
        {
            let cpi_accounts = system_program::Transfer {
                from: bet.to_account_info(),
                to: ctx.accounts.participant1.to_account_info(),
            };
            let cpi_ctx = CpiContext::new_with_signer(
                ctx.accounts.system_program.to_account_info(),
                cpi_accounts,
                signer,
            );
            system_program::transfer(cpi_ctx, wager)?;
        }

        // Refund participant2
        {
            let cpi_accounts = system_program::Transfer {
                from: bet.to_account_info(),
                to: ctx.accounts.participant2.to_account_info(),
            };
            let cpi_ctx = CpiContext::new_with_signer(
                ctx.accounts.system_program.to_account_info(),
                cpi_accounts,
                signer,
            );
            system_program::transfer(cpi_ctx, wager)?;
        }

        bet.state = BetState::Resolved;
        Ok(())
    }
}

// ... rest of your account structs, state, and error definitions remain unchanged


/// Accounts for `join` — both parties must sign in the same tx.
#[derive(Accounts)]
#[instruction(delay: u64, wager: u64)]
pub struct JoinCtx<'info> {
    #[account(mut)]
    pub participant1: Signer<'info>,

    #[account(mut)]
    pub participant2: Signer<'info>,

    /// CHECK: stored for later validation; does not sign
    pub oracle: AccountInfo<'info>,

    #[account(
        init,
        payer = participant1,
        seeds = [participant1.key().as_ref(), participant2.key().as_ref()],
        bump,
        space = 8 + BetInfo::LEN
    )]
    pub bet_info: Account<'info, BetInfo>,

    pub system_program: Program<'info, System>,
}



/// Accounts for `win` — only the oracle signs.
#[derive(Accounts)]
pub struct WinCtx<'info> {
    /// CHECK:
    #[account(mut, signer)]
    pub oracle: Signer<'info>,

    /// CHECK:
    #[account(mut)]
    pub winner: UncheckedAccount<'info>,

    #[account(
        mut,
        seeds = [participant1.key().as_ref(), participant2.key().as_ref()],
        bump = bet_info.bump,
        has_one = participant1,
        has_one = participant2
    )]
    pub bet_info: Account<'info, BetInfo>,


    /// CHECK: used only for PDA derivation & has_one check
    pub participant1: UncheckedAccount<'info>,

    /// CHECK: used only for PDA derivation & has_one check
    pub participant2: UncheckedAccount<'info>,

    pub system_program: Program<'info, System>,
}


/// Accounts for `timeout` — either participant may sign after deadline.
#[derive(Accounts)]
pub struct TimeoutCtx<'info> {
    /// CHECK:
    #[account(mut)]
    pub participant1: UncheckedAccount<'info>,
    /// CHECK:
    #[account(mut)]
    pub participant2: UncheckedAccount<'info>,
    
    /// CHECK:
    #[account(
        mut,
        seeds = [participant1.key().as_ref(), participant2.key().as_ref()],
        bump = bet_info.bump,
        has_one = participant1,
        has_one = participant2
    )]
    pub bet_info: Account<'info, BetInfo>,

    pub system_program: Program<'info, System>,
}

/// On‐chain state for each bet.
#[account]
pub struct BetInfo {
    pub participant1: Pubkey,
    pub participant2: Pubkey,
    pub oracle:       Pubkey,
    pub wager:        u64,
    pub deadline:     u64,
    pub state:        BetState,
    pub bump:         u8,
}

impl BetInfo {
    // 32*3 bytes for pubkeys + 8*2 for u64s + 1 for enum + 1 for bump
    pub const LEN: usize = 32 * 3 + 8 * 2 + 1 + 1;
}

/// Two states: Active until resolved, then never re‐used.
#[derive(AnchorSerialize, AnchorDeserialize, PartialEq, Eq, Clone)]
pub enum BetState {
    Active,
    Resolved,
}

/// Custom errors for every failure case.
#[error_code]
pub enum BetError {
    #[msg("Wager must be greater than zero")]
    InvalidWager,
    #[msg("Insufficient funds to place wager")]
    InsufficientFunds,
    #[msg("Attempted arithmetic overflow")]
    ArithmeticOverflow,
    #[msg("Bet has already been resolved")]
    AlreadyResolved,
    #[msg("Caller is not the designated oracle")]
    UnauthorizedOracle,
    #[msg("Caller is not one of the participants")]
    UnauthorizedParticipant,
    #[msg("Deadline has not yet been reached")]
    DeadlineNotReached,
    #[msg("Deadline has already passed")]
    DeadlinePassed,
    #[msg("Winner must be participant1 or participant2")]
    InvalidWinner,
    #[msg("BetInfo account is not properly initialized or corrupted")]
    InvalidBetAccount,
}

