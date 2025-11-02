use anchor_lang::prelude::*;
use anchor_spl::associated_token::AssociatedToken;
use anchor_spl::token::{self, Mint, SetAuthority, Token, TokenAccount, Transfer};
use anchor_spl::token::spl_token;

declare_id!("JUj8h2LS831VNTKr9McFmAofqRTmoJT7675zpXShxnW");

#[program]
pub mod constant_product_amm {
    use super::*;

    pub fn initialize(ctx: Context<InitializeCtx>) -> Result<()> {
        let amm = &mut ctx.accounts.amm_info;

        amm.mint0 = ctx.accounts.mint0.key();
        amm.mint1 = ctx.accounts.mint1.key();
        amm.token_account0 = ctx.accounts.token_account0.key();
        amm.token_account1 = ctx.accounts.token_account1.key();
        amm.reserve0 = 0;
        amm.reserve1 = 0;
        amm.ever_deposited = false;
        amm.supply = 0;

        // Transfer ownership of the provided token accounts to the AMM PDA
        let amm_pda_key = ctx.accounts.amm_info.to_account_info().key();

        token::set_authority(
            CpiContext::new(
                ctx.accounts.token_program.to_account_info(),
                SetAuthority {
                    account_or_mint: ctx.accounts.token_account0.to_account_info(),
                    current_authority: ctx.accounts.initializer.to_account_info(),
                },
            ),
            spl_token::instruction::AuthorityType::AccountOwner,
            Some(amm_pda_key),
        )?;

        token::set_authority(
            CpiContext::new(
                ctx.accounts.token_program.to_account_info(),
                SetAuthority {
                    account_or_mint: ctx.accounts.token_account1.to_account_info(),
                    current_authority: ctx.accounts.initializer.to_account_info(),
                },
            ),
            spl_token::instruction::AuthorityType::AccountOwner,
            Some(amm_pda_key),
        )?;

        Ok(())
    }

