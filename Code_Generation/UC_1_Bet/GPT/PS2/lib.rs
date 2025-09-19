use anchor_lang::prelude::*;
use anchor_lang::system_program;

declare_id!("DnozfNC6MfEXQgnpYoYtTEpK4oPgfhdwb6UvGaCVHJdH");

#[program]
pub mod betting {
    use super::*;

    /// join: both participants must sign this single transaction.
    /// - delay: number of slots to add to current slot to form deadline
    /// - wager: lamports each participant will deposit
    pub fn join(ctx: Context<JoinCtx>, delay: u64, wager: u64) -> Result<()> {
    require!(wager > 0, ErrorCode::InvalidWager);

    let bet_info = &mut ctx.accounts.bet_info;

    // store participants, oracle, wager, deadline
    bet_info.participant1 = ctx.accounts.participant1.key();
    bet_info.participant2 = ctx.accounts.participant2.key();
    bet_info.oracle = ctx.accounts.oracle.key();
    bet_info.wager = wager;

    let clock = Clock::get()?;
    bet_info.deadline = clock
        .slot
        .checked_add(delay)
        .ok_or(ErrorCode::NumericOverflow)?;
    bet_info.settled = false;

    // Transfer wagers
    system_program::transfer(
        CpiContext::new(
            ctx.accounts.system_program.to_account_info(),
            system_program::Transfer {
                from: ctx.accounts.participant1.to_account_info(),
                to: ctx.accounts.bet_info.to_account_info(),
            },
        ),
        wager,
    )?;
    system_program::transfer(
        CpiContext::new(
            ctx.accounts.system_program.to_account_info(),
            system_program::Transfer {
                from: ctx.accounts.participant2.to_account_info(),
                to: ctx.accounts.bet_info.to_account_info(),
            },
        ),
        wager,
    )?;

    Ok(())
}



    /// win: only callable by the oracle signer.
    /// Transfers the entire pot to `winner` by closing the bet_info PDA to `winner`.
    pub fn win(ctx: Context<WinCtx>) -> Result<()> {
        // Only the pre-designated oracle may call
        require!(
            ctx.accounts.oracle.key() == ctx.accounts.bet_info.oracle,
            ErrorCode::Unauthorized
        );

        // Bet must not already be settled
        require!(!ctx.accounts.bet_info.settled, ErrorCode::AlreadySettled);

        // Winner must be one of the two participants
        let winner_key = ctx.accounts.winner.key();
        let p1 = ctx.accounts.participant1.key();
        let p2 = ctx.accounts.participant2.key();
        require!(
            winner_key == p1 || winner_key == p2,
            ErrorCode::InvalidWinner
        );

        // Mark settled (optional since account will be closed by Anchor after handler)
        ctx.accounts.bet_info.settled = true;

        // `close = winner` attribute on bet_info will send all remaining lamports to winner
        Ok(())
    }

    /// timeout: callable by either participant after deadline.
    /// Returns each participant their original wager and closes the PDA.
pub fn timeout(ctx: Context<TimeoutCtx>) -> Result<()> {
    // Extract immutable AccountInfos first
    let bet_info_account = ctx.accounts.bet_info.to_account_info();
    let participant1_account = ctx.accounts.participant1.to_account_info();
    let participant2_account = ctx.accounts.participant2.to_account_info();

    // PDA seeds for signing
    let seeds: &[&[u8]] = &[
        ctx.accounts.participant1.key.as_ref(),
        ctx.accounts.participant2.key.as_ref(),
    ];
    let signer_seeds: &[&[&[u8]]] = &[seeds];

    let bet_info = &mut ctx.accounts.bet_info;

    // validations
    require!(
        ctx.accounts.participant1.key() == bet_info.participant1 &&
        ctx.accounts.participant2.key() == bet_info.participant2,
        ErrorCode::InvalidParticipants
    );
    let clock = Clock::get()?;
    require!(clock.slot > bet_info.deadline, ErrorCode::DeadlineNotReached);
    require!(!bet_info.settled, ErrorCode::AlreadySettled);

    // Refund wagers
    system_program::transfer(
        CpiContext::new_with_signer(
            ctx.accounts.system_program.to_account_info(),
            system_program::Transfer {
                from: bet_info_account.clone(),
                to: participant1_account,
            },
            signer_seeds,
        ),
        bet_info.wager,
    )?;
    system_program::transfer(
        CpiContext::new_with_signer(
            ctx.accounts.system_program.to_account_info(),
            system_program::Transfer {
                from: bet_info_account,
                to: participant2_account,
            },
            signer_seeds,
        ),
        bet_info.wager,
    )?;

    bet_info.settled = true;
    Ok(())
}



}

