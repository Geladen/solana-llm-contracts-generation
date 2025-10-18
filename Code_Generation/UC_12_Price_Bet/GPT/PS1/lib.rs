use anchor_lang::prelude::*;
use anchor_lang::solana_program::{
    program::invoke_signed,
    system_instruction,
};
use pyth_sdk_solana::{load_price_feed_from_account_info, state::PriceStatus};

declare_id!("2gF69apd5jKyZuxKMLfwQqLyLBT8xJBhTuTw14tVhirb");

#[program]
pub mod price_bet {
    use super::*;

    /// Owner initializes a bet and deposits their wager into the PDA.
    pub fn init(ctx: Context<InitCtx>, delay: u64, wager: u64, rate: u64) -> Result<()> {
        let clock = Clock::get()?;
        let deadline = clock.unix_timestamp as u64 + delay;

        let bet = &mut ctx.accounts.bet_info;
        bet.owner = *ctx.accounts.owner.key;
        bet.player = Pubkey::default();
        bet.wager = wager;
        bet.deadline = deadline;
        bet.rate = rate;
        bet.bump = ctx.bumps.bet_info;

        // Transfer wager lamports from owner to PDA
        let ix = system_instruction::transfer(
            ctx.accounts.owner.key,
            ctx.accounts.bet_info.to_account_info().key,
            wager,
        );
        anchor_lang::solana_program::program::invoke(
            &ix,
            &[
                ctx.accounts.owner.to_account_info(),
                ctx.accounts.bet_info.to_account_info(),
                ctx.accounts.system_program.to_account_info(),
            ],
        )?;

        Ok(())
    }

    /// Player joins by matching the owner's wager.
    pub fn join(ctx: Context<JoinCtx>) -> Result<()> {
        let wager = ctx.accounts.bet_info.wager;

        // require not already joined
        require!(ctx.accounts.bet_info.player == Pubkey::default(), BetError::AlreadyJoined);

        // require before deadline
        let clock = Clock::get()?;
        require!((clock.unix_timestamp as u64) < ctx.accounts.bet_info.deadline, BetError::DeadlinePassed);

        // transfer from player to PDA
        let ix = system_instruction::transfer(
            ctx.accounts.player.key,
            ctx.accounts.bet_info.to_account_info().key,
            wager,
        );
        anchor_lang::solana_program::program::invoke(
            &ix,
            &[
                ctx.accounts.player.to_account_info(),
                ctx.accounts.bet_info.to_account_info(),
                ctx.accounts.system_program.to_account_info(),
            ],
        )?;

        // record player after transfer
        let bet = &mut ctx.accounts.bet_info;
        bet.player = *ctx.accounts.player.key;

        Ok(())
    }

    /// Player claims pot if price condition satisfied.
    pub fn win(ctx: Context<WinCtx>) -> Result<()> {
        let clock = Clock::get()?;
        let bet = &ctx.accounts.bet_info;

        // Reject if deadline has passed
        require!(
            (clock.unix_timestamp as u64) < bet.deadline,
            BetError::DeadlinePassed
        );

        // Check that the player matches
        require!(
            bet.player == *ctx.accounts.player.key,
            BetError::UnauthorizedPlayer
        );

        // Load Pyth price feed safely
        let price_feed = load_price_feed_from_account_info(&ctx.accounts.price_feed)
            .map_err(|_| BetError::InvalidOracleOwner)?;

        let price_data = price_feed
            .get_price_no_older_than(clock.slot as i64, 120)
            .ok_or(BetError::PriceStale)?;

        // Compare price to rate
        require!(
            price_data.price >= bet.rate as i64,
            BetError::BetNotWon
        );

        // Transfer lamports to player and close PDA
        let player_account = &mut ctx.accounts.player.to_account_info();
        let bet_account = &mut ctx.accounts.bet_info.to_account_info();

        **player_account.try_borrow_mut_lamports()? += **bet_account.lamports.borrow();
        **bet_account.try_borrow_mut_lamports()? = 0;

        Ok(())
    }

