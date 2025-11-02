use anchor_lang::prelude::*;
use anchor_spl::associated_token::AssociatedToken;
use anchor_spl::token::{self, Mint, SetAuthority, Token, TokenAccount, Transfer};
use anchor_spl::token::spl_token;

declare_id!("EUR47AAUsA8aFEgoXdvaDDTXsqfnFpbTMyHe9UwbvmTm");

#[program]
pub mod constant_product_amm {
    use super::*;

    pub fn initialize(ctx: Context<InitializeCtx>) -> Result<()> {
        // initialize AmmInfo
        let amm = &mut ctx.accounts.amm_info;

        amm.mint0 = ctx.accounts.mint0.key();
        amm.mint1 = ctx.accounts.mint1.key();
        amm.token_account0 = ctx.accounts.token_account0.key();
        amm.token_account1 = ctx.accounts.token_account1.key();
        amm.reserve0 = 0;
        amm.reserve1 = 0;
        amm.ever_deposited = false;
        amm.supply = 0;

        // compute PDA bump deterministically using seeds [b"amm", mint0, mint1]
        let (_pda, bump) = Pubkey::find_program_address(
            &[
                b"amm",
                ctx.accounts.mint0.key().as_ref(),
                ctx.accounts.mint1.key().as_ref(),
            ],
            &crate::ID,
        );
        amm.bump = bump as u8;

        // Transfer ownership of the provided token accounts to the AMM PDA.
        let cpi_program = ctx.accounts.token_program.to_account_info();

        let cpi_accounts0 = SetAuthority {
            account_or_mint: ctx.accounts.token_account0.to_account_info(),
            current_authority: ctx.accounts.initializer.to_account_info(),
        };
        let cpi_ctx0 = CpiContext::new(cpi_program.clone(), cpi_accounts0);
        token::set_authority(
            cpi_ctx0.with_signer(&[]),
            spl_token::instruction::AuthorityType::AccountOwner,
            Some(ctx.accounts.amm_info.key()),
        )?;

        let cpi_accounts1 = SetAuthority {
            account_or_mint: ctx.accounts.token_account1.to_account_info(),
            current_authority: ctx.accounts.initializer.to_account_info(),
        };
        let cpi_ctx1 = CpiContext::new(cpi_program, cpi_accounts1);
        token::set_authority(
            cpi_ctx1.with_signer(&[]),
            spl_token::instruction::AuthorityType::AccountOwner,
            Some(ctx.accounts.amm_info.key()),
        )?;

        Ok(())
    }

