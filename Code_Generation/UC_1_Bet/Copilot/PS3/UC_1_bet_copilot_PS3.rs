#![allow(unexpected_cfgs)]
use anchor_lang::prelude::*;
use anchor_lang::system_program;
use anchor_lang::system_program::Transfer;
use anchor_lang::solana_program::sysvar::clock::Clock;

declare_id!("4X2nX158G3dXj21jCuWRMCjpFhwXUZpLmP4C8PRyADr2");

#[program]
pub mod two_party_betting {
    use super::*;

    /// Both participants deposit the same wager into a PDA.
    pub fn join(
    ctx: Context<JoinCtx>,
    delay: u64,
    wager: u64,
) -> Result<()> {
    // ensure participants are distinct
    if ctx.accounts.participant1.key() == ctx.accounts.participant2.key() {
        return err!(ErrorCode::SameParticipant);
    }

    // extract bump via struct field
    let bump = ctx.bumps.bet_info;

    // now you can mutably borrow bet_info
    let bet = &mut ctx.accounts.bet_info;
    bet.bump         = bump;
    bet.participant1 = ctx.accounts.participant1.key();
    bet.participant2 = ctx.accounts.participant2.key();
    bet.oracle       = ctx.accounts.oracle.key();
    bet.wager        = wager;
    bet.deadline     = Clock::get()?
        .slot
        .checked_add(delay)
        .ok_or(ErrorCode::NumericalOverflow)?;
    bet.resolved     = false;

    // transfer wagers
    let cpi1 = Transfer {
        from: ctx.accounts.participant1.to_account_info(),
        to:   ctx.accounts.bet_info.to_account_info(),
    };
    system_program::transfer(
        CpiContext::new(ctx.accounts.system_program.to_account_info(), cpi1),
        wager,
    )?;

    let cpi2 = Transfer {
        from: ctx.accounts.participant2.to_account_info(),
        to:   ctx.accounts.bet_info.to_account_info(),
    };
    system_program::transfer(
        CpiContext::new(ctx.accounts.system_program.to_account_info(), cpi2),
        wager,
    )?;

    Ok(())
}

    pub fn win(ctx: Context<WinCtx>) -> Result<()> {
        let bet = &mut ctx.accounts.bet_info;

        // 1) Only the stored oracle may call
        require!(
            ctx.accounts.oracle.key() == bet.oracle,
            ErrorCode::InvalidOracle
        );

        // 2) Must not already be resolved, and must be before the deadline
        require!(!bet.resolved, ErrorCode::AlreadyResolved);
        let now = Clock::get()?.slot;
        require!(now <= bet.deadline, ErrorCode::DeadlinePassed);

        // 3) Winner must match one of the two stored participants
        let winner_key = ctx.accounts.winner.key();
        require!(
            winner_key == bet.participant1 || winner_key == bet.participant2,
            ErrorCode::InvalidWinner
        );

        // 4) Compute the pot = wager × 2
        let pot = bet
            .wager
            .checked_mul(2)
            .ok_or(ErrorCode::NumericalOverflow)?;

        // 5) Move lamports out of the PDA into the winner
        //    We pull each AccountInfo into a local so the RefMut<u64> borrow
        //    lives for the entire block.

        let bet_info_ai = ctx.accounts.bet_info.to_account_info();
        let winner_ai   = ctx.accounts.winner.to_account_info();

        // ── Drain the PDA ───────────────────────────────────────────────────────
        {
            let mut from_lamports = bet_info_ai.lamports.borrow_mut(); // RefMut<u64>
            let new_balance = (*from_lamports)
                .checked_sub(pot)
                .ok_or(ErrorCode::NumericalOverflow)?;
            *from_lamports = new_balance;
        }

        // ── Credit the winner ───────────────────────────────────────────────────
        {
            let mut to_lamports = winner_ai.lamports.borrow_mut();      // RefMut<u64>
            let new_balance = (*to_lamports)
                .checked_add(pot)
                .ok_or(ErrorCode::NumericalOverflow)?;
            *to_lamports = new_balance;
        }

        // 6) Mark the bet resolved
        bet.resolved = true;

        Ok(())
    }





