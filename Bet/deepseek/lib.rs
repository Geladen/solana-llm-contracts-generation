use anchor_lang::prelude::*;
use anchor_lang::system_program;

declare_id!("8D7BjnpoxZ5qdk8a4vbkTizFJt9RJkRwfAdsUs3YGCWu");

#[program]
pub mod bet_contract {
    use super::*;

    pub fn initialize(
        ctx: Context<Initialize>,
        delay: u64,
        wager: u64,
    ) -> Result<()> {
        let bet = &mut ctx.accounts.bet;
        bet.oracle = ctx.accounts.oracle.key();
        bet.deadline = Clock::get()?.slot + delay;
        bet.wager = wager;
        bet.state = BetState::Initialized;
        Ok(())
    }

    pub fn join(ctx: Context<Join>) -> Result<()> {
        // First get the wager amount without mutable borrow
        let wager_amount = ctx.accounts.bet.wager;
        
        // Check lamports before mutable operations
        let lamports = ctx.accounts.player.to_account_info().lamports();
        require!(lamports == wager_amount, BetError::InvalidWagerAmount);
        
        // Now do mutable operations
        let bet = &mut ctx.accounts.bet;
        require!(bet.state == BetState::Initialized, BetError::InvalidState);
        
        if bet.player1 == Pubkey::default() {
            bet.player1 = ctx.accounts.player.key();
            bet.player1_deposit = wager_amount;
            msg!("Player 1 joined with {} lamports", wager_amount);
        } else if bet.player2 == Pubkey::default() {
            require!(bet.player1 != ctx.accounts.player.key(), BetError::DuplicatePlayer);
            bet.player2 = ctx.accounts.player.key();
            bet.player2_deposit = wager_amount;
            bet.state = BetState::BothJoined;
            msg!("Player 2 joined with {} lamports", wager_amount);
        } else {
            return Err(BetError::GameFull.into());
        }
        
        // Prepare transfer after all mutable operations on bet are complete
        let transfer_ctx = CpiContext::new(
            ctx.accounts.system_program.to_account_info(),
            system_program::Transfer {
                from: ctx.accounts.player.to_account_info(),
                to: ctx.accounts.bet.to_account_info(),
            },
        );
        
        system_program::transfer(transfer_ctx, wager_amount)?;
        
        Ok(())
    }

    pub fn win(ctx: Context<Win>, winner: Pubkey) -> Result<()> {
        // First do all checks without mutable borrows
        {
            let bet = &ctx.accounts.bet;
            require!(bet.state == BetState::BothJoined, BetError::InvalidState);
            require!(ctx.accounts.oracle.key() == bet.oracle, BetError::Unauthorized);
            require!(Clock::get()?.slot <= bet.deadline, BetError::DeadlineExceeded);
            require!(
                winner == bet.player1 || winner == bet.player2,
                BetError::InvalidWinner
            );
        }
        
        // Then do the transfers
        let total_pot = ctx.accounts.bet.player1_deposit + ctx.accounts.bet.player2_deposit;
        
        let winner_account = if winner == ctx.accounts.bet.player1 {
            ctx.accounts.player1.clone()
        } else {
            ctx.accounts.player2.clone()
        };
        
        // Perform transfers without holding mutable bet reference
        **ctx.accounts.bet.to_account_info().try_borrow_mut_lamports()? -= total_pot;
        **winner_account.try_borrow_mut_lamports()? += total_pot;
        
        // Finally update state
        let bet = &mut ctx.accounts.bet;
        bet.state = BetState::Completed;
        
        msg!("Winner declared: {}", winner);
        Ok(())
    }

    pub fn timeout(ctx: Context<Timeout>) -> Result<()> {
        // First do all checks without mutable borrows
        {
            let bet = &ctx.accounts.bet;
            require!(bet.state == BetState::BothJoined, BetError::InvalidState);
            require!(Clock::get()?.slot > bet.deadline, BetError::DeadlineNotReached);
        }
        
        // Perform transfers
        let player1_lamports = ctx.accounts.bet.player1_deposit;
        let player2_lamports = ctx.accounts.bet.player2_deposit;
        
        **ctx.accounts.bet.to_account_info().try_borrow_mut_lamports()? -= player1_lamports;
        **ctx.accounts.player1.try_borrow_mut_lamports()? += player1_lamports;
        
        **ctx.accounts.bet.to_account_info().try_borrow_mut_lamports()? -= player2_lamports;
        **ctx.accounts.player2.try_borrow_mut_lamports()? += player2_lamports;
        
        // Finally update state
        let bet = &mut ctx.accounts.bet;
        bet.state = BetState::TimedOut;
        
        msg!("Contract timed out, funds refunded");
        Ok(())
    }
}

#[derive(Accounts)]
pub struct Initialize<'info> {
    #[account(
        init,
        payer = oracle,
        space = 8 + Bet::MAX_SIZE,
        seeds = [b"bet", oracle.key().as_ref()],
        bump
    )]
    pub bet: Account<'info, Bet>,
    #[account(mut)]
    pub oracle: Signer<'info>,
    pub system_program: Program<'info, System>,
}

#[derive(Accounts)]
pub struct Join<'info> {
    #[account(mut)]
    pub bet: Account<'info, Bet>,
    #[account(mut)]
    pub player: Signer<'info>,
    pub system_program: Program<'info, System>,
}

#[derive(Accounts)]
pub struct Win<'info> {
    #[account(mut)]
    pub bet: Account<'info, Bet>,
    #[account(mut)]
    pub oracle: Signer<'info>,
    /// CHECK: Verified by the bet account
    #[account(mut)]
    pub player1: UncheckedAccount<'info>,
    /// CHECK: Verified by the bet account
    #[account(mut)]
    pub player2: UncheckedAccount<'info>,
    pub system_program: Program<'info, System>,
}

#[derive(Accounts)]
pub struct Timeout<'info> {
    #[account(mut)]
    pub bet: Account<'info, Bet>,
    /// CHECK: Verified by the bet account
    #[account(mut)]
    pub player1: UncheckedAccount<'info>,
    /// CHECK: Verified by the bet account
    #[account(mut)]
    pub player2: UncheckedAccount<'info>,
    pub system_program: Program<'info, System>,
}

#[account]
pub struct Bet {
    pub oracle: Pubkey,
    pub deadline: u64,
    pub wager: u64,
    pub player1: Pubkey,
    pub player2: Pubkey,
    pub player1_deposit: u64,
    pub player2_deposit: u64,
    pub state: BetState,
}

impl Bet {
    pub const MAX_SIZE: usize = 32 + 8 + 8 + 32 + 32 + 8 + 8 + 1;
}

#[derive(AnchorSerialize, AnchorDeserialize, Clone, PartialEq, Eq)]
pub enum BetState {
    Initialized,
    BothJoined,
    Completed,
    TimedOut,
}

#[error_code]
pub enum BetError {
    #[msg("Invalid bet state for this operation")]
    InvalidState,
    #[msg("Unauthorized access")]
    Unauthorized,
    #[msg("Deadline has been exceeded")]
    DeadlineExceeded,
    #[msg("Deadline has not been reached yet")]
    DeadlineNotReached,
    #[msg("Invalid wager amount")]
    InvalidWagerAmount,
    #[msg("Game already has two players")]
    GameFull,
    #[msg("Player cannot join twice")]
    DuplicatePlayer,
    #[msg("Winner must be one of the players")]
    InvalidWinner,
}
