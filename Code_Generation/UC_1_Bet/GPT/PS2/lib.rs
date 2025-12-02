use anchor_lang::prelude::*;
use anchor_lang::system_program;

declare_id!("3u7jbREqa2fnJBnTw3Cb5GFrcxw5fDauDDWXfm4dFRiu"); // replace with your program id

#[program]
pub mod bet_openai {
    use super::*;

    pub fn join(ctx: Context<JoinCtx>, delay: u64, wager: u64) -> Result<()> {
        require!(wager > 0, BetError::InvalidWager);

        let p1 = ctx.accounts.participant1.key();
        let p2 = ctx.accounts.participant2.key();

        let (smaller, larger) = if p1 < p2 { (p1, p2) } else { (p2, p1) };

        let bet_info = &mut ctx.accounts.bet_info;
        bet_info.participant1 = smaller;
        bet_info.participant2 = larger;
        bet_info.oracle = ctx.accounts.oracle.key();
        bet_info.wager = wager;
        bet_info.deadline = Clock::get()?.slot + delay;
        bet_info.bump = ctx.bumps.bet_info;

        // ✅ Transfer wagers safely via CPI
        let ix1 = anchor_lang::system_program::Transfer {
            from: ctx.accounts.participant1.to_account_info(),
            to: ctx.accounts.bet_info.to_account_info(),
        };
        let cpi_ctx1 = CpiContext::new(ctx.accounts.system_program.to_account_info(), ix1);
        system_program::transfer(cpi_ctx1, wager)?;

        let ix2 = anchor_lang::system_program::Transfer {
            from: ctx.accounts.participant2.to_account_info(),
            to: ctx.accounts.bet_info.to_account_info(),
        };
        let cpi_ctx2 = CpiContext::new(ctx.accounts.system_program.to_account_info(), ix2);
        system_program::transfer(cpi_ctx2, wager)?;

        Ok(())
    }

    pub fn win(ctx: Context<WinCtx>) -> Result<()> {
        let bet_info = &mut ctx.accounts.bet_info;

        // ✅ Ensure winner is a participant
        require!(
            ctx.accounts.winner.key() == ctx.accounts.participant1.key()
                || ctx.accounts.winner.key() == ctx.accounts.participant2.key(),
            BetError::InvalidWinner
        );

        let pot = bet_info.wager * 2;

        // ✅ Manual lamport transfer from PDA (bet_info) to winner
        **ctx.accounts.bet_info.to_account_info().try_borrow_mut_lamports()? -= pot;
        **ctx.accounts.winner.to_account_info().try_borrow_mut_lamports()? += pot;

        Ok(())
    }

    pub fn timeout(ctx: Context<TimeoutCtx>) -> Result<()> {
        let bet_info = &ctx.accounts.bet_info;
        let clock = Clock::get()?;
        require!(clock.slot > bet_info.deadline, BetError::DeadlineNotPassed);

        // Refund both participants
        let balance = **ctx.accounts.bet_info.to_account_info().lamports.borrow();
        let refund = balance / 2;

        **ctx.accounts.bet_info.to_account_info().try_borrow_mut_lamports()? -= balance;
        **ctx.accounts.participant1.try_borrow_mut_lamports()? += refund;
        **ctx.accounts.participant2.try_borrow_mut_lamports()? += balance - refund;

        Ok(())
    }
}

#[account]
pub struct BetInfo {
    pub participant1: Pubkey, // always smaller
    pub participant2: Pubkey, // always larger
    pub oracle: Pubkey,
    pub wager: u64,
    pub deadline: u64,
    pub bump: u8,
}

impl BetInfo {
    pub const LEN: usize = 8 + 32 + 32 + 32 + 8 + 8 + 1;
}

#[derive(Accounts)]
#[instruction(delay: u64, wager: u64)]
pub struct JoinCtx<'info> {
    #[account(mut)]
    pub participant1: Signer<'info>,
    #[account(mut)]
    pub participant2: Signer<'info>,

    /// CHECK: stored in bet_info
    pub oracle: UncheckedAccount<'info>,

    #[account(
        init,
        payer = participant1,
        space = BetInfo::LEN,
        seeds = [participant1.key().as_ref(), participant2.key().as_ref()],
        bump
    )]
    pub bet_info: Account<'info, BetInfo>,

    pub system_program: Program<'info, System>,
}


#[derive(Accounts)]
pub struct WinCtx<'info> {
    pub oracle: Signer<'info>,
    #[account(mut)]
    pub winner: SystemAccount<'info>,

    #[account(
        mut,
        seeds = [participant1.key().as_ref(), participant2.key().as_ref()],
        bump = bet_info.bump,
        has_one = oracle,
        close = winner
    )]
    pub bet_info: Account<'info, BetInfo>,

    /// CHECK: Only used for PDA derivation
    pub participant1: UncheckedAccount<'info>,
    /// CHECK: Only used for PDA derivation
    pub participant2: UncheckedAccount<'info>,

    pub system_program: Program<'info, System>,
}


#[derive(Accounts)]
pub struct TimeoutCtx<'info> {
    #[account(mut)]
    pub participant1: SystemAccount<'info>,
    #[account(mut)]
    pub participant2: SystemAccount<'info>,

    #[account(
        mut,
        seeds = [participant1.key().as_ref(), participant2.key().as_ref()],
        bump = bet_info.bump
    )]
    pub bet_info: Account<'info, BetInfo>,

    pub system_program: Program<'info, System>,
}


#[error_code]
pub enum BetError {
    #[msg("Invalid wager amount")]
    InvalidWager,
    #[msg("Deadline has not yet passed")]
    DeadlineNotPassed,
    #[msg("The declared winner is not a participant in this bet")]
    InvalidWinner,   // ✅ new error
}