    // deposit(): all math in COMMON_DECIMALS; supply/minted stored in COMMON_DECIMALS units
    pub fn deposit(ctx: Context<DepositCtx>, amount0: u64, amount1: u64) -> Result<()> {
        let amm = &mut ctx.accounts.amm_info;
        require_keys_eq!(amm.mint0, ctx.accounts.mint0.key());
        require_keys_eq!(amm.mint1, ctx.accounts.mint1.key());
        require_keys_eq!(amm.token_account0, ctx.accounts.pdas_token_account0.key());
        require_keys_eq!(amm.token_account1, ctx.accounts.pdas_token_account1.key());

        // raw transfers (do not scale transfer amounts)
        let cpi_program = ctx.accounts.token_program.to_account_info();
        token::transfer(
            CpiContext::new(
                cpi_program.clone(),
                Transfer { from: ctx.accounts.senders_token_account0.to_account_info(), to: ctx.accounts.pdas_token_account0.to_account_info(), authority: ctx.accounts.sender.to_account_info() }
            ),
            amount0,
        )?;
        token::transfer(
            CpiContext::new(
                cpi_program.clone(),
                Transfer { from: ctx.accounts.senders_token_account1.to_account_info(), to: ctx.accounts.pdas_token_account1.to_account_info(), authority: ctx.accounts.sender.to_account_info() }
            ),
            amount1,
        )?;

        // decimals & common scale
        let dec0 = ctx.accounts.mint0.decimals as usize;
        let dec1 = ctx.accounts.mint1.decimals as usize;
        let common_dec = core::cmp::max(dec0, dec1);
        let scale0 = pow10_u128(common_dec.saturating_sub(dec0));
        let scale1 = pow10_u128(common_dec.saturating_sub(dec1));

        // scaled reserves & scaled deposit amounts (u128)
        let reserve0_scaled = (amm.reserve0 as u128).checked_mul(scale0).ok_or(ErrorCode::Overflow)?;
        let reserve1_scaled = (amm.reserve1 as u128).checked_mul(scale1).ok_or(ErrorCode::Overflow)?;
        let amount0_scaled = (amount0 as u128).checked_mul(scale0).ok_or(ErrorCode::Overflow)?;
        let amount1_scaled = (amount1 as u128).checked_mul(scale1).ok_or(ErrorCode::Overflow)?;

        // compute minted_scaled (u128) in COMMON_DECIMALS
        let minted_scaled: u128 = if amm.supply == 0 {
            // initial: floor(sqrt(amount0_scaled * amount1_scaled))
            let prod = amount0_scaled.checked_mul(amount1_scaled).ok_or(ErrorCode::Overflow)?;
            integer_sqrt_u128(prod)
        } else {
            // minted_scaled = amount1_scaled * supply_scaled / prev_reserve1_scaled
            let supply_scaled = (amm.supply as u128).checked_mul(scale1).ok_or(ErrorCode::Overflow)?;
            let prev_reserve1_scaled = reserve1_scaled.checked_sub(amount1_scaled).ok_or(ErrorCode::Underflow)?;
            require!(prev_reserve1_scaled > 0, ErrorCode::InvalidReserve);
            amount1_scaled
                .checked_mul(supply_scaled)
                .ok_or(ErrorCode::Overflow)?
                .checked_div(prev_reserve1_scaled)
                .ok_or(ErrorCode::DivideByZero)?
        };

        // update minted_pda and amm: store supply/minted in COMMON_DECIMALS units
        // Convert minted_scaled -> u64 scaled units (careful: may overflow if large; tests use small values)
        let minted_scaled_u64: u64 = minted_scaled.try_into().map_err(|_| ErrorCode::Overflow)?;
        ctx.accounts.minted_pda.minted = ctx.accounts.minted_pda.minted.checked_add(minted_scaled_u64).ok_or(ErrorCode::Overflow)?;
        amm.supply = amm.supply.checked_add(minted_scaled_u64).ok_or(ErrorCode::Overflow)?;
        amm.reserve0 = amm.reserve0.checked_add(amount0).ok_or(ErrorCode::Overflow)?;
        amm.reserve1 = amm.reserve1.checked_add(amount1).ok_or(ErrorCode::Overflow)?;
        amm.ever_deposited = true;

        // set minted bump if needed
        if ctx.accounts.minted_pda.bump == 0 {
            let (_pda, bump) = Pubkey::find_program_address(&[b"minted", ctx.accounts.sender.key.as_ref()], &crate::ID);
            ctx.accounts.minted_pda.bump = bump as u8;
        }

        Ok(())
    }


