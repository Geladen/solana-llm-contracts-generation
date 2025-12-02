use anchor_lang::prelude::*;
use anchor_lang::system_program;

declare_id!("28H1btqWd9eRPpQepxHMpRcYyw2nenGWusNpzXbYoxyB");

#[program]
pub mod betting {
    use super::*;

    /// Both participants deposit their wagers into the PDA in one atomic call.
    pub fn join(
        ctx: Context<Join>,
        delay: u64,
        wager: u64,
    ) -> Result<()> {
        let p1     = &ctx.accounts.participant1;
        let p2     = &ctx.accounts.participant2;
        let oracle = &ctx.accounts.oracle;
        let bet    = &mut ctx.accounts.bet_info;

        // Basic validation
        require!(wager > 0, ErrorCode::InvalidWager);
        require!(p1.key() != p2.key(), ErrorCode::SameParticipant);
        require!(oracle.key() != p1.key(), ErrorCode::NotAuthorized);
        require!(oracle.key() != p2.key(), ErrorCode::NotAuthorized);

        // Initialize state
        bet.participant1 = *p1.key;
        bet.participant2 = *p2.key;
        bet.oracle       = *oracle.key;
        bet.wager        = wager;

        // Compute and store deadline
        let clock = Clock::get()?;
        bet.deadline = clock
            .slot
            .checked_add(delay)
            .ok_or(ErrorCode::DeadlineOverflow)?;

        // Store bump, mark unsettled
        bet.bump    = ctx.bumps.bet_info;
        bet.settled = false;

        // Transfer both wagers into the PDA
        let sys   = ctx.accounts.system_program.to_account_info();
        let to_pda = ctx.accounts.bet_info.to_account_info();

        // Participant1 → PDA
        system_program::transfer(
            CpiContext::new(
                sys.clone(),
                system_program::Transfer {
                    from: p1.to_account_info(),
                    to:   to_pda.clone(),
                },
            ),
            wager,
        )?;

        // Participant2 → PDA
        system_program::transfer(
            CpiContext::new(
                sys,
                system_program::Transfer {
                    from: p2.to_account_info(),
                    to:   to_pda,
                },
            ),
            wager,
        )?;

        Ok(())
    }

    /// Oracle resolves the bet before the deadline, awarding the full pot to `winner`.
    pub fn win(ctx: Context<Win>) -> Result<()> {
        // Extract the PDA and winner `AccountInfo` before borrowing state
        let pda_ai    = ctx.accounts.bet_info.to_account_info();
        let winner_ai = ctx.accounts.winner.to_account_info();

        // Mutable borrow of state
        let bet   = &mut ctx.accounts.bet_info;
        let oracle = &ctx.accounts.oracle;
        let clock  = Clock::get()?;

        // State validations
        require!(!bet.settled, ErrorCode::AlreadySettled);
        require!(oracle.is_signer, ErrorCode::NotAuthorized);
        require!(oracle.key() == bet.oracle, ErrorCode::NotAuthorized);

        let w_key = ctx.accounts.winner.key();
        require!(
            w_key == bet.participant1 || w_key == bet.participant2,
            ErrorCode::InvalidWinner
        );
        require!(clock.slot <= bet.deadline, ErrorCode::DeadlinePassed);

        // Compute total pot
        let pot = bet.wager.checked_mul(2).unwrap();

        // Direct lamports transfer from PDA → winner    
        {
            let mut from_balance = pda_ai.lamports.borrow_mut();    // RefMut<&mut u64>
            let mut to_balance   = winner_ai.lamports.borrow_mut(); // RefMut<&mut u64>

            // Checked arithmetic on the raw u64
            let new_from = (**from_balance)
                .checked_sub(pot)
                .ok_or(ErrorCode::Underflow)?;
            let new_to = (**to_balance)
                .checked_add(pot)
                .ok_or(ErrorCode::LamportOverflow)?;

            // Write back via double-deref
            **from_balance = new_from;
            **to_balance   = new_to;
        }

        bet.settled = true;
        Ok(())
    }

