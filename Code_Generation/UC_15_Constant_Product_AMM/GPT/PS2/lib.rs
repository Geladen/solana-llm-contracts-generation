use anchor_lang::prelude::*;
use anchor_spl::token::{self, Mint, TokenAccount, Token, MintTo, Transfer, Burn};
use std::convert::TryInto;

declare_id!("BX8pVMzffzaGJfVZ7T8c736vEgTrDXb9pF2PtcTc9EBQ");

#[program]
pub mod amm_gpt {
    use super::*;

    pub fn initialize(ctx: Context<Initialize>, bump: u8) -> Result<()> {
        let seeds = &[b"amm", &[bump]];
        let signer = &[&seeds[..]];

        // Initialize AMM info account
        ctx.accounts.amm_info.supply = 0;
        ctx.accounts.amm_info.reserve0 = 0;
        ctx.accounts.amm_info.reserve1 = 0;

        Ok(())
    }

    pub fn deposit(ctx: Context<Deposit>, amount0: u64, amount1: u64) -> Result<u64> {
        let amm = &mut ctx.accounts.amm_info;

        // Compute liquidity
        let liquidity = if amm.supply == 0 {
            ((amount0 as u128 * amount1 as u128).integer_sqrt() as u64)
        } else {
            std::cmp::min(
                amount0.checked_mul(amm.supply).unwrap() / amm.reserve0,
                amount1.checked_mul(amm.supply).unwrap() / amm.reserve1,
            )
        };

        // Signer seeds for PDA
        let bump = ctx.bumps.amm_info; // Correct field
        let seeds = &[b"amm", &[bump]];
        let signer = &[&seeds[..]];

        // Transfer token0 in
        let cpi_accounts0 = Transfer {
            from: ctx.accounts.sender_token_account0.to_account_info(),
            to: ctx.accounts.pdas_token_account0.to_account_info(),
            authority: ctx.accounts.sender.to_account_info(),
        };
        token::transfer(
            CpiContext::new(ctx.accounts.token_program.to_account_info(), cpi_accounts0),
            amount0,
        )?;

        // Transfer token1 in
        let cpi_accounts1 = Transfer {
            from: ctx.accounts.sender_token_account1.to_account_info(),
            to: ctx.accounts.pdas_token_account1.to_account_info(),
            authority: ctx.accounts.sender.to_account_info(),
        };
        token::transfer(
            CpiContext::new(ctx.accounts.token_program.to_account_info(), cpi_accounts1),
            amount1,
        )?;

        // Mint LP tokens to sender
        let cpi_accounts_mint = MintTo {
            mint: ctx.accounts.lp_mint.to_account_info(),
            to: ctx.accounts.sender_lp.to_account_info(),
            authority: ctx.accounts.amm_info.to_account_info(),
        };
        token::mint_to(
            CpiContext::new_with_signer(
                ctx.accounts.token_program.to_account_info(),
                cpi_accounts_mint,
                signer,
            ),
            liquidity,
        )?;

        // Update reserves and supply
        amm.reserve0 = amm.reserve0.checked_add(amount0).unwrap();
        amm.reserve1 = amm.reserve1.checked_add(amount1).unwrap();
        amm.supply = amm.supply.checked_add(liquidity).unwrap();

        Ok(liquidity)
    }


    pub fn redeem(ctx: Context<Redeem>, amount: u64) -> Result<(u64, u64)> {
        let amm = &mut ctx.accounts.amm_info;

        let amount0 = amount.checked_mul(amm.reserve0).unwrap() / amm.supply;
        let amount1 = amount.checked_mul(amm.reserve1).unwrap() / amm.supply;

        // Burn LP tokens
        let bump = ctx.bumps.amm; // <-- correct bump
        let seeds = &[b"amm", &[bump]];
        let signer = &[&seeds[..]];
        let cpi_accounts_burn = Burn {
            from: ctx.accounts.sender_lp.to_account_info(),
            mint: ctx.accounts.lp_mint.to_account_info(),
            authority: ctx.accounts.sender.to_account_info(),
        };
        token::burn(
            CpiContext::new_with_signer(
                ctx.accounts.token_program.to_account_info(),
                cpi_accounts_burn,
                signer,
            ),
            amount,
        )?;

        // Transfer token0 out
        let cpi_accounts0 = Transfer {
            from: ctx.accounts.pdas_token_account0.to_account_info(),
            to: ctx.accounts.sender_token_account0.to_account_info(),
            authority: ctx.accounts.amm_info.to_account_info(),
        };
        token::transfer(
            CpiContext::new_with_signer(
                ctx.accounts.token_program.to_account_info(),
                cpi_accounts0,
                signer,
            ),
            amount0,
        )?;

        // Transfer token1 out
        let cpi_accounts1 = Transfer {
            from: ctx.accounts.pdas_token_account1.to_account_info(),
            to: ctx.accounts.sender_token_account1.to_account_info(),
            authority: ctx.accounts.amm_info.to_account_info(),
        };
        token::transfer(
            CpiContext::new_with_signer(
                ctx.accounts.token_program.to_account_info(),
                cpi_accounts1,
                signer,
            ),
            amount1,
        )?;

        amm.reserve0 = amm.reserve0.checked_sub(amount0).unwrap();
        amm.reserve1 = amm.reserve1.checked_sub(amount1).unwrap();
        amm.supply = amm.supply.checked_sub(amount).unwrap();

        Ok((amount0, amount1))
    }