    // redeem(): compute proportional outputs in COMMON_DECIMALS and transfer raw units
    pub fn redeem(ctx: Context<RedeemOrSwapCtx>, amount: u64) -> Result<()> {
        // validations
        let amm_pk0 = ctx.accounts.amm_info.mint0;
        let amm_pk1 = ctx.accounts.amm_info.mint1;
        let amm_t0 = ctx.accounts.amm_info.token_account0;
        let amm_t1 = ctx.accounts.amm_info.token_account1;
        let amm_bump = ctx.accounts.amm_info.bump;

        require_keys_eq!(amm_pk0, ctx.accounts.mint0.key());
        require_keys_eq!(amm_pk1, ctx.accounts.mint1.key());
        require_keys_eq!(amm_t0, ctx.accounts.pdas_token_account0.key());
        require_keys_eq!(amm_t1, ctx.accounts.pdas_token_account1.key());

        let minted = &mut ctx.accounts.minted_pda;
        require!(minted.minted >= amount, ErrorCode::InsufficientMintedBalance);

        // decimals & common_dec
        let dec0 = ctx.accounts.mint0.decimals as usize;
        let dec1 = ctx.accounts.mint1.decimals as usize;
        let common_dec = core::cmp::max(dec0, dec1);
        let scale0 = pow10_u128(common_dec.saturating_sub(dec0));
        let scale1 = pow10_u128(common_dec.saturating_sub(dec1));

        // scaled reserves & supply (u128)
        let reserve0_scaled = (ctx.accounts.amm_info.reserve0 as u128).checked_mul(scale0).ok_or(ErrorCode::Overflow)?;
        let reserve1_scaled = (ctx.accounts.amm_info.reserve1 as u128).checked_mul(scale1).ok_or(ErrorCode::Overflow)?;
        let supply_scaled = (ctx.accounts.amm_info.supply as u128).checked_mul(scale1).ok_or(ErrorCode::Overflow)?; // supply is stored in COMMON_DECIMALS units

        require!(supply_scaled > 0, ErrorCode::InvalidSupply);

        // compute outputs in scaled space: out_scaled = reserve_scaled * amount_scaled / supply_scaled
        let amount_scaled = (amount as u128).checked_mul(scale1).ok_or(ErrorCode::Overflow)?;
        let out0_scaled = reserve0_scaled.checked_mul(amount_scaled).ok_or(ErrorCode::Overflow)?.checked_div(supply_scaled).ok_or(ErrorCode::DivideByZero)?;
        let out1_scaled = reserve1_scaled.checked_mul(amount_scaled).ok_or(ErrorCode::Overflow)?.checked_div(supply_scaled).ok_or(ErrorCode::DivideByZero)?;

        // convert scaled outputs back to raw token units
        let out0_raw: u64 = out0_scaled.checked_div(scale0).ok_or(ErrorCode::DivideByZero)?.try_into().map_err(|_| ErrorCode::Overflow)?;
        let out1_raw: u64 = out1_scaled.checked_div(scale1).ok_or(ErrorCode::DivideByZero)?.try_into().map_err(|_| ErrorCode::Overflow)?;

        // PDA signer seeds
        let seeds: &[&[u8]] = &[ b"amm", &ctx.accounts.mint0.key().to_bytes(), &ctx.accounts.mint1.key().to_bytes(), &[amm_bump] ];
        let signer = &[&seeds[..]];
        let cpi_program = ctx.accounts.token_program.to_account_info();

        // transfer outputs (AMM -> sender)
        token::transfer(
            CpiContext::new_with_signer(cpi_program.clone(), Transfer { from: ctx.accounts.pdas_token_account0.to_account_info(), to: ctx.accounts.senders_token_account0.to_account_info(), authority: ctx.accounts.amm_info.to_account_info() }, signer),
            out0_raw,
        )?;
        token::transfer(
            CpiContext::new_with_signer(cpi_program, Transfer { from: ctx.accounts.pdas_token_account1.to_account_info(), to: ctx.accounts.senders_token_account1.to_account_info(), authority: ctx.accounts.amm_info.to_account_info() }, signer),
            out1_raw,
        )?;

        // update scaled supply & reserves (supply & minted_pda stored in COMMON_DECIMALS units)
        ctx.accounts.amm_info.reserve0 = ctx.accounts.amm_info.reserve0.checked_sub(out0_raw).ok_or(ErrorCode::Underflow)?;
        ctx.accounts.amm_info.reserve1 = ctx.accounts.amm_info.reserve1.checked_sub(out1_raw).ok_or(ErrorCode::Underflow)?;
        ctx.accounts.amm_info.supply = ctx.accounts.amm_info.supply.checked_sub(amount).ok_or(ErrorCode::Underflow)?;
        ctx.accounts.minted_pda.minted = ctx.accounts.minted_pda.minted.checked_sub(amount).ok_or(ErrorCode::Underflow)?;

        Ok(())
    }