    /// After the deadline, either participant can refund both wagers.
    pub fn timeout(ctx: Context<Timeout>) -> Result<()> {
        // Extract AccountInfos before borrowing state
        let pda_ai = ctx.accounts.bet_info.to_account_info();
        let a1_ai  = ctx.accounts.participant1.to_account_info();
        let a2_ai  = ctx.accounts.participant2.to_account_info();

        let bet = &mut ctx.accounts.bet_info;
        let clock = Clock::get()?;

        // State validations
        require!(!bet.settled, ErrorCode::AlreadySettled);
        require!(clock.slot > bet.deadline, ErrorCode::DeadlineNotReached);
        require!(
            ctx.accounts.participant1.is_signer ||
            ctx.accounts.participant2.is_signer,
            ErrorCode::NotAuthorized
        );

        // Refund participant1
        {
            let mut from_balance = pda_ai.lamports.borrow_mut();
            let mut to_balance   = a1_ai.lamports.borrow_mut();

            let new_from = (**from_balance)
                .checked_sub(bet.wager)
                .ok_or(ErrorCode::Underflow)?;
            let new_to = (**to_balance)
                .checked_add(bet.wager)
                .ok_or(ErrorCode::LamportOverflow)?;

            **from_balance = new_from;
            **to_balance   = new_to;
        }

        // Refund participant2
        {
            let mut from_balance = pda_ai.lamports.borrow_mut();
            let mut to_balance   = a2_ai.lamports.borrow_mut();

            let new_from = (**from_balance)
                .checked_sub(bet.wager)
                .ok_or(ErrorCode::Underflow)?;
            let new_to = (**to_balance)
                .checked_add(bet.wager)
                .ok_or(ErrorCode::LamportOverflow)?;

            **from_balance = new_from;
            **to_balance   = new_to;
        }

        bet.settled = true;
        Ok(())
    }
}

#[derive(Accounts)]
#[instruction(delay: u64, wager: u64)]
pub struct Join<'info> {
    /// CHECK: funds their wager
    #[account(mut, signer)]
    participant1: AccountInfo<'info>,

    /// CHECK: funds their wager
    #[account(mut, signer)]
    participant2: AccountInfo<'info>,

    /// CHECK: stored as the oracle’s Pubkey
    oracle: AccountInfo<'info>,

    #[account(
        init,
        payer  = participant1,
        space  = 8 + BetInfo::LEN,
        seeds  = [participant1.key().as_ref(), participant2.key().as_ref()],
        bump
    )]
    bet_info: Account<'info, BetInfo>,

    system_program: Program<'info, System>,
}

#[derive(Accounts)]
pub struct Win<'info> {
    /// CHECK: must sign; key is verified in handler
    #[account(signer)]
    oracle: AccountInfo<'info>,

    /// CHECK: the declared winner; lamports will be deposited here
    #[account(mut)]
    winner: AccountInfo<'info>,

    #[account(
        mut,
        seeds   = [bet_info.participant1.as_ref(), bet_info.participant2.as_ref()],
        bump    = bet_info.bump,
        has_one = oracle
    )]
    bet_info: Account<'info, BetInfo>,

    /// CHECK: used for PDA derivation
    participant1: AccountInfo<'info>,

    /// CHECK: used for PDA derivation
    participant2: AccountInfo<'info>,

    system_program: Program<'info, System>,
}

#[derive(Accounts)]
pub struct Timeout<'info> {
    /// CHECK: original participant
    #[account(mut)]
    participant1: AccountInfo<'info>,

    /// CHECK: original participant
    #[account(mut)]
    participant2: AccountInfo<'info>,

    #[account(
        mut,
        seeds = [bet_info.participant1.as_ref(), bet_info.participant2.as_ref()],
        bump  = bet_info.bump
    )]
    bet_info: Account<'info, BetInfo>,

    system_program: Program<'info, System>,
}

#[account]
pub struct BetInfo {
    participant1: Pubkey,
    participant2: Pubkey,
    oracle:       Pubkey,
    deadline:     u64,
    wager:        u64,
    bump:         u8,
    settled:      bool,
}

impl BetInfo {
    /// 3×Pubkey (32 bytes each) + 2×u64 + 1×u8 + 1×bool
    pub const LEN: usize = 32 * 3 + 8 * 2 + 1 + 1;
}

#[error_code]
pub enum ErrorCode {
    #[msg("Wager must be non-zero.")]
    InvalidWager,
    #[msg("Participants must be distinct.")]
    SameParticipant,
    #[msg("Overflow computing deadline.")]
    DeadlineOverflow,
    #[msg("Not authorized.")]
    NotAuthorized,
    #[msg("Bet already settled.")]
    AlreadySettled,
    #[msg("Deadline not reached yet.")]
    DeadlineNotReached,
    #[msg("Deadline has passed.")]
    DeadlinePassed,
    #[msg("Winner not a participant.")]
    InvalidWinner,
    #[msg("Math underflow.")]
    Underflow,
    #[msg("Math overflow on lamports.")]
    LamportOverflow,
}