    pub fn swap(ctx: Context<Swap>, amount_in: u64, is_mint0: bool) -> Result<u64> {
        let amm = &mut ctx.accounts.amm_info;

        let (reserve_in, reserve_out, sender_token_in, pdas_token_out) = if is_mint0 {
            (
                &mut amm.reserve0,
                &mut amm.reserve1,
                &ctx.accounts.sender_token_account0,
                &ctx.accounts.pdas_token_account1,
            )
        } else {
            (
                &mut amm.reserve1,
                &mut amm.reserve0,
                &ctx.accounts.sender_token_account1,
                &ctx.accounts.pdas_token_account0,
            )
        };

        let amount_in_with_fee = amount_in.checked_mul(997).unwrap() / 1000;
        let numerator = amount_in_with_fee.checked_mul(*reserve_out).unwrap();
        let denominator = (*reserve_in).checked_add(amount_in_with_fee).unwrap();
        let amount_out = numerator.checked_div(denominator).unwrap();

        let bump = ctx.bumps.amm;
        let seeds = &[b"amm", &[bump]];
        let signer = &[&seeds[..]];

        // Transfer in
        let cpi_accounts_in = Transfer {
            from: sender_token_in.to_account_info(),
            to: if is_mint0 { ctx.accounts.pdas_token_account0.to_account_info() } else { ctx.accounts.pdas_token_account1.to_account_info() },
            authority: ctx.accounts.sender.to_account_info(),
        };
        token::transfer(
            CpiContext::new(ctx.accounts.token_program.to_account_info(), cpi_accounts_in),
            amount_in,
        )?;

        // Transfer out
        let cpi_accounts_out = Transfer {
            from: if is_mint0 { ctx.accounts.pdas_token_account1.to_account_info() } else { ctx.accounts.pdas_token_account0.to_account_info() },
            to: if is_mint0 { ctx.accounts.sender_token_account1.to_account_info() } else { ctx.accounts.sender_token_account0.to_account_info() },
            authority: ctx.accounts.amm_info.to_account_info(),
        };
        token::transfer(
            CpiContext::new_with_signer(
                ctx.accounts.token_program.to_account_info(),
                cpi_accounts_out,
                signer,
            ),
            amount_out,
        )?;

        *reserve_in = reserve_in.checked_add(amount_in).unwrap();
        *reserve_out = reserve_out.checked_sub(amount_out).unwrap();

        Ok(amount_out)
    }
}

// Helper trait for integer square root
trait IntegerSqrt {
    fn integer_sqrt(self) -> Self;
}

impl IntegerSqrt for u128 {
    fn integer_sqrt(self) -> Self {
        (self as f64).sqrt() as u128
    }
}

#[account]
pub struct AMM {
    pub supply: u64,
    pub reserve0: u64,
    pub reserve1: u64,
}

#[derive(Accounts)]
pub struct Initialize<'info> {
    #[account(init, payer = payer, space = 8 + 24, seeds = [b"amm"], bump)]
    pub amm_info: Account<'info, AMM>,
    #[account(mut)]
    pub payer: Signer<'info>,
    pub system_program: Program<'info, System>,
}

#[derive(Accounts)]
pub struct Deposit<'info> {
    #[account(mut, seeds = [b"amm"], bump)]
    pub amm_info: Account<'info, AMM>,
    #[account(mut)]
    pub sender: Signer<'info>,
    #[account(mut)]
    pub sender_token_account0: Account<'info, TokenAccount>,
    #[account(mut)]
    pub sender_token_account1: Account<'info, TokenAccount>,
    #[account(mut)]
    pub pdas_token_account0: Account<'info, TokenAccount>,
    #[account(mut)]
    pub pdas_token_account1: Account<'info, TokenAccount>,
    #[account(mut)]
    pub lp_mint: Account<'info, Mint>,
    #[account(mut)]
    pub sender_lp: Account<'info, TokenAccount>,
    pub token_program: Program<'info, Token>,
}

#[derive(Accounts)]
pub struct Redeem<'info> {
    #[account(mut, seeds = [b"amm"], bump)]
    pub amm_info: Account<'info, AMM>,
    #[account(mut)]
    pub sender: Signer<'info>,
    #[account(mut)]
    pub sender_token_account0: Account<'info, TokenAccount>,
    #[account(mut)]
    pub sender_token_account1: Account<'info, TokenAccount>,
    #[account(mut)]
    pub pdas_token_account0: Account<'info, TokenAccount>,
    #[account(mut)]
    pub pdas_token_account1: Account<'info, TokenAccount>,
    #[account(mut)]
    pub lp_mint: Account<'info, Mint>,
    #[account(mut)]
    pub sender_lp: Account<'info, TokenAccount>,
    pub token_program: Program<'info, Token>,
}

#[derive(Accounts)]
pub struct Swap<'info> {
    #[account(mut, seeds = [b"amm"], bump)]
    pub amm_info: Account<'info, AMM>,
    #[account(mut)]
    pub sender: Signer<'info>,
    #[account(mut)]
    pub sender_token_account0: Account<'info, TokenAccount>,
    #[account(mut)]
    pub sender_token_account1: Account<'info, TokenAccount>,
    #[account(mut)]
    pub pdas_token_account0: Account<'info, TokenAccount>,
    #[account(mut)]
    pub pdas_token_account1: Account<'info, TokenAccount>,
    pub token_program: Program<'info, Token>,
}