    pub fn swap(
        ctx: Context<RedeemOrSwapCtx>,
        is_mint0: bool,
        amount_in: u64,
        min_out_amount: u64,
    ) -> Result<()> {
        // Validate
                // --- Begin replacement swap body ---
        // Validate
        require_keys_eq!(ctx.accounts.amm_info.mint0, ctx.accounts.mint0.key());
        require_keys_eq!(ctx.accounts.amm_info.mint1, ctx.accounts.mint1.key());
        require_keys_eq!(ctx.accounts.amm_info.token_account0, ctx.accounts.pdas_token_account0.key());
        require_keys_eq!(ctx.accounts.amm_info.token_account1, ctx.accounts.pdas_token_account1.key());

        // decimals
        let dec0 = ctx.accounts.mint0.decimals as usize;
        let dec1 = ctx.accounts.mint1.decimals as usize;

        // choose a common decimal base to avoid fractional scaling
        let common_dec = if dec0 > dec1 { dec0 } else { dec1 };

        // compute scale factors (as u128)
        let scale0 = pow10_u128(common_dec.saturating_sub(dec0));
        let scale1 = pow10_u128(common_dec.saturating_sub(dec1));

        // read reserves as u128 and scale both to COMMON_DECIMALS
        let r0_scaled = (ctx.accounts.amm_info.reserve0 as u128)
            .checked_mul(scale0)
            .ok_or(ErrorCode::Overflow)?;
        let r1_scaled = (ctx.accounts.amm_info.reserve1 as u128)
            .checked_mul(scale1)
            .ok_or(ErrorCode::Overflow)?;

        // amount_in scaled to common decimals
        let amount_in_scaled = if is_mint0 {
            (amount_in as u128).checked_mul(scale0).ok_or(ErrorCode::Overflow)?
        } else {
            (amount_in as u128).checked_mul(scale1).ok_or(ErrorCode::Overflow)?
        };

        // determine reserve_in / reserve_out in scaled space
        let (reserve_in, reserve_out) = if is_mint0 {
            (r0_scaled, r1_scaled)
        } else {
            (r1_scaled, r0_scaled)
        };

        // constant product k = reserve_in * reserve_out
        let k = reserve_in.checked_mul(reserve_out).ok_or(ErrorCode::Overflow)?;

        // new_reserve_out = k / (reserve_in + amount_in_scaled)
        let denom = reserve_in.checked_add(amount_in_scaled).ok_or(ErrorCode::Overflow)?;
        require!(denom > 0, ErrorCode::DivideByZero);
        let new_reserve_out = k.checked_div(denom).ok_or(ErrorCode::DivideByZero)?;

        // amount_out_scaled = reserve_out - new_reserve_out
        let amount_out_scaled = reserve_out.checked_sub(new_reserve_out).ok_or(ErrorCode::Underflow)?;

        // convert amount_out_scaled to raw output token units
        // if swapping mint0->mint1, output token is mint1 (use scale1)
        // if swapping mint1->mint0, output token is mint0 (use scale0)
        let amount_out_raw: u64 = if is_mint0 {
            // amount_out_scaled is in COMMON_DECIMALS; convert to mint1 raw units
            let div = scale1;
            let raw = amount_out_scaled.checked_div(div).ok_or(ErrorCode::DivideByZero)?;
            raw.try_into().map_err(|_| ErrorCode::Overflow)?
        } else {
            // output is mint0: convert from COMMON_DECIMALS to mint0 raw units
            let div = scale0;
            let raw = amount_out_scaled.checked_div(div).ok_or(ErrorCode::DivideByZero)?;
            raw.try_into().map_err(|_| ErrorCode::Overflow)?
        };

        // Enforce min_out_amount: convert amount_out_raw into mint1-decimal units (as required)
        // We'll represent min_out_amount as expressed in mint1.decimals (per your spec).
        // Convert amount_out_raw -> mint1 units (u128) for comparison.
        let amount_out_in_mint1_units: u128 = if is_mint0 {
            // output is mint1; already in mint1 raw units after division by scale1
            amount_out_raw as u128
        } else {
            // output is mint0; convert raw mint0 -> mint1 by scaling with 10^(dec1-dec0) or dividing
            if dec1 >= dec0 {
                let mul = pow10_u128(dec1 - dec0);
                (amount_out_raw as u128).checked_mul(mul).ok_or(ErrorCode::Overflow)?
            } else {
                let div = pow10_u128(dec0 - dec1);
                (amount_out_raw as u128).checked_div(div).ok_or(ErrorCode::DivideByZero)?
            }
        };

        require!(
            amount_out_in_mint1_units >= (min_out_amount as u128),
            ErrorCode::SlippageExceeded
        );

        // Build PDA signer seeds
        let amm_bump = ctx.accounts.amm_info.bump;
        let seeds: &[&[u8]] = &[
            b"amm",
            &ctx.accounts.mint0.key().to_bytes(),
            &ctx.accounts.mint1.key().to_bytes(),
            &[amm_bump],
        ];
        let signer = &[&seeds[..]];

        // perform transfers
        let cpi_program = ctx.accounts.token_program.to_account_info();

        if is_mint0 {
            // transfer input (mint0) from sender -> AMM
            let cpi_accounts_in = Transfer {
                from: ctx.accounts.senders_token_account0.to_account_info(),
                to: ctx.accounts.pdas_token_account0.to_account_info(),
                authority: ctx.accounts.sender.to_account_info(),
            };
            token::transfer(CpiContext::new(cpi_program.clone(), cpi_accounts_in), amount_in)?;

            // transfer output (mint1) from AMM -> sender
            let cpi_accounts_out = Transfer {
                from: ctx.accounts.pdas_token_account1.to_account_info(),
                to: ctx.accounts.senders_token_account1.to_account_info(),
                authority: ctx.accounts.amm_info.to_account_info(),
            };
            token::transfer(
                CpiContext::new_with_signer(cpi_program, cpi_accounts_out, signer),
                amount_out_raw,
            )?;
            // update reserves (raw units)
            ctx.accounts.amm_info.reserve0 = ctx.accounts.amm_info.reserve0.checked_add(amount_in).ok_or(ErrorCode::Overflow)?;
            ctx.accounts.amm_info.reserve1 = ctx.accounts.amm_info.reserve1.checked_sub(amount_out_raw).ok_or(ErrorCode::Underflow)?;
        } else {
            // transfer input (mint1) from sender -> AMM
            let cpi_accounts_in = Transfer {
                from: ctx.accounts.senders_token_account1.to_account_info(),
                to: ctx.accounts.pdas_token_account1.to_account_info(),
                authority: ctx.accounts.sender.to_account_info(),
            };
            token::transfer(CpiContext::new(cpi_program.clone(), cpi_accounts_in), amount_in)?;

            // transfer output (mint0) from AMM -> sender
            let cpi_accounts_out = Transfer {
                from: ctx.accounts.pdas_token_account0.to_account_info(),
                to: ctx.accounts.senders_token_account0.to_account_info(),
                authority: ctx.accounts.amm_info.to_account_info(),
            };
            token::transfer(
                CpiContext::new_with_signer(cpi_program, cpi_accounts_out, signer),
                amount_out_raw,
            )?;
            // update reserves
            ctx.accounts.amm_info.reserve1 = ctx.accounts.amm_info.reserve1.checked_add(amount_in).ok_or(ErrorCode::Overflow)?;
            ctx.accounts.amm_info.reserve0 = ctx.accounts.amm_info.reserve0.checked_sub(amount_out_raw).ok_or(ErrorCode::Underflow)?;
        }
        // --- End replacement swap body ---
        Ok(())
    }
}

