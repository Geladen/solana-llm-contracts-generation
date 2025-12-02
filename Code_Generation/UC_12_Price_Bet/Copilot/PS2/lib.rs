use anchor_lang::prelude::*;
use anchor_lang::solana_program::program::invoke;
use anchor_lang::solana_program::system_instruction;
use anchor_lang::solana_program::sysvar::clock::Clock;
use pyth_client::{load_price, Price};

declare_id!("4Ghm1P18qwp45b3ufwHhNsFF4EwpbmYTXENRUjZ37wXH");

#[program]
pub mod price_bet {
    use super::*;

    pub fn init(ctx: Context<InitCtx>, delay: u64, wager: u64, rate: u64) -> Result<()> {
        let clock = Clock::get()?;
        let deadline = clock.slot.checked_add(delay).ok_or(ErrorCode::Overflow)?;

        let bet = &mut ctx.accounts.bet_info;
        bet.owner = *ctx.accounts.owner.key;
        bet.player = Pubkey::default();
        bet.wager = wager;
        bet.deadline = deadline;
        bet.rate = rate;

        if wager > 0 {
            let ix = system_instruction::transfer(
                ctx.accounts.owner.key,
                ctx.accounts.bet_info.to_account_info().key,
                wager,
            );
            invoke(
                &ix,
                &[
                    ctx.accounts.owner.to_account_info(),
                    ctx.accounts.bet_info.to_account_info(),
                    ctx.accounts.system_program.to_account_info(),
                ],
            )?;
        }

        Ok(())
    }

    pub fn join(ctx: Context<JoinCtx>) -> Result<()> {
        let bet = &mut ctx.accounts.bet_info;
        require!(bet.owner == *ctx.accounts.owner.key, ErrorCode::InvalidOwner);
        require!(bet.player == Pubkey::default(), ErrorCode::AlreadyJoined);
        require!(ctx.accounts.player.key() != ctx.accounts.owner.key(), ErrorCode::OwnerCannotJoin);
        require!(bet.wager > 0, ErrorCode::InvalidWagerAmount);

        let ix = system_instruction::transfer(
            ctx.accounts.player.key,
            ctx.accounts.bet_info.to_account_info().key,
            bet.wager,
        );
        invoke(
            &ix,
            &[
                ctx.accounts.player.to_account_info(),
                ctx.accounts.bet_info.to_account_info(),
                ctx.accounts.system_program.to_account_info(),
            ],
        )?;

        bet.player = *ctx.accounts.player.key;
        Ok(())
    }

    pub fn win(ctx: Context<WinCtx>) -> Result<()> {
        let bet = &ctx.accounts.bet_info;
        require!(bet.player == *ctx.accounts.player.key, ErrorCode::NotBetPlayer);

        let clock = Clock::get()?;
        require!(clock.slot >= bet.deadline, ErrorCode::TooEarlyToResolve);

        let pyth_program_id = pyth_client::ID;
        require!(ctx.accounts.price_feed.owner == &pyth_program_id, ErrorCode::InvalidOracleOwner);

        let price: Price = load_price(&ctx.accounts.price_feed).ok_or(ErrorCode::InvalidOracleData)?;
        let price_slot = price.valid_slot;
        let slot_now = clock.slot;
        const MAX_STALE_SLOTS: u64 = 300;
        require!(slot_now.checked_sub(price_slot).unwrap_or(u64::MAX) <= MAX_STALE_SLOTS, ErrorCode::StaleOraclePrice);

        let agg_price = price.agg.price;
        let expo = price.expo;

        let mut oracle_scaled: i128 = agg_price as i128;
        let mut rate_scaled: i128 = bet.rate as i128;

        if expo < 0 {
            let mult = 10i128.checked_pow((-expo) as u32).ok_or(ErrorCode::Overflow)?;
            rate_scaled = rate_scaled.checked_mul(mult).ok_or(ErrorCode::Overflow)?;
        } else if expo > 0 {
            let mult = 10i128.checked_pow(expo as u32).ok_or(ErrorCode::Overflow)?;
            oracle_scaled = oracle_scaled.checked_mul(mult).ok_or(ErrorCode::Overflow)?;
        }

        require!(oracle_scaled >= rate_scaled, ErrorCode::PlayerDidNotWin);
        Ok(())
    }