/// ACCOUNTS: strictly the accounts you asked for per instruction
#[derive(Accounts)]
#[instruction(delay: u64, wager: u64)]
pub struct JoinCtx<'info> {
    /// Participant 1 must be signer and mutable
    #[account(mut)]
    pub participant1: Signer<'info>,

    /// Participant 2 must be signer and mutable
    #[account(mut)]
    pub participant2: Signer<'info>,

    /// CHECK: This is the oracle account, stored in BetInfo for reference only.
    pub oracle: UncheckedAccount<'info>,

    /// PDA account to hold bet state and lamports.
    /// Seeds exactly: [participant1.key().as_ref(), participant2.key().as_ref()]
    #[account(
        init,
        seeds = [participant1.key().as_ref(), participant2.key().as_ref()],
        bump,
        payer = participant1,
        space = BetInfo::LEN
    )]
    pub bet_info: Account<'info, BetInfo>,

    pub system_program: Program<'info, System>,
}

#[derive(Accounts)]
pub struct WinCtx<'info> {
    /// Oracle must sign
    pub oracle: Signer<'info>,

    /// Winner account (mutable) - will receive full pot when bet_info is closed
    /// CHECK: The winner account must be one of the participants. Validated in the instruction.
    #[account(mut)]
    pub winner: UncheckedAccount<'info>,

    /// Bet info PDA (mutable). Close to winner after payout.
    #[account(
        mut,
        seeds = [participant1.key().as_ref(), participant2.key().as_ref()],
        bump,
        close = winner
    )]
    pub bet_info: Account<'info, BetInfo>,

    /// Participant1 (used for PDA derivation)
    /// CHECK: Used only for PDA seed derivation
    pub participant1: UncheckedAccount<'info>,

    /// Participant2 (used for PDA derivation)
    /// CHECK: Used only for PDA seed derivation
    pub participant2: UncheckedAccount<'info>,

    pub system_program: Program<'info, System>,
}


#[derive(Accounts)]
pub struct TimeoutCtx<'info> {
    /// CHECK: Participant1 mutable, must sign or participate
    #[account(mut)]
    pub participant1: UncheckedAccount<'info>,

    /// CHECK: Participant2 mutable, must sign or participate
    #[account(mut)]
    pub participant2: UncheckedAccount<'info>,

    /// bet_info PDA
    #[account(
        mut,
        seeds = [participant1.key().as_ref(), participant2.key().as_ref()],
        bump,
        close = participant1
    )]
    pub bet_info: Account<'info, BetInfo>,

    pub system_program: Program<'info, System>,
}


/// State stored in the PDA
#[account]
pub struct BetInfo {
    pub participant1: Pubkey,
    pub participant2: Pubkey,
    pub oracle: Pubkey,
    pub wager: u64,     // per-participant
    pub deadline: u64,  // slot number
    pub settled: bool,
}

impl BetInfo {
    // size: discriminator(8) + 3*32 (pubkeys) + 2*8 (u64s) + 1 (bool) + 1 (bump) + padding
    pub const LEN: usize = 8 + 32 * 3 + 8 * 2 + 1 + 1 + 6;
}

#[error_code]
pub enum ErrorCode {
    #[msg("Wager must be > 0")]
    InvalidWager,
    #[msg("Unauthorized: signer not permitted")]
    Unauthorized,
    #[msg("Deadline has not been reached yet")]
    DeadlineNotReached,
    #[msg("Bet already settled")]
    AlreadySettled,
    #[msg("Provided participant accounts do not match PDA's participants")]
    InvalidParticipants,
    #[msg("Winner must be one of the two participants")]
    InvalidWinner,
    #[msg("Numeric overflow")]
    NumericOverflow,
}

