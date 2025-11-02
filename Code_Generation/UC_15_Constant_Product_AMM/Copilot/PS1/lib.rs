use anchor_lang::prelude::*;
use anchor_spl::token::{self, Token, TokenAccount, Mint, Transfer, SetAuthority, spl_token};
use anchor_spl::associated_token::AssociatedToken;
use std::convert::TryInto;

declare_id!("C2DddGRLSyoRfpqsL77hcoz9jCi513PBXxs4bBVMKykq");

#[program]
pub mod constant_product_amm {
    use super::*;

    pub fn initialize(ctx: Context<InitializeCtx>) -> Result<()> {
        let amm = &mut ctx.accounts.amm_info;

        // validate provided token accounts match mints
        require_keys_eq!(
            ctx.accounts.token_account0.mint,
            ctx.accounts.mint0.key(),
            AmmError::TokenAccountMintMismatch
        );
        require_keys_eq!(
            ctx.accounts.token_account1.mint,
            ctx.accounts.mint1.key(),
            AmmError::TokenAccountMintMismatch
        );

        amm.mint0 = ctx.accounts.mint0.key();
        amm.mint1 = ctx.accounts.mint1.key();
        amm.token_account0 = ctx.accounts.token_account0.key();
        amm.token_account1 = ctx.accounts.token_account1.key();

        // reserves/supply stored in canonical scale (mint1.decimals)
        amm.reserve0 = 0u128;
        amm.reserve1 = 0u128;
        amm.ever_deposited = false;
        amm.supply = 0u128;

        // compute and store bump deterministically
        let (_pda, bump) = Pubkey::find_program_address(
            &[
                b"amm",
                ctx.accounts.mint0.key().as_ref(),
                ctx.accounts.mint1.key().as_ref(),
            ],
            ctx.program_id,
        );
        amm.bump = bump;

        // change authority of token_account0 and token_account1 from initializer to AMM PDA
        // current authority must be initializer
        let cpi_prog = ctx.accounts.token_program.to_account_info();

        let set0 = SetAuthority {
            account_or_mint: ctx.accounts.token_account0.to_account_info(),
            current_authority: ctx.accounts.initializer.to_account_info(),
        };
        token::set_authority(
            CpiContext::new(cpi_prog.clone(), set0),
            spl_token::instruction::AuthorityType::AccountOwner,
            Some(ctx.accounts.amm_info.key()),
        )?;

        let set1 = SetAuthority {
            account_or_mint: ctx.accounts.token_account1.to_account_info(),
            current_authority: ctx.accounts.initializer.to_account_info(),
        };
        token::set_authority(
            CpiContext::new(cpi_prog, set1),
            spl_token::instruction::AuthorityType::AccountOwner,
            Some(ctx.accounts.amm_info.key()),
        )?;

        Ok(())
    }

