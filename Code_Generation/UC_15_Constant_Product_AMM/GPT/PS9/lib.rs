use anchor_lang::prelude::*;
use anchor_spl::associated_token::AssociatedToken;
use anchor_spl::token::{self, Mint, SetAuthority, Token, TokenAccount, Transfer};
use anchor_spl::token::spl_token;

declare_id!("xYDAqnYrFLsgmstgPFEBtYmNnhfdedJFN8Uffy2xXCK");

#[program]
pub mod constant_product_amm {
    use super::*;

    // ---------- Initialize (borrow-safe, stores bump) ----------
    pub fn initialize(ctx: Context<InitializeCtx>) -> Result<()> {
        // Validate PDA derivation using mint keys before taking mutable borrow
        let mint0_key = ctx.accounts.mint0.key();
        let mint1_key = ctx.accounts.mint1.key();
        let (pda, bump) = Pubkey::find_program_address(
            &[b"amm", mint0_key.as_ref(), mint1_key.as_ref()],
            ctx.program_id,
        );
        require_keys_eq!(pda, ctx.accounts.amm_info.key(), AmmError::InvalidAccount);

        // Transfer owner of token accounts to AMM PDA (initializer must be current owner)
        token::set_authority(
            CpiContext::new(
                ctx.accounts.token_program.to_account_info(),
                SetAuthority {
                    account_or_mint: ctx.accounts.token_account0.to_account_info(),
                    current_authority: ctx.accounts.initializer.to_account_info(),
                },
            ),
            spl_token::instruction::AuthorityType::AccountOwner,
            Some(ctx.accounts.amm_info.key()),
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
            Some(ctx.accounts.amm_info.key()),
        )?;

        // Now mutate amm_info
        let amm = &mut ctx.accounts.amm_info;
        amm.mint0 = mint0_key;
        amm.mint1 = mint1_key;
        amm.token_account0 = ctx.accounts.token_account0.key();
        amm.token_account1 = ctx.accounts.token_account1.key();
        amm.reserve0 = 0u64;
        amm.reserve1 = 0u64;
        amm.ever_deposited = false;
        amm.supply = 0u64;
        amm.bump = bump;

        Ok(())
    }

    // ---------- DEPOSIT ----------
    // amount0, amount1 are HUMAN units (e.g. 100 => 100 of mint decimals)
    pub fn deposit(ctx: Context<DepositCtx>, amount0: u64, amount1: u64) -> Result<()> {
        // Scale to native units for token transfers & reserve bookkeeping
        let amount0_native = human_to_native(amount0, ctx.accounts.mint0.decimals)?;
        let amount1_native = human_to_native(amount1, ctx.accounts.mint1.decimals)?;
        require!(amount0_native > 0 && amount1_native > 0, AmmError::ZeroDeposit);

        // Mutable AmmInfo borrow
        let amm = &mut ctx.accounts.amm_info;

        // Defensive checks: mints must match
        require_keys_eq!(amm.mint0, ctx.accounts.mint0.key(), AmmError::MintMismatch);
        require_keys_eq!(amm.mint1, ctx.accounts.mint1.key(), AmmError::MintMismatch);

        // Transfer native tokens from sender -> AMM token accounts (CPI)
        token::transfer(
            CpiContext::new(
                ctx.accounts.token_program.to_account_info(),
                Transfer {
                    from: ctx.accounts.senders_token_account0.to_account_info(),
                    to: ctx.accounts.pdas_token_account0.to_account_info(),
                    authority: ctx.accounts.sender.to_account_info(),
                },
            ),
            amount0_native,
        )?;
        token::transfer(
            CpiContext::new(
                ctx.accounts.token_program.to_account_info(),
                Transfer {
                    from: ctx.accounts.senders_token_account1.to_account_info(),
                    to: ctx.accounts.pdas_token_account1.to_account_info(),
                    authority: ctx.accounts.sender.to_account_info(),
                },
            ),
            amount1_native,
        )?;

        // Compute minted LP in HUMAN units (per your tests)
        // First deposit -> minted_human = amount0 (human)
        let minted_human: u64;
        if !amm.ever_deposited {
            minted_human = amount0;
            amm.ever_deposited = true;
        } else {
            // For proportional minting we must compare human units:
            // compute reserve0_human = floor(reserve0_native / 10^decimals0)
            // compute minted_human = min(amount0 * supply / reserve0_human, amount1 * supply / reserve1_human)
            let supply_human = amm.supply as u128;
            require!(supply_human > 0, AmmError::DivisionByZero);

            let reserve0_native_u128 = amm.reserve0 as u128;
            let reserve1_native_u128 = amm.reserve1 as u128;

            let r0_human = native_to_human_u128(reserve0_native_u128, ctx.accounts.mint0.decimals)?;
            let r1_human = native_to_human_u128(reserve1_native_u128, ctx.accounts.mint1.decimals)?;
            require!(r0_human > 0 && r1_human > 0, AmmError::DivisionByZero);

            let cand0 = (amount0 as u128)
                .checked_mul(supply_human)
                .ok_or(AmmError::Overflow)?
                .checked_div(r0_human)
                .ok_or(AmmError::DivisionByZero)?;
            let cand1 = (amount1 as u128)
                .checked_mul(supply_human)
                .ok_or(AmmError::Overflow)?
                .checked_div(r1_human)
                .ok_or(AmmError::DivisionByZero)?;

            let minted_u128 = std::cmp::min(cand0, cand1);
            require!(minted_u128 > 0, AmmError::InsufficientLiquidityMinted);
            minted_human = minted_u128.try_into().map_err(|_| AmmError::Overflow)?;
        }

        // Update reserves (native) and supply (human)
        amm.reserve0 = amm.reserve0.checked_add(amount0_native).ok_or(AmmError::Overflow)?;
        amm.reserve1 = amm.reserve1.checked_add(amount1_native).ok_or(AmmError::Overflow)?;
        amm.supply = amm.supply.checked_add(minted_human).ok_or(AmmError::Overflow)?;

        // Update minted_pda (stores human LP units)
        let minted_pda = &mut ctx.accounts.minted_pda;
        minted_pda.minted = minted_pda
            .minted
            .checked_add(minted_human)
            .ok_or(AmmError::Overflow)?;

        Ok(())
    }

