use anchor_lang::prelude::*;
use anchor_lang::solana_program::system_program;
use anchor_lang::system_program::Transfer;

declare_id!("6qvR3ezKg21cJGoRyphGyKttgo1feQwihV7MKTNb9rDo");

#[program]
pub mod bet_program {
    use super::*;

    /// Both participants call this in the same transaction to place equal wagers.
    pub fn join(
    ctx: Context<JoinCtx>,
    delay: u64,
    wager: u64,
) -> Result<()> {
    let bet = &mut ctx.accounts.bet_info;
    bet.participant1 = *ctx.accounts.participant1.key;
    bet.participant2 = *ctx.accounts.participant2.key;
    bet.oracle       = *ctx.accounts.oracle.key;
    let clock        = Clock::get()?;
    bet.deadline     = clock.slot.checked_add(delay).unwrap();
    bet.wager        = wager;
    // ← direct field lookup instead of `ctx.bumps.get()`
    bet.bump         = ctx.bumps.bet_info;

    // transfer from participant1
    anchor_lang::system_program::transfer(
        CpiContext::new(
            ctx.accounts.system_program.to_account_info(),
            Transfer {
                from: ctx.accounts.participant1.to_account_info(),
                to:   ctx.accounts.bet_info.to_account_info(),
            },
        ),
        wager,
    )?;

    // transfer from participant2
    anchor_lang::system_program::transfer(
        CpiContext::new(
            ctx.accounts.system_program.to_account_info(),
            Transfer {
                from: ctx.accounts.participant2.to_account_info(),
                to:   ctx.accounts.bet_info.to_account_info(),
            },
        ),
        wager,
    )?;

    Ok(())
}

    /// Oracle declares the winner before the deadline.
    pub fn win(ctx: Context<WinCtx>) -> Result<()> {
    let mut bet_info_ai     = ctx.accounts.bet_info.to_account_info();
    let mut winner_ai       = ctx.accounts.winner.to_account_info();

    // now that we have the AIs, borrow the state mutably
    let bet = &mut ctx.accounts.bet_info;

    require!(bet.wager > 0, ErrorCode::AlreadySettled);
    let clock = Clock::get()?;
    require!(clock.slot <= bet.deadline, ErrorCode::TooLateForWin);

    let wkey = ctx.accounts.winner.key();
    require!(
        wkey == bet.participant1 || wkey == bet.participant2,
        ErrorCode::InvalidWinner
    );

    let pot = bet.wager.checked_mul(2).unwrap();

    // manual lamport move
    **bet_info_ai.try_borrow_mut_lamports()? -= pot;
    **winner_ai.try_borrow_mut_lamports()?  += pot;

    bet.wager = 0;
    Ok(())
}

    /// After deadline, either participant can refund their wager.
    pub fn timeout(ctx: Context<TimeoutCtx>) -> Result<()> {
    let mut bet_info_ai  = ctx.accounts.bet_info.to_account_info();
    let mut p1_ai        = ctx.accounts.participant1.to_account_info();
    let mut p2_ai        = ctx.accounts.participant2.to_account_info();

    let bet = &mut ctx.accounts.bet_info;
    require!(bet.wager > 0, ErrorCode::AlreadySettled);

    let clock = Clock::get()?;
    require!(clock.slot > bet.deadline, ErrorCode::TooEarlyForTimeout);

    require!(
        ctx.accounts.participant1.is_signer
        || ctx.accounts.participant2.is_signer,
        ErrorCode::Unauthorized
    );

    let w = bet.wager;

    // refund participant1
    **bet_info_ai.try_borrow_mut_lamports()? -= w;
    **p1_ai.try_borrow_mut_lamports()?       += w;

    // refund participant2
    **bet_info_ai.try_borrow_mut_lamports()? -= w;
    **p2_ai.try_borrow_mut_lamports()?       += w;

    bet.wager = 0;
    Ok(())
}
}

#[derive(Accounts)]
#[instruction(delay: u64, wager: u64)]
pub struct JoinCtx<'info> {
    /// Each participant must sign and the lamports are debited from them.
    #[account(mut)]
    pub participant1: Signer<'info>,

    #[account(mut)]
    pub participant2: Signer<'info>,

    /// CHECK: oracle’s pubkey is only stored in state; we do not read or write its data.
    pub oracle: UncheckedAccount<'info>,

    #[account(
        init,
        payer = participant1,
        space = 8 + 32 + 32 + 32 + 8 + 8 + 1,
        seeds = [
            participant1.key().as_ref(),
            participant2.key().as_ref()
        ],
        bump
    )]
    pub bet_info: Account<'info, BetInfo>,

    pub system_program: Program<'info, System>,
}

#[derive(Accounts)]
pub struct WinCtx<'info> {
    /// The oracle must sign to call `win` and is checked against `bet_info.oracle`.
    #[account(signer)]
    /// CHECK: we simply verify `oracle.key() == bet_info.oracle`; no further deserialization needed.
    pub oracle: AccountInfo<'info>,

    /// The account that will receive the pot—no signature required.
    #[account(mut)]
    /// CHECK: we only transfer lamports into this account.
    pub winner: AccountInfo<'info>,

    #[account(
        mut,
        seeds = [
            participant1.key().as_ref(),
            participant2.key().as_ref()
        ],
        bump = bet_info.bump,
        has_one = participant1,
        has_one = participant2,
        constraint = bet_info.oracle == oracle.key() @ ErrorCode::UnauthorizedOracle
    )]
    pub bet_info: Account<'info, BetInfo>,

    /// CHECK: used only for PDA derivation; we never read or write its data.
    pub participant1: UncheckedAccount<'info>,

    /// CHECK: used only for PDA derivation; we never read or write its data.
    pub participant2: UncheckedAccount<'info>,

    pub system_program: Program<'info, System>,
}

#[derive(Accounts)]
pub struct TimeoutCtx<'info> {
    /// One of these must sign; we enforce it manually in the handler.
    #[account(mut)]
    /// CHECK: signer‐status is checked at runtime; no other data access needed.
    pub participant1: AccountInfo<'info>,

    #[account(mut)]
    /// CHECK: signer‐status is checked at runtime; no other data access needed.
    pub participant2: AccountInfo<'info>,

    #[account(
        mut,
        seeds = [
            participant1.key().as_ref(),
            participant2.key().as_ref()
        ],
        bump = bet_info.bump
    )]
    pub bet_info: Account<'info, BetInfo>,

    pub system_program: Program<'info, System>,
}

#[account]
pub struct BetInfo {
    pub participant1: Pubkey,
    pub participant2: Pubkey,
    pub oracle:       Pubkey,
    pub deadline:     u64,
    pub wager:        u64,
    pub bump:         u8,
}

#[error_code]
pub enum ErrorCode {
    #[msg("Bet has already been settled.")]
    AlreadySettled,

    #[msg("Too late to call win after deadline.")]
    TooLateForWin,

    #[msg("Too early to call timeout.")]
    TooEarlyForTimeout,

    #[msg("Oracle is not authorized.")]
    UnauthorizedOracle,

    #[msg("Winner must be one of the two participants.")]
    InvalidWinner,

    #[msg("Caller is not a participant.")]
    Unauthorized,
}