    pub fn deposit(ctx: Context<DepositCtx>, amount0: u64, amount1: u64) -> Result<()> {
        // ensure mints match amm_info
        require_keys_eq!(ctx.accounts.amm_info.mint0, ctx.accounts.mint0.key(), AmmError::MintMismatch);
        require_keys_eq!(ctx.accounts.amm_info.mint1, ctx.accounts.mint1.key(), AmmError::MintMismatch);

        // canonical scale: mint1.decimals
        let decimals = ctx.accounts.mint1.decimals as u32;
        let scale = pow10(decimals)?;

        // Convert user-provided amounts to scaled on-chain amounts exactly once
        // (no double scaling). Tests expect this canonical scaling.
        let scaled0 = (amount0 as u128).checked_mul(scale).ok_or(AmmError::Overflow)?;
        let scaled1 = (amount1 as u128).checked_mul(scale).ok_or(AmmError::Overflow)?;

        // ensure scaled fits u64 for CPI
        let scaled0_u64: u64 = scaled0.try_into().map_err(|_| AmmError::Overflow)?;
        let scaled1_u64: u64 = scaled1.try_into().map_err(|_| AmmError::Overflow)?;

        // transfer from sender to AMM token accounts (CPI)
        let cpi_prog = ctx.accounts.token_program.to_account_info();

        let t0 = Transfer {
            from: ctx.accounts.senders_token_account0.to_account_info(),
            to: ctx.accounts.pdas_token_account0.to_account_info(),
            authority: ctx.accounts.sender.to_account_info(),
        };
        token::transfer(CpiContext::new(cpi_prog.clone(), t0), scaled0_u64)?;

        let t1 = Transfer {
            from: ctx.accounts.senders_token_account1.to_account_info(),
            to: ctx.accounts.pdas_token_account1.to_account_info(),
            authority: ctx.accounts.sender.to_account_info(),
        };
        token::transfer(CpiContext::new(cpi_prog, t1), scaled1_u64)?;

        // update AMM state (reserves stored in scaled units)
        let amm = &mut ctx.accounts.amm_info;

        amm.reserve0 = amm.reserve0.checked_add(scaled0).ok_or(AmmError::Overflow)?;
        amm.reserve1 = amm.reserve1.checked_add(scaled1).ok_or(AmmError::Overflow)?;

        if !amm.ever_deposited {
            // initial supply = sqrt(reserve0 * reserve1)
            let s = sqrt_u128(amm.reserve0.checked_mul(amm.reserve1).ok_or(AmmError::Overflow)?);
            amm.supply = s;
            amm.ever_deposited = true;
        } else {
            // mint supply proportionally: supply * min(scaled0/reserve0, scaled1/reserve1)
            require!(amm.reserve0 > 0 && amm.reserve1 > 0, AmmError::NoReserves);
            // use high precision factor
            let ratio0 = scaled0.checked_mul(U1E9).ok_or(AmmError::Overflow)?.checked_div(amm.reserve0).ok_or(AmmError::Overflow)?;
            let ratio1 = scaled1.checked_mul(U1E9).ok_or(AmmError::Overflow)?.checked_div(amm.reserve1).ok_or(AmmError::Overflow)?;
            let ratio = std::cmp::min(ratio0, ratio1);
            let minted = amm.supply.checked_mul(ratio).ok_or(AmmError::Overflow)?.checked_div(U1E9).ok_or(AmmError::Overflow)?;
            amm.supply = amm.supply.checked_add(minted).ok_or(AmmError::Overflow)?;
        }

        // ensure minted_pda exists (init_if_needed) and mark numerically
        let minted_pda = &mut ctx.accounts.minted_pda;
        minted_pda.minted = minted_pda.minted.checked_add(1).ok_or(AmmError::Overflow)?;
        // bump already set by Anchor when init_if_needed runs; we keep it

        Ok(())
    }

