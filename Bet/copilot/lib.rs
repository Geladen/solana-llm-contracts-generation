use anchor_lang::prelude::*;
use anchor_lang::system_program;
use anchor_lang::solana_program::system_instruction;
use anchor_lang::solana_program::sysvar::clock::Clock;

declare_id!("88X6sMMLNmswgGTzYuijYEaFYve6arE2XbYBpbyGtgX4");

#[program]
pub mod bet {
    use super::*;

    /// Initialize the bet with oracle, deadline offset, and wager
    pub fn initialize_bet(
        ctx: Context<InitializeBet>,
        oracle: Pubkey,
        delay: u64,
        wager: u64,
    ) -> Result<()> {
        let bet = &mut ctx.accounts.bet;

        // Set up core state
        bet.player1 = *ctx.accounts.payer.key;
        bet.player2 = Pubkey::default();
        bet.oracle = oracle;
        bet.wager = wager;

        // deadline = current slot + delay
        let clock = Clock::get()?;
        bet.deadline = clock.slot + delay;
        bet.bump = ctx.bumps.bet;
        bet.state = 0; // waiting for player 2

        // Transfer wager lamports from player1 into the PDA
        let cpi_ctx = CpiContext::new(
            ctx.accounts.system_program.to_account_info(),
            system_program::Transfer {
                from: ctx.accounts.payer.to_account_info(),
                to: bet.to_account_info(),
            },
        );
        system_program::transfer(cpi_ctx, wager)?;

        Ok(())
    }

    /// Second player joins by depositing an equal wager
    pub fn join(ctx: Context<JoinBet>) -> Result<()> {
        let bet = &mut ctx.accounts.bet;

        // only allowed if still waiting for player2
        require!(bet.state == 0, BetError::InvalidState);
        // ensure this is not player1
        require!(
            ctx.accounts.player.key() != bet.player1,
            BetError::SamePlayer
        );

        // deposit same wager into PDA
        let cpi_ctx = CpiContext::new(
            ctx.accounts.system_program.to_account_info(),
            system_program::Transfer {
                from: ctx.accounts.player.to_account_info(),
                to: bet.to_account_info(),
            },
        );
        system_program::transfer(cpi_ctx, bet.wager)?;

        // record player2 and move to next state
        bet.player2 = ctx.accounts.player.key();
        bet.state = 1; // ready for oracle
        Ok(())
    }

    /// Oracle picks the winner before the deadline
    pub fn win(ctx: Context<Win>) -> Result<()> {
        let bet = &mut ctx.accounts.bet;

        // authorization & timing checks
        require!(bet.state == 1, BetError::InvalidState);
        require!(
            ctx.accounts.oracle.key() == bet.oracle,
            BetError::UnauthorizedOracle
        );
        let clock = Clock::get()?;
        require!(clock.slot <= bet.deadline, BetError::DeadlineExpired);

        // combine both wagers
        let amount = bet.wager.checked_mul(2).unwrap();

        // sign for PDA to pay out the pot
        let seeds: &[&[u8]] = &[&[
            b"bet",
            bet.player1.as_ref(),
            bet.oracle.as_ref(),
            &[bet.bump],
        ]
        .concat()];
        let signer = &[seeds];

        let cpi_ctx = CpiContext::new_with_signer(
            ctx.accounts.system_program.to_account_info(),
            system_program::Transfer {
                from: bet.to_account_info(),
                to: ctx.accounts.winner.to_account_info(),
            },
            signer,
        );
        system_program::transfer(cpi_ctx, amount)?;

        // state = finished (Anchor will close the PDA to `winner`)
        bet.state = 2;
        Ok(())
    }