    // ---------- REDEEM ----------
    // amount = LP amount in HUMAN units
    pub fn redeem(ctx: Context<RedeemOrSwapCtx>, amount: u64) -> Result<()> {
        require!(amount > 0, AmmError::ZeroRedeem);

        let amm = &mut ctx.accounts.amm_info;
        let minted_pda = &mut ctx.accounts.minted_pda;

        // Validate sender has LP (human units)
        require!(minted_pda.minted >= amount, AmmError::InsufficientMintedForRedeem);
        require!(amm.supply > 0, AmmError::ZeroSupply);

        // Compute proportional native outputs:
        // out0_native = reserve0_native * amount_human / supply_human
        let out0_u128 = (amm.reserve0 as u128)
            .checked_mul(amount as u128)
            .ok_or(AmmError::Overflow)?
            .checked_div(amm.supply as u128)
            .ok_or(AmmError::DivisionByZero)?;
        let out1_u128 = (amm.reserve1 as u128)
            .checked_mul(amount as u128)
            .ok_or(AmmError::Overflow)?
            .checked_div(amm.supply as u128)
            .ok_or(AmmError::DivisionByZero)?;

        let out0: u64 = out0_u128.try_into().map_err(|_| AmmError::Overflow)?;
        let out1: u64 = out1_u128.try_into().map_err(|_| AmmError::Overflow)?;

        // Update AMM state (decrease native reserves, decrease human supply & minted_pda)
        amm.reserve0 = amm.reserve0.checked_sub(out0).ok_or(AmmError::Overflow)?;
        amm.reserve1 = amm.reserve1.checked_sub(out1).ok_or(AmmError::Overflow)?;
        amm.supply = amm.supply.checked_sub(amount).ok_or(AmmError::Overflow)?;
        minted_pda.minted = minted_pda.minted.checked_sub(amount).ok_or(AmmError::Overflow)?;

        // signer seeds for AMM PDA (bind keys to avoid temporary-borrow)
        let mint0_key = ctx.accounts.mint0.key();
        let mint1_key = ctx.accounts.mint1.key();
        let bump = amm.bump;
        let signer_seeds: &[&[u8]] = &[
            b"amm",
            mint0_key.as_ref(),
            mint1_key.as_ref(),
            &[bump],
        ];

        // Transfer native tokens from AMM -> sender
        token::transfer(
            CpiContext::new_with_signer(
                ctx.accounts.token_program.to_account_info(),
                Transfer {
                    from: ctx.accounts.pdas_token_account0.to_account_info(),
                    to: ctx.accounts.senders_token_account0.to_account_info(),
                    authority: ctx.accounts.amm_info.to_account_info(),
                },
                &[signer_seeds],
            ),
            out0,
        )?;
        token::transfer(
            CpiContext::new_with_signer(
                ctx.accounts.token_program.to_account_info(),
                Transfer {
                    from: ctx.accounts.pdas_token_account1.to_account_info(),
                    to: ctx.accounts.senders_token_account1.to_account_info(),
                    authority: ctx.accounts.amm_info.to_account_info(),
                },
                &[signer_seeds],
            ),
            out1,
        )?;

        Ok(())
    }