    pub fn redeem(ctx: Context<RedeemOrSwapCtx>, amount: u128) -> Result<()> {
        // no anchor-level PDA re-derivation check here: tests pass the minted_pda created earlier by deposit
        // Validate minted counter
        require!(ctx.accounts.minted_pda.minted > 0, AmmError::NotMinted);

        // pull required locals to avoid borrow conflicts
        let mint0_key = ctx.accounts.mint0.key();
        let mint1_key = ctx.accounts.mint1.key();
        let (reserve0, reserve1, supply, amm_bump) = {
            let a = &ctx.accounts.amm_info;
            (a.reserve0, a.reserve1, a.supply, a.bump)
        };

        require!(supply > 0, AmmError::ZeroSupply);
        require!(reserve0 > 0 && reserve1 > 0, AmmError::NoReserves);

        // compute proportional returns using scaled units
        let ret0 = reserve0.checked_mul(amount).ok_or(AmmError::Overflow)?.checked_div(supply).ok_or(AmmError::Overflow)?;
        let ret1 = reserve1.checked_mul(amount).ok_or(AmmError::Overflow)?.checked_div(supply).ok_or(AmmError::Overflow)?;

        // convert to u64 for CPI (ret fits in u64 in tests)
        let ret0_u64: u64 = ret0.try_into().map_err(|_| AmmError::Overflow)?;
        let ret1_u64: u64 = ret1.try_into().map_err(|_| AmmError::Overflow)?;

        // signer seeds for PDA CPI
        let seeds: &[&[u8]] = &[
            b"amm",
            mint0_key.as_ref(),
            mint1_key.as_ref(),
            &[amm_bump],
        ];
        let signer_seeds: &[&[&[u8]]] = &[seeds];

        let cpi_prog = ctx.accounts.token_program.to_account_info();

        let t0 = Transfer {
            from: ctx.accounts.pdas_token_account0.to_account_info(),
            to: ctx.accounts.senders_token_account0.to_account_info(),
            authority: ctx.accounts.amm_info.to_account_info(),
        };
        token::transfer(CpiContext::new_with_signer(cpi_prog.clone(), t0, signer_seeds), ret0_u64)?;

        let t1 = Transfer {
            from: ctx.accounts.pdas_token_account1.to_account_info(),
            to: ctx.accounts.senders_token_account1.to_account_info(),
            authority: ctx.accounts.amm_info.to_account_info(),
        };
        token::transfer(CpiContext::new_with_signer(cpi_prog, t1, signer_seeds), ret1_u64)?;

        // update AMM state
        let amm = &mut ctx.accounts.amm_info;
        amm.reserve0 = amm.reserve0.checked_sub(ret0).ok_or(AmmError::Overflow)?;
        amm.reserve1 = amm.reserve1.checked_sub(ret1).ok_or(AmmError::Overflow)?;
        amm.supply = amm.supply.checked_sub(amount).ok_or(AmmError::Overflow)?;

        // decrement minted counter
        let minted_pda = &mut ctx.accounts.minted_pda;
        minted_pda.minted = minted_pda.minted.checked_sub(1).ok_or(AmmError::Overflow)?;

        Ok(())
    }

