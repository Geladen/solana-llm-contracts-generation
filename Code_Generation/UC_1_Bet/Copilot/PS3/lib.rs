use anchor_lang::prelude::*;
use anchor_lang::system_program;

declare_id!("4X2nX158G3dXj21jCuWRMCjpFhwXUZpLmP4C8PRyADr2");

#[program]
pub mod solana_betting {
    use super::*;

    /// Both participants must sign this in one transaction.
    /// Each transfers `wager` lamports into the shared PDA.
    /// PDA stores oracle, participants, wager, deadline.
    pub fn join(
        ctx: Context<JoinCtx>,
        delay: u64,
        wager: u64,
    ) -> Result<()> {
        require!(wager > 0, ErrorCode::InvalidWager);
        let clock = Clock::get()?;

        // Initialize state
        let bet = &mut ctx.accounts.bet_info;
        bet.oracle = ctx.accounts.oracle.key();
        bet.participant1 = ctx.accounts.participant1.key();
        bet.participant2 = ctx.accounts.participant2.key();
        bet.wager = wager;
        bet.deadline = clock.slot.checked_add(delay)
            .ok_or(ErrorCode::CalculationOverflow)?;

        // Transfer wagers into PDA
        // participant1 pays rent + their wager via `init` payer
        let cpi_p1 = CpiContext::new(
            ctx.accounts.system_program.to_account_info(),
            system_program::Transfer {
                from: ctx.accounts.participant1.to_account_info(),
                to: ctx.accounts.bet_info.to_account_info(),
            },
        );
        system_program::transfer(cpi_p1, wager)?;

        let cpi_p2 = CpiContext::new(
            ctx.accounts.system_program.to_account_info(),
            system_program::Transfer {
                from: ctx.accounts.participant2.to_account_info(),
                to: ctx.accounts.bet_info.to_account_info(),
            },
        );
        system_program::transfer(cpi_p2, wager)?;

        Ok(())
    }

    /// Only the designated oracle may call.
    /// Transfers the full pot (2 * wager) to `winner`.
    pub fn win(ctx: Context<WinCtx>) -> Result<()> {
    let bet = &ctx.accounts.bet_info;

    // 1) Only allow the stored oracle to call
    //    (you already do this via the constraint on WinCtx)

    // 2) Ensure the `winner` is either participant1 or participant2
    require!(
        ctx.accounts.winner.key() == bet.participant1
         || ctx.accounts.winner.key() == bet.participant2,
        ErrorCode::InvalidWinner
    );

    // 3) Re‐derive PDA and bump
    let (pda, bump) = Pubkey::find_program_address(
        &[
            bet.participant1.as_ref(),
            bet.participant2.as_ref(),
        ],
        ctx.program_id,
    );
    require_keys_eq!(pda, ctx.accounts.bet_info.key(), ErrorCode::InvalidPda);

    // 4) Compute full pot
    let pot = bet.wager.checked_mul(2)
        .ok_or(ErrorCode::CalculationOverflow)?;

    // 5) Move lamports out of PDA into `winner`
    let bet_info_ai = &mut ctx.accounts.bet_info.to_account_info();
    let winner_ai   = &mut ctx.accounts.winner;
    **bet_info_ai.lamports.borrow_mut() -= pot;
    **winner_ai.lamports.borrow_mut()   += pot;

    Ok(())
}


    /// After `deadline`, either participant may call.
    /// Returns each their original wager.
    pub fn timeout(ctx: Context<TimeoutCtx>) -> Result<()> {
        let clock = Clock::get()?;
        let bet   = &ctx.accounts.bet_info;

        // 1) Enforce deadline
        require!(clock.slot > bet.deadline, ErrorCode::DeadlineNotReached);

        // 2) Only one of the participants may call
        require!(
            ctx.accounts.participant1.is_signer
            || ctx.accounts.participant2.is_signer,
            ErrorCode::UnauthorizedParticipant
        );

        // 3) Re-derive bump and verify PDA
        let (pda, _bump) = Pubkey::find_program_address(
            &[
                bet.participant1.as_ref(),
                bet.participant2.as_ref(),
            ],
            ctx.program_id,
        );
        require_keys_eq!(pda, ctx.accounts.bet_info.key(), ErrorCode::InvalidPda);

        // 4) Pull out AccountInfos
        let bet_info_ai = &mut ctx.accounts.bet_info.to_account_info();
        let p1_ai       = &mut ctx.accounts.participant1;
        let p2_ai       = &mut ctx.accounts.participant2;

        // 5) Refund each participant
        let refund = bet.wager;
        **bet_info_ai.lamports.borrow_mut() -= refund;
        **p1_ai.lamports.borrow_mut()       += refund;

        **bet_info_ai.lamports.borrow_mut() -= refund;
        **p2_ai.lamports.borrow_mut()       += refund;

        Ok(())
    }
}