    // ---------- SWAP ----------
    // amount_in, min_out_amount are HUMAN units; function scales to native for transfers & math.
    // is_mint0: true => input mint0 -> output mint1
    pub fn swap(
        ctx: Context<RedeemOrSwapCtx>,
        is_mint0: bool,
        amount_in: u64,
        min_out_amount: u64,
    ) -> Result<()> {
        require!(amount_in > 0, AmmError::ZeroSwapAmount);

        let amm = &mut ctx.accounts.amm_info;
        require!(amm.ever_deposited, AmmError::NotInitialized);

        // Bind keys to avoid temporary-borrow lifetimes
        let mint0_key = ctx.accounts.mint0.key();
        let mint1_key = ctx.accounts.mint1.key();

        // Determine decimals for scaling
        let (in_decimals, out_decimals) = if is_mint0 {
            (ctx.accounts.mint0.decimals, ctx.accounts.mint1.decimals)
        } else {
            (ctx.accounts.mint1.decimals, ctx.accounts.mint0.decimals)
        };

        // Human -> native
        let amount_in_native = human_to_native(amount_in, in_decimals)?;
        let min_out_native = human_to_native(min_out_amount, out_decimals)?;

        // Select reserves (native) and accounts
        let (reserve_in_native, reserve_out_native, pdas_in_info, pdas_out_info, sender_in_info, sender_out_info) =
            if is_mint0 {
                (
                    amm.reserve0 as u128,
                    amm.reserve1 as u128,
                    ctx.accounts.pdas_token_account0.to_account_info(),
                    ctx.accounts.pdas_token_account1.to_account_info(),
                    ctx.accounts.senders_token_account0.to_account_info(),
                    ctx.accounts.senders_token_account1.to_account_info(),
                )
            } else {
                (
                    amm.reserve1 as u128,
                    amm.reserve0 as u128,
                    ctx.accounts.pdas_token_account1.to_account_info(),
                    ctx.accounts.pdas_token_account0.to_account_info(),
                    ctx.accounts.senders_token_account1.to_account_info(),
                    ctx.accounts.senders_token_account0.to_account_info(),
                )
            };

        // Transfer input from sender -> AMM (native)
        token::transfer(
            CpiContext::new(
                ctx.accounts.token_program.to_account_info(),
                Transfer {
                    from: sender_in_info.clone(),
                    to: pdas_in_info.clone(),
                    authority: ctx.accounts.sender.to_account_info(),
                },
            ),
            amount_in_native,
        )?;

        // Constant-product calculation in native units:
        // amount_out_native = reserve_out - (reserve_in * reserve_out) / (reserve_in + amount_in)
        let dx = amount_in_native as u128;
        let x = reserve_in_native;
        let y = reserve_out_native;
        require!(x > 0 && y > 0 && x.checked_add(dx).is_some(), AmmError::DivisionByZero);

        let k = x.checked_mul(y).ok_or(AmmError::Overflow)?;
        let new_y = k
            .checked_div(x.checked_add(dx).ok_or(AmmError::Overflow)?)
            .ok_or(AmmError::DivisionByZero)?;
        let amount_out_native_u128 = y.checked_sub(new_y).ok_or(AmmError::Overflow)?;
        let amount_out_native: u64 = amount_out_native_u128.try_into().map_err(|_| AmmError::Overflow)?;

        // Slippage (native)
        require!(amount_out_native >= min_out_native, AmmError::SlippageExceeded);

        // Update reserves: reserves are native units
        if is_mint0 {
            amm.reserve0 = amm.reserve0.checked_add(amount_in_native).ok_or(AmmError::Overflow)?;
            amm.reserve1 = amm.reserve1.checked_sub(amount_out_native).ok_or(AmmError::Overflow)?;
        } else {
            amm.reserve1 = amm.reserve1.checked_add(amount_in_native).ok_or(AmmError::Overflow)?;
            amm.reserve0 = amm.reserve0.checked_sub(amount_out_native).ok_or(AmmError::Overflow)?;
        }

        // AMM PDA signs transfer of output to sender
        let bump = amm.bump;
        let signer_seeds: &[&[u8]] = &[
            b"amm",
            mint0_key.as_ref(),
            mint1_key.as_ref(),
            &[bump],
        ];

        token::transfer(
            CpiContext::new_with_signer(
                ctx.accounts.token_program.to_account_info(),
                Transfer {
                    from: pdas_out_info.clone(),
                    to: sender_out_info.clone(),
                    authority: ctx.accounts.amm_info.to_account_info(),
                },
                &[signer_seeds],
            ),
            amount_out_native,
        )?;

        Ok(())
    }
}