    // Replaced deposit handler
    // Corrected deposit handler:
    // - scales incoming human amounts into base units using mint.decimals
    // - performs transfers using scaled base-unit amounts
    // - initial LP minted = sqrt(amount0_base * amount1_base)
    // - subsequent LP minted computed proportionally
    pub fn deposit(ctx: Context<DepositCtx>, amount0: u64, amount1: u64) -> Result<()> {
        // read decimals and compute scales
        let dec0 = ctx.accounts.mint0.decimals;
        let dec1 = ctx.accounts.mint1.decimals;
        let scale0 = decimals_to_scale(dec0);
        let scale1 = decimals_to_scale(dec1);

        // scale user-supplied amounts (interpreted as "human" amounts in tests) into base units
        // careful: multiply as u128 then try_into
        let amount0_base_u128 = (amount0 as u128)
            .checked_mul(scale0)
            .ok_or(ErrorCode::Overflow)?;
        let amount1_base_u128 = (amount1 as u128)
            .checked_mul(scale1)
            .ok_or(ErrorCode::Overflow)?;
        let amount0_base: u64 = amount0_base_u128.try_into().map_err(|_| ErrorCode::Overflow)?;
        let amount1_base: u64 = amount1_base_u128.try_into().map_err(|_| ErrorCode::Overflow)?;

        // Transfer token0 from sender to AMM token account0 (scaled)
        token::transfer(
            CpiContext::new(
                ctx.accounts.token_program.to_account_info(),
                Transfer {
                    from: ctx.accounts.senders_token_account0.to_account_info(),
                    to: ctx.accounts.pdas_token_account0.to_account_info(),
                    authority: ctx.accounts.sender.to_account_info(),
                },
            ),
            amount0_base,
        )?;

        // Transfer token1 from sender to AMM token account1 (scaled)
        token::transfer(
            CpiContext::new(
                ctx.accounts.token_program.to_account_info(),
                Transfer {
                    from: ctx.accounts.senders_token_account1.to_account_info(),
                    to: ctx.accounts.pdas_token_account1.to_account_info(),
                    authority: ctx.accounts.sender.to_account_info(),
                },
            ),
            amount1_base,
        )?;

        // Compute minted LP accounting in base units
        let minted_amount: u64;
        let amm = &mut ctx.accounts.amm_info;
        let minted = &mut ctx.accounts.minted_pda;

        if !amm.ever_deposited {
            // canonical initial LP minted = floor(sqrt(amount0_base * amount1_base))
            let prod = (amount0_base as u128)
                .checked_mul(amount1_base as u128)
                .ok_or(ErrorCode::Overflow)?;
            let minted_u64 = integer_sqrt(prod);
            require!(minted_u64 > 0, ErrorCode::InvalidAmount);

            minted_amount = minted_u64;
            amm.ever_deposited = true;
            amm.reserve0 = amm.reserve0.checked_add(amount0_base).ok_or(ErrorCode::Overflow)?;
            amm.reserve1 = amm.reserve1.checked_add(amount1_base).ok_or(ErrorCode::Overflow)?;
            amm.supply = amm.supply.checked_add(minted_amount).ok_or(ErrorCode::Overflow)?;
        } else {
            require!(amm.reserve0 > 0 && amm.reserve1 > 0, ErrorCode::InvalidReserve);

            let supply = amm.supply as u128;
            let r0 = amm.reserve0 as u128;
            let r1 = amm.reserve1 as u128;

            let m0 = (amount0_base as u128)
                .checked_mul(supply)
                .ok_or(ErrorCode::Overflow)?
                .checked_div(r0)
                .ok_or(ErrorCode::DivisionByZero)?;
            let m1 = (amount1_base as u128)
                .checked_mul(supply)
                .ok_or(ErrorCode::Overflow)?
                .checked_div(r1)
                .ok_or(ErrorCode::DivisionByZero)?;

            let minted_u128 = core::cmp::min(m0, m1);
            minted_amount = minted_u128.try_into().map_err(|_| ErrorCode::Overflow)?;

            amm.reserve0 = amm.reserve0.checked_add(amount0_base).ok_or(ErrorCode::Overflow)?;
            amm.reserve1 = amm.reserve1.checked_add(amount1_base).ok_or(ErrorCode::Overflow)?;
            amm.supply = amm.supply.checked_add(minted_amount).ok_or(ErrorCode::Overflow)?;
        }

        // Update minted PDA accounting (this is LP accounting in minted base units)
        minted.minted = minted.minted.checked_add(minted_amount).ok_or(ErrorCode::Overflow)?;

        Ok(())
    }