    pub fn timeout(ctx: Context<TimeoutCtx>) -> Result<()> {
        // 1) Immutable borrow to read state & verify
        let bet_acc = &ctx.accounts.bet_info;
        let now = Clock::get()?.slot;

        // signer check
        if !ctx.accounts.participant1.is_signer
            && !ctx.accounts.participant2.is_signer
        {
            return err!(ErrorCode::MissingSigner);
        }
        require!(!bet_acc.resolved, ErrorCode::AlreadyResolved);
        require!(now > bet_acc.deadline, ErrorCode::DeadlineNotReached);

        // 2) Build seeds
        let seed1: &[u8] = bet_acc.participant1.as_ref();
        let seed2: &[u8] = bet_acc.participant2.as_ref();
        let bump_slice: &[u8] = &[bet_acc.bump];
        let seeds = [seed1, seed2, bump_slice];
        let signer_seeds: &[&[&[u8]]] = &[&seeds];

        // 3) Return wagers
        let cpi1 = Transfer {
            from: ctx.accounts.bet_info.to_account_info(),
            to: ctx.accounts.participant1.to_account_info(),
        };
        let cpi_ctx1 =
            CpiContext::new_with_signer(
                ctx.accounts.system_program.to_account_info(),
                cpi1,
                signer_seeds,
            );
        system_program::transfer(cpi_ctx1, bet_acc.wager)?;

        let cpi2 = Transfer {
            from: ctx.accounts.bet_info.to_account_info(),
            to: ctx.accounts.participant2.to_account_info(),
        };
        let cpi_ctx2 =
            CpiContext::new_with_signer(
                ctx.accounts.system_program.to_account_info(),
                cpi2,
                signer_seeds,
            );
        system_program::transfer(cpi_ctx2, bet_acc.wager)?;

        // 4) Now mutate to mark resolved
        let bet = &mut ctx.accounts.bet_info;
        bet.resolved = true;
        Ok(())
    }
}

#[derive(Accounts)]
#[instruction(delay: u64, wager: u64)]
pub struct JoinCtx<'info> {
    #[account(mut, signer)]
    /// CHECK: participant1 is the first bettor signing and funding the PDA.
    pub participant1: AccountInfo<'info>,

    #[account(mut, signer)]
    /// CHECK: participant2 is the second bettor signing and funding the PDA.
    pub participant2: AccountInfo<'info>,

    /// CHECK: oracle is only stored on‐chain for later signature‐checks in win().
    pub oracle: AccountInfo<'info>,

    #[account(
        init,
        payer = participant1,
        space = 8 + BetInfo::LEN,
        seeds = [participant1.key().as_ref(), participant2.key().as_ref()],
        bump
    )]
    pub bet_info: Account<'info, BetInfo>,

    pub system_program: Program<'info, System>,
}

#[derive(Accounts)]
pub struct WinCtx<'info> {
    #[account(signer)]
    /// CHECK: oracle must match the stored oracle pubkey in BetInfo — signature enforced here.
    pub oracle: AccountInfo<'info>,

    #[account(mut)]
    /// CHECK: winner is one of the participants; we verify in code that it matches BetInfo.
    pub winner: AccountInfo<'info>,

    #[account(
        mut,
        seeds = [participant1.key().as_ref(), participant2.key().as_ref()],
        bump = bet_info.bump,
    )]
    pub bet_info: Account<'info, BetInfo>,

    /// CHECK: used purely for PDA derivation, no on‐chain data read.
    pub participant1: AccountInfo<'info>,

    /// CHECK: used purely for PDA derivation, no on‐chain data read.
    pub participant2: AccountInfo<'info>,

    pub system_program: Program<'info, System>,
}

#[derive(Accounts)]
pub struct TimeoutCtx<'info> {
    #[account(mut)]
    /// CHECK: participant1 must match BetInfo.participant1; signature enforced in code.
    pub participant1: AccountInfo<'info>,

    #[account(mut)]
    /// CHECK: participant2 must match BetInfo.participant2; signature enforced in code.
    pub participant2: AccountInfo<'info>,

    #[account(
        mut,
        seeds = [participant1.key().as_ref(), participant2.key().as_ref()],
        bump = bet_info.bump,
    )]
    pub bet_info: Account<'info, BetInfo>,

    pub system_program: Program<'info, System>,
}

#[account]
pub struct BetInfo {
    pub bump: u8,
    pub participant1: Pubkey,
    pub participant2: Pubkey,
    pub oracle: Pubkey,
    pub wager: u64,
    pub deadline: u64,
    pub resolved: bool,
}

impl BetInfo {
    // 1 + 32*3 + 8*2 + 1 = 1 + 96 + 16 + 1 = 114
    pub const LEN: usize = 114;
}

#[error_code]
pub enum ErrorCode {
    #[msg("Participants must be distinct.")]
    SameParticipant,
    #[msg("Overflow computing deadline or pot.")]
    NumericalOverflow,
    #[msg("Bet already resolved.")]
    AlreadyResolved,
    #[msg("Called by invalid oracle.")]
    InvalidOracle,
    #[msg("Deadline has already passed.")]
    DeadlinePassed,
    #[msg("Timeout deadline not yet reached.")]
    DeadlineNotReached,
    #[msg("Missing required signer.")]
    MissingSigner,
    #[msg("Winner must be one of the participants.")]
    InvalidWinner,
}