/// Shared on‐chain state for each bet
#[account]
pub struct BetInfo {
    pub oracle:      Pubkey,
    pub participant1: Pubkey,
    pub participant2: Pubkey,
    pub wager:       u64,
    pub deadline:    u64,
}

/// Context for `join`
#[derive(Accounts)]
#[instruction(delay: u64, wager: u64)]
pub struct JoinCtx<'info> {
    /// The first bettor, must sign and fund the PDA
    #[account(mut, signer)]
    pub participant1: Signer<'info>,

    /// The second bettor, must sign and fund the PDA
    #[account(mut, signer)]
    pub participant2: Signer<'info>,

    /// CHECK: only used to capture & store the oracle pubkey in BetInfo
    pub oracle: UncheckedAccount<'info>,

    /// PDA to store state & hold the lamports
    #[account(
        init,
        payer = participant1,
        seeds = [
            participant1.key().as_ref(),
            participant2.key().as_ref()
        ],
        bump,
        space = 8 + 32*3 + 8 + 8
    )]
    pub bet_info: Account<'info, BetInfo>,

    /// System program for transfers
    pub system_program: Program<'info, System>,
}

#[derive(Accounts)]
pub struct WinCtx<'info> {
    /// CHECK: must match `BetInfo.oracle`, not read or written
    #[account(signer)]
    pub oracle: AccountInfo<'info>,

    /// CHECK: recipient of the pot, not read or written
    #[account(mut)]
    pub winner: AccountInfo<'info>,

    /// PDA holding the funds & metadata
    #[account(
        mut,
        seeds = [
            participant1.key().as_ref(),
            participant2.key().as_ref()
        ],
        bump,
        constraint = oracle.key() == bet_info.oracle @ ErrorCode::UnauthorizedOracle
    )]
    pub bet_info: Account<'info, BetInfo>,

    /// CHECK: used only for PDA derivation
    pub participant1: UncheckedAccount<'info>,

    /// CHECK: used only for PDA derivation
    pub participant2: UncheckedAccount<'info>,

    /// System program for transfers
    pub system_program: Program<'info, System>,
}

#[derive(Accounts)]
pub struct TimeoutCtx<'info> {
    /// CHECK: signer for refund, no data read/written
    #[account(mut)]
    pub participant1: AccountInfo<'info>,

    /// CHECK: signer for refund, no data read/written
    #[account(mut)]
    pub participant2: AccountInfo<'info>,

    /// PDA holding the funds & metadata
    #[account(
        mut,
        seeds = [
            participant1.key().as_ref(),
            participant2.key().as_ref()
        ],
        bump
    )]
    pub bet_info: Account<'info, BetInfo>,

    /// System program for transfers
    pub system_program: Program<'info, System>,
}

/// Custom errors
#[error_code]
pub enum ErrorCode {
    #[msg("Wager must be greater than zero")]
    InvalidWager,
    #[msg("Calculation overflow occurred")]
    CalculationOverflow,
    #[msg("Only the designated oracle may call this")]
    UnauthorizedOracle,
    #[msg("Deadline has not yet been reached")]
    DeadlineNotReached,
    #[msg("Only participant1 or participant2 may call this")]
    UnauthorizedParticipant,
    #[msg("PDA derivation mismatch")]
    InvalidPda,
    #[msg("Winner must be one of the participants")]
    InvalidWinner,
}