    // Corrected redeem handler:
    // - expects `amount` param to be LP tokens in the same LP supply units
    // - computes outs in base units and transfers scaled base-unit tokens back
    pub fn redeem(ctx: Context<RedeemOrSwapCtx>, amount: u64) -> Result<()> {
        // read immutable values early to avoid borrow conflicts
        let mint0 = ctx.accounts.amm_info.mint0;
        let mint1 = ctx.accounts.amm_info.mint1;
        let amm_bump = ctx.bumps.amm_info;

        // compute outs and update reserves inside short mutable scope
        let (out0_base, out1_base) = {
            let amm = &mut ctx.accounts.amm_info;
            let minted = &mut ctx.accounts.minted_pda;

            require!(amount > 0, ErrorCode::InvalidAmount);
            require!(amm.supply > 0, ErrorCode::InvalidSupply);
            require!(minted.minted >= amount, ErrorCode::InsufficientMinted);

            let supply = amm.supply as u128;
            let out0_u128 = (amount as u128)
                .checked_mul(amm.reserve0 as u128)
                .ok_or(ErrorCode::Overflow)?
                .checked_div(supply)
                .ok_or(ErrorCode::DivisionByZero)?;
            let out1_u128 = (amount as u128)
                .checked_mul(amm.reserve1 as u128)
                .ok_or(ErrorCode::Overflow)?
                .checked_div(supply)
                .ok_or(ErrorCode::DivisionByZero)?;

            let out0: u64 = out0_u128.try_into().map_err(|_| ErrorCode::Overflow)?;
            let out1: u64 = out1_u128.try_into().map_err(|_| ErrorCode::Overflow)?;

            amm.reserve0 = amm.reserve0.checked_sub(out0).ok_or(ErrorCode::Overflow)?;
            amm.reserve1 = amm.reserve1.checked_sub(out1).ok_or(ErrorCode::Overflow)?;
            amm.supply = amm.supply.checked_sub(amount).ok_or(ErrorCode::Overflow)?;
            minted.minted = minted.minted.checked_sub(amount).ok_or(ErrorCode::Overflow)?;

            (out0, out1)
        };

        // signer seeds built from constants read earlier (mint0/mint1) and bump
        let seeds = &[b"amm", mint0.as_ref(), mint1.as_ref(), &[amm_bump]];
        let signer = &[&seeds[..]];

        // Transfer token0 base units from AMM to sender
        token::transfer(
            CpiContext::new_with_signer(
                ctx.accounts.token_program.to_account_info(),
                Transfer {
                    from: ctx.accounts.pdas_token_account0.to_account_info(),
                    to: ctx.accounts.senders_token_account0.to_account_info(),
                    authority: ctx.accounts.amm_info.to_account_info(),
                },
                signer,
            ),
            out0_base,
        )?;

        // Transfer token1 base units from AMM to sender
        token::transfer(
            CpiContext::new_with_signer(
                ctx.accounts.token_program.to_account_info(),
                Transfer {
                    from: ctx.accounts.pdas_token_account1.to_account_info(),
                    to: ctx.accounts.senders_token_account1.to_account_info(),
                    authority: ctx.accounts.amm_info.to_account_info(),
                },
                signer,
            ),
            out1_base,
        )?;

        Ok(())
    }