    /// Owner reclaims pot after deadline if no win occurred.
    pub fn timeout(ctx: Context<TimeoutCtx>) -> Result<()> {
        let clock = Clock::get()?;
        let bet = &ctx.accounts.bet_info;

        // Only allow timeout after deadline
        require!(
            (clock.unix_timestamp as u64) >= bet.deadline,
            BetError::DeadlineNotPassed
        );

        // Only owner can timeout
        require!(
            bet.owner == *ctx.accounts.owner.key,
            BetError::UnauthorizedOwner
        );

        // No need for manual transfer: `close = owner` handles lamports refund and account closure
        Ok(())
    }
}

// ------------------------- Accounts -------------------------

#[derive(Accounts)]
pub struct InitCtx<'info> {
    #[account(mut)]
    pub owner: Signer<'info>,   // signer creating the bet

    #[account(
        init,
        payer = owner,
        space = 8 + OracleBetInfo::LEN,
        seeds = [owner.key().as_ref()],
        bump
    )]
    pub bet_info: Account<'info, OracleBetInfo>,

    pub system_program: Program<'info, System>,
}

#[derive(Accounts)]
pub struct JoinCtx<'info> {
    #[account(mut)]
    pub player: Signer<'info>,   // signer joining the bet

    /// CHECK: Only used as PDA seed for bet_info. Not read or written.
    pub owner: AccountInfo<'info>,

    #[account(
        mut,
        seeds = [owner.key().as_ref()],
        bump = bet_info.bump,
    )]
    pub bet_info: Account<'info, OracleBetInfo>,

    pub system_program: Program<'info, System>,
}

#[derive(Accounts)]
pub struct WinCtx<'info> {
    /// CHECK: player must match the stored player in bet_info
    pub player: AccountInfo<'info>, // <-- was Signer<'info'], now AccountInfo

    /// CHECK: only used as PDA seed for bet_info
    pub owner: AccountInfo<'info>,

    #[account(
        mut,
        seeds = [owner.key().as_ref()],
        bump = bet_info.bump,
    )]
    pub bet_info: Account<'info, OracleBetInfo>,

    /// CHECK: Pyth price feed account
    pub price_feed: AccountInfo<'info>,

    pub system_program: Program<'info, System>,
}

#[derive(Accounts)]
pub struct TimeoutCtx<'info> {
    #[account(mut)]
    pub owner: Signer<'info>, // signer reclaiming pot

    #[account(
        mut,
        seeds = [owner.key().as_ref()],
        bump = bet_info.bump,
        close = owner  // ✅ automatically refunds lamports to owner
    )]
    pub bet_info: Account<'info, OracleBetInfo>,

    pub system_program: Program<'info, System>,
}


// ------------------------- State -------------------------

#[account]
pub struct OracleBetInfo {
    pub owner: Pubkey,
    pub player: Pubkey,
    pub wager: u64,
    pub deadline: u64,
    pub rate: u64,
    pub bump: u8,
}

impl OracleBetInfo {
    pub const LEN: usize = 32 + 32 + 8 + 8 + 8 + 1;
}

// ------------------------- Errors -------------------------

#[error_code]
pub enum BetError {
    #[msg("Player already joined")]
    AlreadyJoined,
    #[msg("Deadline passed")]
    DeadlinePassed,
    #[msg("Deadline not yet passed")]
    DeadlineNotPassed,
    #[msg("Unauthorized player")]
    UnauthorizedPlayer,
    #[msg("Unauthorized owner")]
    UnauthorizedOwner,
    #[msg("Oracle price is stale")]
    PriceStale,
    #[msg("Oracle feed owner is invalid")]
    InvalidOracleOwner,     // ✅ <-- add this
    #[msg("Player did not win")]
    BetNotWon,
}
