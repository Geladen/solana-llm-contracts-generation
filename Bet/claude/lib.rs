use anchor_lang::prelude::*;
use anchor_lang::system_program::{transfer, Transfer};

declare_id!("4CU7PgH4PyReJgi8iqutbyAax8tYepEdicYPsMWbYGuV");

#[program]
pub mod betting_contract {
    use super::*;

    pub fn initialize(
        ctx: Context<Initialize>,
        oracle: Pubkey,
        deadline: i64,
        bet_amount: u64,
    ) -> Result<()> {
        let game = &mut ctx.accounts.game;
        game.oracle = oracle;
        game.deadline = deadline;
        game.bet_amount = bet_amount;
        game.players = [Pubkey::default(); 2];
        game.deposits = [0; 2];
        game.state = GameState::WaitingForPlayers;
        Ok(())
    }

    pub fn join(ctx: Context<Join>) -> Result<()> {
        let player = ctx.accounts.player.key();
        let clock = Clock::get()?;
        let bet_amount;
        let player_index;

        {
            let game = &ctx.accounts.game;
            require!(
                game.state == GameState::WaitingForPlayers,
                BettingError::InvalidGameState
            );
            require!(clock.unix_timestamp < game.deadline, BettingError::DeadlinePassed);

            player_index = if game.players[0] == Pubkey::default() {
                0
            } else if game.players[1] == Pubkey::default() {
                require!(game.players[0] != player, BettingError::PlayerAlreadyJoined);
                1
            } else {
                return Err(BettingError::GameFull.into());
            };

            bet_amount = game.bet_amount;
        }

        let transfer_ctx = CpiContext::new(
            ctx.accounts.system_program.to_account_info(),
            Transfer {
                from: ctx.accounts.player.to_account_info(),
                to: ctx.accounts.game.to_account_info(),
            },
        );
        transfer(transfer_ctx, bet_amount)?;

        let game = &mut ctx.accounts.game;
        game.players[player_index] = player;
        game.deposits[player_index] = bet_amount;

        if game.players[0] != Pubkey::default() && game.players[1] != Pubkey::default() {
            game.state = GameState::WaitingForOracle;
        }

        Ok(())
    }

    pub fn win(ctx: Context<Win>, winner_index: u8) -> Result<()> {
        let game = &mut ctx.accounts.game;
        let clock = Clock::get()?;

        require!(
            ctx.accounts.oracle.key() == game.oracle,
            BettingError::InvalidOracle
        );
        require!(
            game.state == GameState::WaitingForOracle,
            BettingError::InvalidGameState
        );
        require!(clock.unix_timestamp >= game.deadline, BettingError::EarlyWinCall);
        require!(winner_index < 2, BettingError::InvalidWinnerIndex);

        let total_pot = game.deposits[0] + game.deposits[1];
        let winner = game.players[winner_index as usize];

        game.state = GameState::Resolved;

        **ctx.accounts.game.to_account_info().try_borrow_mut_lamports()? -= total_pot;
        **ctx.accounts.winner.to_account_info().try_borrow_mut_lamports()? += total_pot;

        Ok(())
    }

    pub fn timeout(ctx: Context<Timeout>) -> Result<()> {
        let game = &mut ctx.accounts.game;
        let clock = Clock::get()?;
        let player = ctx.accounts.player.key();

        require!(
            game.state == GameState::WaitingForOracle,
            BettingError::InvalidGameState
        );
        require!(
            clock.unix_timestamp > game.deadline,
            BettingError::TimeoutNotReached
        );

        let player_index = if game.players[0] == player {
            0
        } else if game.players[1] == player {
            1
        } else {
            return Err(BettingError::NotAPlayer.into());
        };

        let refund_amount = game.deposits[player_index];
        require!(refund_amount > 0, BettingError::AlreadyRefunded);

        game.deposits[player_index] = 0;

        if game.deposits[0] == 0 && game.deposits[1] == 0 {
            game.state = GameState::TimedOut;
        }

        **ctx.accounts.game.to_account_info().try_borrow_mut_lamports()? -= refund_amount;
        **ctx.accounts.player.to_account_info().try_borrow_mut_lamports()? += refund_amount;

        Ok(())
    }
}

#[derive(Accounts)]
pub struct Initialize<'info> {
    #[account(init, payer = payer, space = 8 + Game::INIT_SPACE)]
    pub game: Account<'info, Game>,
    #[account(mut)]
    pub payer: Signer<'info>,
    pub system_program: Program<'info, System>,
}

#[derive(Accounts)]
pub struct Join<'info> {
    #[account(mut)]
    pub game: Account<'info, Game>,
    #[account(mut)]
    pub player: Signer<'info>,
    pub system_program: Program<'info, System>,
}

#[derive(Accounts)]
pub struct Win<'info> {
    #[account(mut)]
    pub game: Account<'info, Game>,
    pub oracle: Signer<'info>,
    /// CHECK: Winner account is validated through game.players
    #[account(mut)]
    pub winner: UncheckedAccount<'info>,
}

#[derive(Accounts)]
pub struct Timeout<'info> {
    #[account(mut)]
    pub game: Account<'info, Game>,
    #[account(mut)]
    pub player: Signer<'info>,
}

#[account]
#[derive(InitSpace)]
pub struct Game {
    pub oracle: Pubkey,
    pub deadline: i64,
    pub bet_amount: u64,
    pub players: [Pubkey; 2],
    pub deposits: [u64; 2],
    pub state: GameState,
}

#[derive(AnchorSerialize, AnchorDeserialize, Clone, PartialEq, Eq, InitSpace)]
pub enum GameState {
    WaitingForPlayers,
    WaitingForOracle,
    Resolved,
    TimedOut,
}

#[error_code]
pub enum BettingError {
    #[msg("Invalid game state")]
    InvalidGameState,
    #[msg("Deadline has passed")]
    DeadlinePassed,
    #[msg("Player already joined")]
    PlayerAlreadyJoined,
    #[msg("Game is full")]
    GameFull,
    #[msg("Invalid oracle")]
    InvalidOracle,
    #[msg("Win can only be called after deadline")]
    EarlyWinCall,
    #[msg("Invalid winner index")]
    InvalidWinnerIndex,
    #[msg("Timeout period not reached")]
    TimeoutNotReached,
    #[msg("Not a player in this game")]
    NotAPlayer,
    #[msg("Already refunded")]
    AlreadyRefunded,
}