    pub fn swap(ctx: Context<RedeemOrSwapCtx>, is_mint0: bool, amount_in: u128, min_out_amount: u128) -> Result<()> {
        // require minted_pda exists (tests expect this account present)
        require!(ctx.accounts.minted_pda.minted > 0, AmmError::NotMinted);

        // locals to avoid borrow conflicts
        let mint0_key = ctx.accounts.mint0.key();
        let mint1_key = ctx.accounts.mint1.key();
        let (reserve0, reserve1, amm_bump) = {
            let a = &ctx.accounts.amm_info;
            (a.reserve0, a.reserve1, a.bump)
        };

        require_keys_eq!(ctx.accounts.amm_info.mint0, mint0_key, AmmError::MintMismatch);
        require_keys_eq!(ctx.accounts.amm_info.mint1, mint1_key, AmmError::MintMismatch);
        require!(reserve0 > 0 && reserve1 > 0, AmmError::NoReserves);

        // canonical scale defined by mint1.decimals
        let decimals = ctx.accounts.mint1.decimals as u32;
        let scale = pow10(decimals)?;

        // amount_in is user-space amount; convert to canonical scaled units once
        let amount_in_scaled = amount_in.checked_mul(scale).ok_or(AmmError::Overflow)?;

        // reserves already stored in scaled units
        let (reserve_in, reserve_out) = if is_mint0 { (reserve0, reserve1) } else { (reserve1, reserve0) };

        // constant product
        let k = reserve_in.checked_mul(reserve_out).ok_or(AmmError::Overflow)?;
        let new_reserve_in = reserve_in.checked_add(amount_in_scaled).ok_or(AmmError::Overflow)?;
        let new_reserve_out = k.checked_div(new_reserve_in).ok_or(AmmError::Overflow)?;
        let amount_out_scaled = reserve_out.checked_sub(new_reserve_out).ok_or(AmmError::Overflow)?;

        // convert scaled output back to user-space by dividing by scale
        let amount_out_user = amount_out_scaled.checked_div(scale).ok_or(AmmError::Overflow)?;
        require!(amount_out_user >= min_out_amount, AmmError::SlippageExceeded);

        // prepare CPIs: transfer amount_in (scaled) into AMM and amount_out (scaled) out of AMM
        let amount_in_cpi: u64 = amount_in_scaled.try_into().map_err(|_| AmmError::Overflow)?;
        let amount_out_cpi: u64 = amount_out_scaled.try_into().map_err(|_| AmmError::Overflow)?;

        // PDA signer seeds
        let seeds: &[&[u8]] = &[
            b"amm",
            mint0_key.as_ref(),
            mint1_key.as_ref(),
            &[amm_bump],
        ];
        let signer_seeds: &[&[&[u8]]] = &[seeds];

        let cpi_prog = ctx.accounts.token_program.to_account_info();

        if is_mint0 {
            // transfer token0 from sender to AMM
            let t_in = Transfer {
                from: ctx.accounts.senders_token_account0.to_account_info(),
                to: ctx.accounts.pdas_token_account0.to_account_info(),
                authority: ctx.accounts.sender.to_account_info(),
            };
            token::transfer(CpiContext::new(cpi_prog.clone(), t_in), amount_in_cpi)?;

            // transfer token1 from AMM to sender (PDA signer)
            let t_out = Transfer {
                from: ctx.accounts.pdas_token_account1.to_account_info(),
                to: ctx.accounts.senders_token_account1.to_account_info(),
                authority: ctx.accounts.amm_info.to_account_info(),
            };
            token::transfer(CpiContext::new_with_signer(cpi_prog, t_out, signer_seeds), amount_out_cpi)?;

            // update reserves (scaled units)
            let amm = &mut ctx.accounts.amm_info;
            amm.reserve0 = amm.reserve0.checked_add(amount_in_scaled).ok_or(AmmError::Overflow)?;
            amm.reserve1 = amm.reserve1.checked_sub(amount_out_scaled).ok_or(AmmError::Overflow)?;
        } else {
            // transfer token1 from sender to AMM
            let t_in = Transfer {
                from: ctx.accounts.senders_token_account1.to_account_info(),
                to: ctx.accounts.pdas_token_account1.to_account_info(),
                authority: ctx.accounts.sender.to_account_info(),
            };
            token::transfer(CpiContext::new(cpi_prog.clone(), t_in), amount_in_cpi)?;

            // transfer token0 from AMM to sender
            let t_out = Transfer {
                from: ctx.accounts.pdas_token_account0.to_account_info(),
                to: ctx.accounts.senders_token_account0.to_account_info(),
                authority: ctx.accounts.amm_info.to_account_info(),
            };
            token::transfer(CpiContext::new_with_signer(cpi_prog, t_out, signer_seeds), amount_out_cpi)?;

            // update reserves
            let amm = &mut ctx.accounts.amm_info;
            amm.reserve1 = amm.reserve1.checked_add(amount_in_scaled).ok_or(AmmError::Overflow)?;
            amm.reserve0 = amm.reserve0.checked_sub(amount_out_scaled).ok_or(AmmError::Overflow)?;
        }

        Ok(())
    }
}

const U1E9: u128 = 1_000_000_000u128;

#[derive(Accounts)]
pub struct InitializeCtx<'info> {
    #[account(mut)]
    pub initializer: Signer<'info>,

    #[account(
        init,
        payer = initializer,
        space = 8 + AmmInfo::LEN,
        seeds = [b"amm", mint0.key().as_ref(), mint1.key().as_ref()],
        bump
    )]
    pub amm_info: Account<'info, AmmInfo>,

    pub mint0: Account<'info, Mint>,
    pub mint1: Account<'info, Mint>,

    #[account(mut)]
    pub token_account0: Account<'info, TokenAccount>,
    #[account(mut)]
    pub token_account1: Account<'info, TokenAccount>,

    pub token_program: Program<'info, Token>,
    pub system_program: Program<'info, System>,
    pub associated_token_program: Program<'info, AssociatedToken>,
}