    // Corrected swap handler:
    // - scales amount_in from human units to base units using mint decimals
    // - performs input transfer with base units
    // - computes amount_out in base units using constant-product + fee (997/1000)
    // - updates reserves in base units and performs AMM-signed output transfer
    pub fn swap(
        ctx: Context<RedeemOrSwapCtx>,
        is_mint0: bool,
        amount_in: u64,
        min_out_amount: u64,
    ) -> Result<()> {
        // read decimals and scales
        let dec0 = ctx.accounts.mint0.decimals;
        let dec1 = ctx.accounts.mint1.decimals;
        let scale0 = decimals_to_scale(dec0);
        let scale1 = decimals_to_scale(dec1);

        // scale amount_in and min_out_amount from test "human" units to base units accordingly
        let amount_in_base: u64;
        let min_out_base: u64;
        if is_mint0 {
            amount_in_base = (amount_in as u128)
                .checked_mul(scale0)
                .ok_or(ErrorCode::Overflow)?
                .try_into()
                .map_err(|_| ErrorCode::Overflow)?;
            min_out_base = (min_out_amount as u128)
                .checked_mul(scale1)
                .ok_or(ErrorCode::Overflow)?
                .try_into()
                .map_err(|_| ErrorCode::Overflow)?;
        } else {
            amount_in_base = (amount_in as u128)
                .checked_mul(scale1)
                .ok_or(ErrorCode::Overflow)?
                .try_into()
                .map_err(|_| ErrorCode::Overflow)?;
            min_out_base = (min_out_amount as u128)
                .checked_mul(scale0)
                .ok_or(ErrorCode::Overflow)?
                .try_into()
                .map_err(|_| ErrorCode::Overflow)?;
        }

        require!(amount_in_base > 0, ErrorCode::InvalidAmount);

        // transfer input into AMM (base units)
        if is_mint0 {
            token::transfer(
                CpiContext::new(
                    ctx.accounts.token_program.to_account_info(),
                    Transfer {
                        from: ctx.accounts.senders_token_account0.to_account_info(),
                        to: ctx.accounts.pdas_token_account0.to_account_info(),
                        authority: ctx.accounts.sender.to_account_info(),
                    },
                ),
                amount_in_base,
            )?;
        } else {
            token::transfer(
                CpiContext::new(
                    ctx.accounts.token_program.to_account_info(),
                    Transfer {
                        from: ctx.accounts.senders_token_account1.to_account_info(),
                        to: ctx.accounts.pdas_token_account1.to_account_info(),
                        authority: ctx.accounts.sender.to_account_info(),
                    },
                ),
                amount_in_base,
            )?;
        }

        // read immutable mint pubkeys and bump for signer construction
        let mint0_pk = ctx.accounts.amm_info.mint0;
        let mint1_pk = ctx.accounts.amm_info.mint1;
        let amm_bump = ctx.bumps.amm_info;

        // compute amount_out and update reserves inside mutable scope
        let amount_out_base = {
            let amm = &mut ctx.accounts.amm_info;

            let (reserve_in, reserve_out) = if is_mint0 {
                (amm.reserve0 as u128, amm.reserve1 as u128)
            } else {
                (amm.reserve1 as u128, amm.reserve0 as u128)
            };

            require!(reserve_in > 0 && reserve_out > 0, ErrorCode::InvalidReserve);

            // fee applied: 0.3% -> keep numerator 997/1000
            let amount_in_with_fee = (amount_in_base as u128)
                .checked_mul(997)
                .ok_or(ErrorCode::Overflow)?
                .checked_div(1000)
                .ok_or(ErrorCode::DivisionByZero)?;

            let numerator = reserve_out.checked_mul(amount_in_with_fee).ok_or(ErrorCode::Overflow)?;
            let denominator = reserve_in.checked_add(amount_in_with_fee).ok_or(ErrorCode::Overflow)?;
            let amount_out_u128 = numerator.checked_div(denominator).ok_or(ErrorCode::DivisionByZero)?;
            let amount_out: u64 = amount_out_u128.try_into().map_err(|_| ErrorCode::Overflow)?;
            require!(amount_out >= min_out_base, ErrorCode::SlippageExceeded);

            if is_mint0 {
                amm.reserve0 = amm.reserve0.checked_add(amount_in_base).ok_or(ErrorCode::Overflow)?;
                amm.reserve1 = amm.reserve1.checked_sub(amount_out).ok_or(ErrorCode::Overflow)?;
            } else {
                amm.reserve1 = amm.reserve1.checked_add(amount_in_base).ok_or(ErrorCode::Overflow)?;
                amm.reserve0 = amm.reserve0.checked_sub(amount_out).ok_or(ErrorCode::Overflow)?;
            }

            amount_out
        };

        // signer seeds using amm mint pubkeys and bump
        let seeds = &[b"amm", mint0_pk.as_ref(), mint1_pk.as_ref(), &[amm_bump]];
        let signer = &[&seeds[..]];

        // transfer output base units from AMM to sender
        if is_mint0 {
            // output is token1
            token::transfer(
                CpiContext::new_with_signer(
                    ctx.accounts.token_program.to_account_info(),
                    Transfer {
                        from: ctx.accounts.pdas_token_account1.to_account_info(),
                        to: ctx.accounts.senders_token_account1.to_account_info(),
                        authority: ctx.accounts.amm_info.to_account_info(),
                    },
                    signer,
                ),
                amount_out_base,
            )?;
        } else {
            // output is token0
            token::transfer(
                CpiContext::new_with_signer(
                    ctx.accounts.token_program.to_account_info(),
                    Transfer {
                        from: ctx.accounts.pdas_token_account0.to_account_info(),
                        to: ctx.accounts.senders_token_account0.to_account_info(),
                        authority: ctx.accounts.amm_info.to_account_info(),
                    },
                    signer,
                ),
                amount_out_base,
            )?;
        }

        Ok(())
    }
}