    pub fn timeout(ctx: Context<TimeoutCtx>) -> Result<()> {
        let bet = &ctx.accounts.bet_info;
        require!(bet.owner == *ctx.accounts.owner.key, ErrorCode::InvalidOwner);

        let clock = Clock::get()?;
        require!(clock.slot >= bet.deadline, ErrorCode::TooEarlyToResolve);
        Ok(())
    }
}

#[account]
pub struct OracleBetInfo {
    pub owner: Pubkey,
    pub player: Pubkey,
    pub wager: u64,
    pub deadline: u64,
    pub rate: u64,
    pub bump: u8,
}

#[derive(Accounts)]
pub struct InitCtx<'info> {
    #[account(mut, signer)]
    pub owner: Signer<'info>,

    #[account(
        init,
        payer = owner,
        space = 8 + std::mem::size_of::<OracleBetInfo>(),
        seeds = [owner.key().as_ref()],
        bump
    )]
    pub bet_info: Account<'info, OracleBetInfo>,

    pub system_program: Program<'info, System>,
}

#[derive(Accounts)]
pub struct JoinCtx<'info> {
    #[account(mut, signer)]
    pub player: Signer<'info>,

    /// CHECK: owner used only for PDA derivation
    pub owner: AccountInfo<'info>,

    #[account(
        mut,
        seeds = [owner.key().as_ref()],
        bump = bet_info.bump,
        has_one = owner @ ErrorCode::InvalidOwner
    )]
    pub bet_info: Account<'info, OracleBetInfo>,

    pub system_program: Program<'info, System>,
}

#[derive(Accounts)]
pub struct WinCtx<'info> {
    #[account(mut, signer)]
    pub player: Signer<'info>,

    /// CHECK: owner used only for PDA derivation
    pub owner: AccountInfo<'info>,

    #[account(
        mut,
        seeds = [owner.key().as_ref()],
        bump = bet_info.bump,
        has_one = player @ ErrorCode::NotBetPlayer,
        close = player
    )]
    pub bet_info: Account<'info, OracleBetInfo>,

    /// CHECK: Pyth price account
    pub price_feed: AccountInfo<'info>,

    pub system_program: Program<'info, System>,
}

#[derive(Accounts)]
pub struct TimeoutCtx<'info> {
    #[account(mut, signer)]
    pub owner: Signer<'info>,

    #[account(
        mut,
        seeds = [owner.key().as_ref()],
        bump = bet_info.bump,
        has_one = owner @ ErrorCode::InvalidOwner,
        close = owner
    )]
    pub bet_info: Account<'info, OracleBetInfo>,

    pub system_program: Program<'info, System>,
}

#[error_code]
pub enum ErrorCode {
    #[msg("Invalid owner for this bet.")]
    InvalidOwner,
    #[msg("Bet already has a player.")]
    AlreadyJoined,
    #[msg("Owner cannot join this bet.")]
    OwnerCannotJoin,
    #[msg("Invalid wager amount.")]
    InvalidWagerAmount,
    #[msg("Overflow.")]
    Overflow,
    #[msg("Not the player who joined this bet.")]
    NotBetPlayer,
    #[msg("Too early to resolve the bet.")]
    TooEarlyToResolve,
    #[msg("Invalid oracle owner.")]
    InvalidOracleOwner,
    #[msg("Invalid oracle data.")]
    InvalidOracleData,
    #[msg("Oracle price is stale.")]
    StaleOraclePrice,
    #[msg("Player did not win.")]
    PlayerDidNotWin,
}