// --- helper: scale human -> native units using mint.decimals ---
fn scale_to_native(amount_human: u64, decimals: u8) -> Result<u64> {
    if amount_human == 0 { return Ok(0); }
    let factor: u128 = 10u128
        .checked_pow(decimals as u32)
        .ok_or(AmmError::Overflow)?;
    let scaled = (amount_human as u128)
        .checked_mul(factor)
        .ok_or(AmmError::Overflow)?;
    let scaled_u64: u64 = scaled.try_into().map_err(|_| AmmError::Overflow)?;
    Ok(scaled_u64)
}

// Helper: scale human -> native units using mint.decimals
fn scale_to_native_checked(amount_human: u64, decimals: u8) -> Result<u64> {
    if amount_human == 0 {
        return Ok(0);
    }
    // 10^decimals as u128
    let factor = 10u128
        .checked_pow(decimals as u32)
        .ok_or(AmmError::Overflow)?;
    let scaled = (amount_human as u128)
        .checked_mul(factor)
        .ok_or(AmmError::Overflow)?;
    let scaled_u64 = scaled.try_into().map_err(|_| AmmError::Overflow)?;
    Ok(scaled_u64)
}



// Returns 10^decimals as u128
fn pow10_u128(decimals: u8) -> Result<u128> {
    let pow = 10u128
        .checked_pow(decimals as u32)
        .ok_or(AmmError::Overflow)?;
    Ok(pow)
}

// Scale human -> native (checked), using mint decimals
fn human_to_native(amount_human: u64, decimals: u8) -> Result<u64> {
    if amount_human == 0 { return Ok(0); }
    let factor = pow10_u128(decimals)?;
    let scaled = (amount_human as u128)
        .checked_mul(factor)
        .ok_or(AmmError::Overflow)?;
    Ok(scaled.try_into().map_err(|_| AmmError::Overflow)?)
}

// Convert native -> human by integer division (floor).
// Returning u128 to allow intermediate math without overflow.
fn native_to_human_u128(native: u128, decimals: u8) -> Result<u128> {
    let factor = pow10_u128(decimals)?;
    Ok(native.checked_div(factor).ok_or(AmmError::DivisionByZero)?)
}
/// -----------------
/// Accounts / Contexts
/// -----------------
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

#[account]
pub struct MintedPDA {
    pub minted: u64,
    // bump not strictly necessary but often stored; we omit bump field to match minimal spec
}

#[derive(Accounts)]
#[instruction()]
pub struct InitializeCtx<'info> {
    /// initializer must sign
    #[account(mut, signer)]
    pub initializer: Signer<'info>,

    /// AMM info PDA
    #[account(
        init,
        payer = initializer,
        seeds = [b"amm", mint0.key().as_ref(), mint1.key().as_ref()],
        bump,
        space = 8 + std::mem::size_of::<AmmInfo>()
    )]
    pub amm_info: Box<Account<'info, AmmInfo>>,

    /// Token mints for pair
    pub mint0: Account<'info, Mint>,
    pub mint1: Account<'info, Mint>,

    /// Token accounts that will become owned by AMM PDA
    /// These token accounts must already exist and be initialized.
    #[account(mut)]
    pub token_account0: Account<'info, TokenAccount>,
    #[account(mut)]
    pub token_account1: Account<'info, TokenAccount>,

    /// Programs
    pub token_program: Program<'info, Token>,
    pub system_program: Program<'info, System>,
    pub associated_token_program: Program<'info, AssociatedToken>,
}