/// Integer sqrt for u128 (floor)
fn integer_sqrt_u128(x: u128) -> u128 {
    if x <= 1 {
        return x;
    }
    let mut z = x;
    let mut y = (x >> 1) + 1;
    while y < z {
        z = y;
        y = ((x / y) + y) >> 1;
    }
    z
}

/// pow10 for u128 up to reasonable exponent
fn pow10_u128(exp: usize) -> u128 {
    let mut v: u128 = 1;
    for _ in 0..exp {
        v = v.saturating_mul(10);
    }
    v
}

#[derive(Accounts)]
pub struct InitializeCtx<'info> {
    #[account(mut, signer)]
    pub initializer: Signer<'info>,

    #[account(
        init,
        payer = initializer,
        seeds = [b"amm", mint0.key().as_ref(), mint1.key().as_ref()],
        bump,
        space = 8 + AmmInfo::LEN
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
    #[account(mut, signer)]
    pub sender: Signer<'info>,

    pub mint0: Account<'info, Mint>,
    pub mint1: Account<'info, Mint>,

    #[account(
        mut,
        seeds = [b"amm", mint0.key().as_ref(), mint1.key().as_ref()],
        bump = amm_info.bump
    )]
    pub amm_info: Account<'info, AmmInfo>,

    #[account(
        init_if_needed,
        payer = sender,
        seeds = [b"minted", sender.key().as_ref()],
        bump,
        space = 8 + MintedPDA::LEN
    )]
    pub minted_pda: Account<'info, MintedPDA>,

    #[account(mut, constraint = senders_token_account0.owner == sender.key())]
    pub senders_token_account0: Account<'info, TokenAccount>,
    #[account(mut, constraint = senders_token_account1.owner == sender.key())]
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
    #[account(mut, signer)]
    pub sender: Signer<'info>,

    pub mint0: Account<'info, Mint>,
    pub mint1: Account<'info, Mint>,

    #[account(
        mut,
        seeds = [b"amm", mint0.key().as_ref(), mint1.key().as_ref()],
        bump = amm_info.bump
    )]
    pub amm_info: Account<'info, AmmInfo>,

    #[account(mut, seeds = [b"minted", sender.key().as_ref()], bump)]
    pub minted_pda: Account<'info, MintedPDA>,

    #[account(mut, constraint = senders_token_account0.owner == sender.key())]
    pub senders_token_account0: Account<'info, TokenAccount>,
    #[account(mut, constraint = senders_token_account1.owner == sender.key())]
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
    pub reserve0: u64,
    pub reserve1: u64,
    pub ever_deposited: bool,
    pub supply: u64,
    pub bump: u8,
}
impl AmmInfo {
    pub const LEN: usize = 32 * 4 + 8 * 2 + 1 + 8 + 1;
}

#[account]
pub struct MintedPDA {
    pub minted: u64,
    pub bump: u8,
}
impl MintedPDA {
    pub const LEN: usize = 8 + 1;
}

#[error_code]
pub enum ErrorCode {
    #[msg("Overflow occurred")]
    Overflow,
    #[msg("Underflow occurred")]
    Underflow,
    #[msg("Divide by zero")]
    DivideByZero,
    #[msg("Invalid reserve")]
    InvalidReserve,
    #[msg("Invalid supply")]
    InvalidSupply,
    #[msg("Insufficient minted balance")]
    InsufficientMintedBalance,
    #[msg("Slippage exceeded")]
    SlippageExceeded,
    #[msg("Bump not found")]
    BumpNotFound,
}