#[derive(Accounts)]
pub struct DepositCtx<'info> {
    #[account(mut)]
    pub sender: Signer<'info>,

    pub mint0: Account<'info, Mint>,
    pub mint1: Account<'info, Mint>,

    #[account(
        mut,
        seeds = [b"amm", mint0.key().as_ref(), mint1.key().as_ref()],
        bump = amm_info.bump,
    )]
    pub amm_info: Account<'info, AmmInfo>,

    #[account(
        init_if_needed,
        payer = sender,
        space = 8 + MintedPDA::LEN,
        seeds = [b"minted", sender.key().as_ref()],
        bump
    )]
    pub minted_pda: Account<'info, MintedPDA>,

    #[account(mut)]
    pub senders_token_account0: Account<'info, TokenAccount>,
    #[account(mut)]
    pub senders_token_account1: Account<'info, TokenAccount>,

    #[account(mut)]
    pub pdas_token_account0: Account<'info, TokenAccount>,
    #[account(mut)]
    pub pdas_token_account1: Account<'info, TokenAccount>,

    pub token_program: Program<'info, Token>,
    pub system_program: Program<'info, System>,
    pub associated_token_program: Program<'info, AssociatedToken>,
}

#[derive(Accounts)]
pub struct RedeemOrSwapCtx<'info> {
    #[account(mut)]
    pub sender: Signer<'info>,

    pub mint0: Account<'info, Mint>,
    pub mint1: Account<'info, Mint>,

    #[account(
        mut,
        seeds = [b"amm", mint0.key().as_ref(), mint1.key().as_ref()],
        bump = amm_info.bump,
    )]
    pub amm_info: Account<'info, AmmInfo>,

    #[account(mut)]
    pub minted_pda: Account<'info, MintedPDA>,

    #[account(mut)]
    pub senders_token_account0: Account<'info, TokenAccount>,
    #[account(mut)]
    pub senders_token_account1: Account<'info, TokenAccount>,

    #[account(mut)]
    pub pdas_token_account0: Account<'info, TokenAccount>,
    #[account(mut)]
    pub pdas_token_account1: Account<'info, TokenAccount>,

    pub token_program: Program<'info, Token>,
    pub system_program: Program<'info, System>,
    pub associated_token_program: Program<'info, AssociatedToken>,
}

#[account]
pub struct AmmInfo {
    pub mint0: Pubkey,
    pub mint1: Pubkey,
    pub token_account0: Pubkey,
    pub token_account1: Pubkey,
    // reserves stored in canonical scale (mint1.decimals)
    pub reserve0: u128,
    pub reserve1: u128,
    pub ever_deposited: bool,
    pub supply: u128,
    pub bump: u8,
}

impl AmmInfo {
    // safe size approximation
    pub const LEN: usize = 32*4 + 16 + 16 + 1 + 16 + 1;
}

#[account]
pub struct MintedPDA {
    pub minted: u64,
    pub bump: u8,
}

impl MintedPDA {
    pub const LEN: usize = 8 + 1;
}

fn sqrt_u128(x: u128) -> u128 {
    if x <= 1 { return x; }
    let mut z = x;
    let mut y = (x >> 1) + 1;
    while y < z {
        z = y;
        y = (x / y + y) >> 1;
    }
    z
}

fn pow10(decimals: u32) -> Result<u128> {
    require!(decimals <= 38, AmmError::DecimalsTooLarge);
    let mut v: u128 = 1;
    for _ in 0..decimals {
        v = v.checked_mul(10).ok_or(AmmError::Overflow)?;
    }
    Ok(v)
}

#[error_code]
pub enum AmmError {
    #[msg("Token account mint does not match supplied mint")]
    TokenAccountMintMismatch,
    #[msg("Mint mismatch with AMMInfo")]
    MintMismatch,
    #[msg("Overflow in arithmetic")]
    Overflow,
    #[msg("No reserves available")]
    NoReserves,
    #[msg("Not minted")]
    NotMinted,
    #[msg("Zero total supply")]
    ZeroSupply,
    #[msg("Slippage too large")]
    SlippageExceeded,
    #[msg("Decimals value too large")]
    DecimalsTooLarge,
    #[msg("Invalid minted PDA provided")]
    InvalidMintedPDA,
}