    /// If oracle never acted, refund both players after deadline
    pub fn timeout(ctx: Context<Timeout>) -> Result<()> {
        let bet = &mut ctx.accounts.bet;

        // only if still unresolved
        require!(bet.state == 1, BetError::InvalidState);
        let clock = Clock::get()?;
        require!(
            clock.slot > bet.deadline,
            BetError::DeadlineNotExpired
        );

        let amount = bet.wager;

        // PDA seeds for signing
        let seeds: &[&[u8]] = &[&[
            b"bet",
            bet.player1.as_ref(),
            bet.oracle.as_ref(),
            &[bet.bump],
        ]
        .concat()];
        let signer = &[seeds];

        // refund player1
        let cpi1 = CpiContext::new_with_signer(
            ctx.accounts.system_program.to_account_info(),
            system_program::Transfer {
                from: bet.to_account_info(),
                to: ctx.accounts.player1.to_account_info(),
            },
            signer,
        );
        system_program::transfer(cpi1, amount)?;

        // refund player2
        let cpi2 = CpiContext::new_with_signer(
            ctx.accounts.system_program.to_account_info(),
            system_program::Transfer {
                from: bet.to_account_info(),
                to: ctx.accounts.player2.to_account_info(),
            },
            signer,
        );
        system_program::transfer(cpi2, amount)?;

        // state = finished (Anchor will close the PDA to `player1`)
        bet.state = 2;
        Ok(())
    }
}

/// PDA and signer contexts for initialization
#[derive(Accounts)]
#[instruction(oracle: Pubkey, delay: u64, wager: u64)]
pub struct InitializeBet<'info> {
    #[account(
        init,
        seeds = [b"bet", payer.key().as_ref(), oracle.as_ref()],
        bump,
        payer = payer,
        space = 8 + Bet::SIZE
    )]
    pub bet: Account<'info, Bet>,

    #[account(mut)]
    pub payer: Signer<'info>,

    pub system_program: Program<'info, System>,
}

/// Context for the second player joining
#[derive(Accounts)]
pub struct JoinBet<'info> {
    #[account(
        mut,
        seeds = [b"bet", bet.player1.as_ref(), bet.oracle.as_ref()],
        bump = bet.bump,
        constraint = bet.state == 0
    )]
    pub bet: Account<'info, Bet>,

    #[account(mut)]
    pub player: Signer<'info>,

    pub system_program: Program<'info, System>,
}

/// Context for the oracle declaring a winner
#[derive(Accounts)]
pub struct Win<'info> {
    #[account(
        mut,
        seeds = [b"bet", bet.player1.as_ref(), bet.oracle.as_ref()],
        bump = bet.bump,
        close = winner,
        constraint = bet.state == 1
    )]
    pub bet: Account<'info, Bet>,

    #[account(signer, address = bet.oracle)]
    pub oracle: Signer<'info>,

    #[account(mut)]
    pub winner: SystemAccount<'info>,

    pub system_program: Program<'info, System>,
}

/// Context for refunding both players after timeout
#[derive(Accounts)]
pub struct Timeout<'info> {
    #[account(
        mut,
        seeds = [b"bet", bet.player1.as_ref(), bet.oracle.as_ref()],
        bump = bet.bump,
        close = player1,
        constraint = bet.state == 1
    )]
    pub bet: Account<'info, Bet>,

    #[account(mut)]
    pub player1: SystemAccount<'info>,

    #[account(mut)]
    pub player2: SystemAccount<'info>,

    pub system_program: Program<'info, System>,
}

/// On‐chain representation of a bet
#[account]
pub struct Bet {
    pub player1: Pubkey,
    pub player2: Pubkey,
    pub oracle: Pubkey,
    pub wager:   u64,
    pub deadline: u64,
    pub bump:     u8,
    pub state:    u8,   // 0 = waiting, 1 = ready, 2 = finished
}

impl Bet {
    // 32*3 Pubkeys + 8*2 u64s + 1 + 1 = 32*3 + 16 + 2 = 114 bytes
    pub const SIZE: usize = 32 * 3 + 8 * 2 + 1 + 1;
}

/// Custom error messages
#[error_code]
pub enum BetError {
    #[msg("Operation invalid in current state")]
    InvalidState,
    #[msg("Only the designated oracle can call this")]
    UnauthorizedOracle,
    #[msg("Deadline has already passed")]
    DeadlineExpired,
    #[msg("Deadline has not yet passed")]
    DeadlineNotExpired,
    #[msg("Second player must be different from the first")]
    SamePlayer,
}