#[derive(Accounts)]
#[instruction()]
pub struct DepositCtx<'info> {
    /// sender must sign
    #[account(mut, signer)]
    pub sender: Signer<'info>,

    /// token mints
    pub mint0: Account<'info, Mint>,
    pub mint1: Account<'info, Mint>,

    /// AMM info PDA
    #[account(
        mut,
        seeds = [b"amm", mint0.key().as_ref(), mint1.key().as_ref()],
        bump = amm_info.bump
    )]
    pub amm_info: Box<Account<'info, AmmInfo>>,

    /// Minted PDA for this sender (init if needed)
    #[account(
        init_if_needed,
        payer = sender,
        seeds = [b"minted", sender.key().as_ref()],
        bump,
        space = 8 + std::mem::size_of::<MintedPDA>()
    )]
    pub minted_pda: Box<Account<'info, MintedPDA>>,

    /// Sender token accounts (source)
    #[account(mut, constraint = senders_token_account0.mint == mint0.key())]
    pub senders_token_account0: Account<'info, TokenAccount>,
    #[account(mut, constraint = senders_token_account1.mint == mint1.key())]
    pub senders_token_account1: Account<'info, TokenAccount>,

    /// AMM token accounts (destination) - must match amm_info's token accounts
    #[account(mut, constraint = pdas_token_account0.key() == amm_info.token_account0)]
    pub pdas_token_account0: Account<'info, TokenAccount>,
    #[account(mut, constraint = pdas_token_account1.key() == amm_info.token_account1)]
    pub pdas_token_account1: Account<'info, TokenAccount>,

    /// Programs
    pub token_program: Program<'info, Token>,
    pub system_program: Program<'info, System>,
    pub associated_token_program: Program<'info, AssociatedToken>,
}

#[derive(Accounts)]
#[instruction()]
pub struct RedeemOrSwapCtx<'info> {
    /// sender must sign
    #[account(mut, signer)]
    pub sender: Signer<'info>,

    /// token mints
    pub mint0: Account<'info, Mint>,
    pub mint1: Account<'info, Mint>,

    /// AMM info PDA (mutable for reserve updates)
    #[account(
        mut,
        seeds = [b"amm", mint0.key().as_ref(), mint1.key().as_ref()],
        bump = amm_info.bump
    )]
    pub amm_info: Box<Account<'info, AmmInfo>>,

    /// Minted PDA (for redeem validation). For swap it's provided but not modified.
    #[account(mut, seeds = [b"minted", sender.key().as_ref()], bump)]
    pub minted_pda: Box<Account<'info, MintedPDA>>,

    /// Sender token accounts
    #[account(mut, constraint = senders_token_account0.mint == mint0.key())]
    pub senders_token_account0: Account<'info, TokenAccount>,
    #[account(mut, constraint = senders_token_account1.mint == mint1.key())]
    pub senders_token_account1: Account<'info, TokenAccount>,

    /// AMM token accounts (must match amm_info)
    #[account(mut, constraint = pdas_token_account0.key() == amm_info.token_account0)]
    pub pdas_token_account0: Account<'info, TokenAccount>,
    #[account(mut, constraint = pdas_token_account1.key() == amm_info.token_account1)]
    pub pdas_token_account1: Account<'info, TokenAccount>,

    /// Programs
    pub token_program: Program<'info, Token>,
    pub system_program: Program<'info, System>,
    pub associated_token_program: Program<'info, AssociatedToken>,
}

/// -----------------
/// Errors
/// -----------------
#[error_code]
pub enum AmmError {
    #[msg("Overflow occurred")]
    Overflow,
    #[msg("Division by zero")]
    DivisionByZero,
    #[msg("Mint mismatch with AMM")]
    MintMismatch,
    #[msg("Zero deposit not allowed")]
    ZeroDeposit,
    #[msg("Insufficient liquidity minted")]
    InsufficientLiquidityMinted,
    #[msg("Zero redeem amount")]
    ZeroRedeem,
    #[msg("Insufficient minted for redeem")]
    InsufficientMintedForRedeem,
    #[msg("Zero supply")]
    ZeroSupply,
    #[msg("Invalid account")]
    InvalidAccount,
    #[msg("Zero swap amount")]
    ZeroSwapAmount,
    #[msg("Slippage exceeded")]
    SlippageExceeded,
    #[msg("AMM not initialized")]
    NotInitialized,
}