// integer sqrt helper (u128 -> u64)
fn integer_sqrt(x: u128) -> u64 {
    if x == 0 {
        return 0;
    }
    let mut z = x;
    let mut y = (x + 1) >> 1;
    while y < z {
        z = y;
        y = (x / y + y) >> 1;
    }
    z as u64
}

// Helper: 10^decimals as u128
fn decimals_to_scale(decimals: u8) -> u128 {
    let mut s: u128 = 1;
    for _ in 0..decimals {
        s = s.checked_mul(10).unwrap();
    }
    s
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
}

#[account]
pub struct MintedPDA {
    pub minted: u64,
}

#[derive(Accounts)]
pub struct InitializeCtx<'info> {
    #[account(mut)]
    pub initializer: Signer<'info>,

    /// AmmInfo PDA created by seeds [b"amm", mint0, mint1]
    #[account(
        init,
        payer = initializer,
        space = 8 + 32 + 32 + 32 + 32 + 8 + 8 + 1 + 8,
        seeds = [b"amm", mint0.key().as_ref(), mint1.key().as_ref()],
        bump
    )]
    pub amm_info: Account<'info, AmmInfo>,

    pub mint0: Account<'info, Mint>,
    pub mint1: Account<'info, Mint>,

    /// These token accounts are existing token accounts; owner will be set to AMM PDA
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

    /// AmmInfo PDA
    #[account(
        mut,
        seeds = [b"amm", mint0.key().as_ref(), mint1.key().as_ref()],
        bump
    )]
    pub amm_info: Account<'info, AmmInfo>,

    /// MintedPDA per-sender (init if needed). `init_if_needed` requires `mut`.
    #[account(
        init_if_needed,
        payer = sender,
        space = 8 + 8,
        seeds = [b"minted", sender.key().as_ref()],
        bump
    )]
    pub minted_pda: Account<'info, MintedPDA>,

    /// Sender's token accounts
    #[account(mut)]
    pub senders_token_account0: Account<'info, TokenAccount>,
    #[account(mut)]
    pub senders_token_account1: Account<'info, TokenAccount>,

    /// AMM's token accounts (owned by AMM PDA)
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

    /// AmmInfo PDA (mutable)
    #[account(
        mut,
        seeds = [b"amm", mint0.key().as_ref(), mint1.key().as_ref()],
        bump
    )]
    pub amm_info: Account<'info, AmmInfo>,

    /// MintedPDA for sender
    #[account(
        mut,
        seeds = [b"minted", sender.key().as_ref()],
        bump
    )]
    pub minted_pda: Account<'info, MintedPDA>,

    /// Sender's token accounts
    #[account(mut)]
    pub senders_token_account0: Account<'info, TokenAccount>,
    #[account(mut)]
    pub senders_token_account1: Account<'info, TokenAccount>,

    /// AMM's token accounts (owned by AMM PDA)
    #[account(mut)]
    pub pdas_token_account0: Account<'info, TokenAccount>,
    #[account(mut)]
    pub pdas_token_account1: Account<'info, TokenAccount>,

    pub token_program: Program<'info, Token>,
    pub system_program: Program<'info, System>,
    pub associated_token_program: Program<'info, AssociatedToken>,
}

#[error_code]
pub enum ErrorCode {
    #[msg("Overflow occurred")]
    Overflow,
    #[msg("Division by zero")]
    DivisionByZero,
    #[msg("Invalid reserve state")]
    InvalidReserve,
    #[msg("Invalid amount")]
    InvalidAmount,
    #[msg("Insufficient minted balance")]
    InsufficientMinted,
    #[msg("Slippage exceeded")]
    SlippageExceeded,
    #[msg("Invalid supply")]
    InvalidSupply,
    #[msg("Invalid bump")]
    InvalidBump,
}
