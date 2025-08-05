use anchor_lang::prelude::*;
use anchor_lang::system_program::{self, Transfer, transfer};

declare_id!("6EUwiPRceY1Ag4NneCEXxiMzMChqLDPph4TXgi6ooTVC");

#[program]
pub mod bet_contract {
    use super::*;

    pub fn initialize(ctx: Context<Initialize>, deadline: u64, wager: u64) -> Result<()> {
        let state = &mut ctx.accounts.state;

        let (vault_pda, _bump) = Pubkey::find_program_address(
            &[b"vault", state.key().as_ref()],
            ctx.program_id,
        );

        state.player_one = Pubkey::default();
        state.player_two = Pubkey::default();
        state.oracle = ctx.accounts.oracle.key();
        state.deadline = deadline;
        state.wager = wager;
        state.status = BetStatus::Open;
        state.vault = vault_pda; // <-- Store vault pubkey

        Ok(())
    }


    pub fn join(ctx: Context<JoinCtx>) -> Result<()> {
        let state = &mut ctx.accounts.state;
        let player = ctx.accounts.player.key();

        require!(Clock::get()?.slot <= state.deadline, BetError::DeadlinePassed);
        require!(state.status == BetStatus::Open, BetError::BetClosed);
        require!(ctx.accounts.player_deposit.lamports() >= state.wager, BetError::InsufficientWager);

        // Ensure only two players and no duplicate joins
        if state.player_one == Pubkey::default() {
            state.player_one = player;
        } else if state.player_two == Pubkey::default() {
            require!(player != state.player_one, BetError::AlreadyJoined);
            state.player_two = player;
        } else {
            return err!(BetError::BetFull);
        }

        // Transfer wager to PDA vault
        let cpi_accounts = Transfer {
            from: ctx.accounts.player_deposit.to_account_info(),
            to: ctx.accounts.vault.to_account_info(),
        };
        let cpi_ctx = CpiContext::new(ctx.accounts.system_program.to_account_info(), cpi_accounts);
        transfer(cpi_ctx, state.wager)?;

        // Close joining if both players are in
        if state.player_one != Pubkey::default() && state.player_two != Pubkey::default() {
            state.status = BetStatus::Locked;
        }

        Ok(())
    }

    pub fn win(ctx: Context<WinCtx>, winner: Pubkey) -> Result<()> {
        let state = &mut ctx.accounts.state;

        require!(Clock::get()?.slot <= state.deadline, BetError::DeadlinePassed);
        require!(ctx.accounts.oracle.key() == state.oracle, BetError::UnauthorizedOracle);
        require!(state.status == BetStatus::Locked, BetError::InvalidState);
        require!(winner == state.player_one || winner == state.player_two, BetError::InvalidWinner);

        let vault_balance = ctx.accounts.vault.lamports();

        // Transfer entire pot to winner
        **ctx.accounts.vault.to_account_info().try_borrow_mut_lamports()? -= vault_balance;
        **ctx.accounts.winner.to_account_info().try_borrow_mut_lamports()? += vault_balance;

        state.status = BetStatus::Closed;

        Ok(())
    }

    pub fn timeout(ctx: Context<TimeoutCtx>) -> Result<()> {
        let state = &mut ctx.accounts.state;

        require!(Clock::get()?.slot > state.deadline, BetError::DeadlineNotPassed);
        require!(state.status == BetStatus::Locked, BetError::InvalidState);

        let half = ctx.accounts.vault.lamports() / 2;

        // Refund players
        **ctx.accounts.vault.to_account_info().try_borrow_mut_lamports()? -= half * 2;
        **ctx.accounts.player_one.to_account_info().try_borrow_mut_lamports()? += half;
        **ctx.accounts.player_two.to_account_info().try_borrow_mut_lamports()? += half;

        state.status = BetStatus::Closed;

        Ok(())
    }
}


#[account]
pub struct BetState {
    pub player_one: Pubkey,
    pub player_two: Pubkey,
    pub oracle: Pubkey,
    pub deadline: u64,
    pub wager: u64,
    pub vault: Pubkey, // <-- Add this line
    pub status: BetStatus,
}

#[derive(AnchorSerialize, AnchorDeserialize, Clone, PartialEq, Eq)]
pub enum BetStatus {
    Open,
    Locked,
    Closed,
}


#[derive(Accounts)]
pub struct Initialize<'info> {
    /// CHECK: 
    #[account(init, payer = initializer, space = 8 + 32*4 + 8*2 + 1)]
    pub state: Account<'info, BetState>,
    
    /// CHECK: 
    #[account(mut)]
    pub initializer: Signer<'info>,
    /// CHECK: Just stored in state, not accessed
    pub oracle: UncheckedAccount<'info>,
    
    /// CHECK: 
    #[account(
        init,
        payer = initializer,
        seeds = [b"vault", state.key().as_ref()],
        bump,
        space = 8, // minimal size; holds lamports only
    )]
    pub vault: AccountInfo<'info>,

    pub system_program: Program<'info, System>,
}

#[derive(Accounts)]
pub struct JoinCtx<'info> {
    /// CHECK: 
    #[account(mut, has_one = vault)]
    pub state: Account<'info, BetState>,
    
    /// CHECK: 
    #[account(mut)]
    pub player: Signer<'info>,
    /// CHECK: 
    #[account(mut)]
    pub player_deposit: AccountInfo<'info>,
    
    /// CHECK: 
    #[account(mut, seeds = [b"vault", state.key().as_ref()], bump)]
    pub vault: AccountInfo<'info>,
    pub system_program: Program<'info, System>,
}

#[derive(Accounts)]
pub struct WinCtx<'info> {
    /// CHECK: aaaa
    #[account(mut, has_one = oracle)]
    pub state: Account<'info, BetState>,
    pub oracle: Signer<'info>,

    /// CHECK: Winner must be one of the players. Validated in instruction logic.
    #[account(mut)]
    pub winner: AccountInfo<'info>,

    /// CHECK: Vault PDA derived via seeds. No data access, only lamports transferred.
    #[account(mut, seeds = [b"vault", state.key().as_ref()], bump)]
    pub vault: AccountInfo<'info>,
}


#[derive(Accounts)]
pub struct TimeoutCtx<'info> {
    /// CHECK: 
    #[account(mut)]
    pub state: Account<'info, BetState>,
    /// CHECK: doesn't need to be a signer
    #[account(mut)]
    pub player_one: AccountInfo<'info>,
    /// CHECK: doesn't need to be a signer
    #[account(mut)]
    pub player_two: AccountInfo<'info>,
    /// CHECK: PDA holding SOL
    #[account(mut)]
    pub vault: AccountInfo<'info>,
}


#[error_code]
pub enum BetError {
    #[msg("Wager must match expected amount.")]
    InsufficientWager,
    #[msg("This player already joined.")]
    AlreadyJoined,
    #[msg("Bet is full.")]
    BetFull,
    #[msg("Bet is not active.")]
    BetClosed,
    #[msg("Deadline already passed.")]
    DeadlinePassed,
    #[msg("Deadline not yet passed.")]
    DeadlineNotPassed,
    #[msg("Caller is not the authorized oracle.")]
    UnauthorizedOracle,
    #[msg("Winner must be one of the players.")]
    InvalidWinner,
    #[msg("Bet is not in a valid state for this action.")]
    InvalidState,
}

