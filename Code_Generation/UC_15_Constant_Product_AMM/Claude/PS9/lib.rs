use anchor_lang::prelude::*;
use anchor_spl::associated_token::AssociatedToken;
use anchor_spl::token::{self, Mint, Token, TokenAccount, Transfer, SetAuthority};
use anchor_spl::token::spl_token::instruction::AuthorityType;

declare_id!("8TA7VF7CWu6RAQJn4mNHqsMop87szWpbor7adRpdiUvd");

#[program]
pub mod constant_product_amm {
    use super::*;

    pub fn initialize(ctx: Context<InitializeCtx>) -> Result<()> {
        let amm_info = &mut ctx.accounts.amm_info;
        
        // Initialize AMM state
        amm_info.mint0 = ctx.accounts.mint0.key();
        amm_info.mint1 = ctx.accounts.mint1.key();
        amm_info.token_account0 = ctx.accounts.token_account0.key();
        amm_info.token_account1 = ctx.accounts.token_account1.key();
        amm_info.reserve0 = 0;
        amm_info.reserve1 = 0;
        amm_info.ever_deposited = false;
        amm_info.supply = 0;
        amm_info.decimals0 = ctx.accounts.mint0.decimals;
        amm_info.decimals1 = ctx.accounts.mint1.decimals;

        // Transfer ownership of token accounts to AMM PDA
        let cpi_accounts_0 = SetAuthority {
            account_or_mint: ctx.accounts.token_account0.to_account_info(),
            current_authority: ctx.accounts.initializer.to_account_info(),
        };
        let cpi_ctx_0 = CpiContext::new(ctx.accounts.token_program.to_account_info(), cpi_accounts_0);
        token::set_authority(cpi_ctx_0, AuthorityType::AccountOwner, Some(amm_info.key()))?;

        let cpi_accounts_1 = SetAuthority {
            account_or_mint: ctx.accounts.token_account1.to_account_info(),
            current_authority: ctx.accounts.initializer.to_account_info(),
        };
        let cpi_ctx_1 = CpiContext::new(ctx.accounts.token_program.to_account_info(), cpi_accounts_1);
        token::set_authority(cpi_ctx_1, AuthorityType::AccountOwner, Some(amm_info.key()))?;

        Ok(())
    }

    pub fn deposit(ctx: Context<DepositCtx>, amount0: u64, amount1: u64) -> Result<()> {
        require!(amount0 > 0 && amount1 > 0, AmmError::InvalidAmount);

        let amm_info = &mut ctx.accounts.amm_info;
        let minted_pda = &mut ctx.accounts.minted_pda;

        // Get decimals for scaling from stored values
        let decimals0 = amm_info.decimals0;
        let decimals1 = amm_info.decimals1;

        // Calculate actual amounts to transfer (logical amount * 10^decimals)
        let actual_amount0 = amount0
            .checked_mul(10u64.pow(decimals0 as u32))
            .ok_or(AmmError::MathOverflow)?;
        let actual_amount1 = amount1
            .checked_mul(10u64.pow(decimals1 as u32))
            .ok_or(AmmError::MathOverflow)?;

        // Transfer tokens from sender to AMM
        let cpi_accounts_0 = Transfer {
            from: ctx.accounts.senders_token_account0.to_account_info(),
            to: ctx.accounts.pdas_token_account0.to_account_info(),
            authority: ctx.accounts.sender.to_account_info(),
        };
        let cpi_ctx_0 = CpiContext::new(ctx.accounts.token_program.to_account_info(), cpi_accounts_0);
        token::transfer(cpi_ctx_0, actual_amount0)?;

        let cpi_accounts_1 = Transfer {
            from: ctx.accounts.senders_token_account1.to_account_info(),
            to: ctx.accounts.pdas_token_account1.to_account_info(),
            authority: ctx.accounts.sender.to_account_info(),
        };
        let cpi_ctx_1 = CpiContext::new(ctx.accounts.token_program.to_account_info(), cpi_accounts_1);
        token::transfer(cpi_ctx_1, actual_amount1)?;

        let liquidity = if !amm_info.ever_deposited {
            // First deposit - liquidity equals amount0
            amm_info.ever_deposited = true;
            amount0
        } else {
            // Subsequent deposits - maintain ratio
            let reserve0 = amm_info.reserve0;
            let reserve1 = amm_info.reserve1;
            
            require!(reserve0 > 0 && reserve1 > 0, AmmError::InvalidReserves);

            // Calculate liquidity based on the minimum ratio to prevent manipulation
            let liquidity0 = (amount0 as u128)
                .checked_mul(amm_info.supply as u128)
                .ok_or(AmmError::MathOverflow)?
                .checked_div(reserve0 as u128)
                .ok_or(AmmError::MathOverflow)? as u64;

            let liquidity1 = (amount1 as u128)
                .checked_mul(amm_info.supply as u128)
                .ok_or(AmmError::MathOverflow)?
                .checked_div(reserve1 as u128)
                .ok_or(AmmError::MathOverflow)? as u64;

            // Take minimum to prevent manipulation
            let liquidity = liquidity0.min(liquidity1);
            require!(liquidity > 0, AmmError::InsufficientLiquidity);
            
            liquidity
        };

        // Update reserves with logical amounts
        amm_info.reserve0 = amm_info.reserve0.checked_add(amount0).ok_or(AmmError::MathOverflow)?;
        amm_info.reserve1 = amm_info.reserve1.checked_add(amount1).ok_or(AmmError::MathOverflow)?;
        amm_info.supply = amm_info.supply.checked_add(liquidity).ok_or(AmmError::MathOverflow)?;

        // Track minted liquidity for this sender
        minted_pda.minted = minted_pda.minted.checked_add(liquidity).ok_or(AmmError::MathOverflow)?;

        Ok(())
    }

    pub fn redeem(ctx: Context<RedeemOrSwapCtx>, amount: u64) -> Result<()> {
        require!(amount > 0, AmmError::InvalidAmount);

        let amm_info = &mut ctx.accounts.amm_info;
        let minted_pda = &mut ctx.accounts.minted_pda;

        require!(amm_info.supply > 0, AmmError::InsufficientLiquidity);
        require!(minted_pda.minted >= amount, AmmError::InsufficientMintedBalance);

        // Get decimals for scaling from stored values
        let decimals0 = amm_info.decimals0;
        let decimals1 = amm_info.decimals1;
        
        let scale0 = 10u64.pow(decimals0 as u32);
        let scale1 = 10u64.pow(decimals1 as u32);

        // Calculate actual amounts to return with proper rounding
        // actual_amount = (amount * reserve * scale) / supply
        let actual_amount0 = (amount as u128)
            .checked_mul(amm_info.reserve0 as u128)
            .ok_or(AmmError::MathOverflow)?
            .checked_mul(scale0 as u128)
            .ok_or(AmmError::MathOverflow)?
            .checked_div(amm_info.supply as u128)
            .ok_or(AmmError::MathOverflow)? as u64;

        let actual_amount1 = (amount as u128)
            .checked_mul(amm_info.reserve1 as u128)
            .ok_or(AmmError::MathOverflow)?
            .checked_mul(scale1 as u128)
            .ok_or(AmmError::MathOverflow)?
            .checked_div(amm_info.supply as u128)
            .ok_or(AmmError::MathOverflow)? as u64;

        require!(actual_amount0 > 0 && actual_amount1 > 0, AmmError::InsufficientOutput);

        // Calculate logical amounts for reserve update
        let amount0 = (amount as u128)
            .checked_mul(amm_info.reserve0 as u128)
            .ok_or(AmmError::MathOverflow)?
            .checked_div(amm_info.supply as u128)
            .ok_or(AmmError::MathOverflow)? as u64;

        let amount1 = (amount as u128)
            .checked_mul(amm_info.reserve1 as u128)
            .ok_or(AmmError::MathOverflow)?
            .checked_div(amm_info.supply as u128)
            .ok_or(AmmError::MathOverflow)? as u64;

        // Prepare PDA signer seeds
        let mint0_key = ctx.accounts.mint0.key();
        let mint1_key = ctx.accounts.mint1.key();
        let seeds = &[
            b"amm",
            mint0_key.as_ref(),
            mint1_key.as_ref(),
            &[ctx.bumps.amm_info],
        ];
        let signer = &[&seeds[..]];

        // Transfer tokens from AMM to sender
        let cpi_accounts_0 = Transfer {
            from: ctx.accounts.pdas_token_account0.to_account_info(),
            to: ctx.accounts.senders_token_account0.to_account_info(),
            authority: amm_info.to_account_info(),
        };
        let cpi_ctx_0 = CpiContext::new_with_signer(
            ctx.accounts.token_program.to_account_info(),
            cpi_accounts_0,
            signer,
        );
        token::transfer(cpi_ctx_0, actual_amount0)?;

        let cpi_accounts_1 = Transfer {
            from: ctx.accounts.pdas_token_account1.to_account_info(),
            to: ctx.accounts.senders_token_account1.to_account_info(),
            authority: amm_info.to_account_info(),
        };
        let cpi_ctx_1 = CpiContext::new_with_signer(
            ctx.accounts.token_program.to_account_info(),
            cpi_accounts_1,
            signer,
        );
        token::transfer(cpi_ctx_1, actual_amount1)?;

        // Update reserves with logical amounts
        amm_info.reserve0 = amm_info.reserve0.checked_sub(amount0).ok_or(AmmError::MathOverflow)?;
        amm_info.reserve1 = amm_info.reserve1.checked_sub(amount1).ok_or(AmmError::MathOverflow)?;
        amm_info.supply = amm_info.supply.checked_sub(amount).ok_or(AmmError::MathOverflow)?;

        // Update minted balance
        minted_pda.minted = minted_pda.minted.checked_sub(amount).ok_or(AmmError::MathOverflow)?;

        Ok(())
    }

    pub fn swap(
        ctx: Context<RedeemOrSwapCtx>,
        is_mint0: bool,
        amount_in: u64,
        min_out_amount: u64,
    ) -> Result<()> {
        require!(amount_in > 0, AmmError::InvalidAmount);

        let amm_info = &mut ctx.accounts.amm_info;
        
        require!(amm_info.reserve0 > 0 && amm_info.reserve1 > 0, AmmError::InsufficientLiquidity);

        // Calculate output amount using constant product formula
        // (reserve_in + amount_in) * (reserve_out - amount_out) = reserve_in * reserve_out
        // amount_out = (amount_in * reserve_out) / (reserve_in + amount_in)
        let (reserve_in, reserve_out) = if is_mint0 {
            (amm_info.reserve0, amm_info.reserve1)
        } else {
            (amm_info.reserve1, amm_info.reserve0)
        };

        let amount_out = (amount_in as u128)
            .checked_mul(reserve_out as u128)
            .ok_or(AmmError::MathOverflow)?
            .checked_div(
                (reserve_in as u128)
                    .checked_add(amount_in as u128)
                    .ok_or(AmmError::MathOverflow)?
            )
            .ok_or(AmmError::MathOverflow)? as u64;

        require!(amount_out >= min_out_amount, AmmError::SlippageExceeded);
        require!(amount_out > 0, AmmError::InsufficientOutput);
        require!(amount_out < reserve_out, AmmError::InsufficientLiquidity);

        // Prepare PDA signer seeds
        let mint0_key = ctx.accounts.mint0.key();
        let mint1_key = ctx.accounts.mint1.key();
        let seeds = &[
            b"amm",
            mint0_key.as_ref(),
            mint1_key.as_ref(),
            &[ctx.bumps.amm_info],
        ];
        let signer = &[&seeds[..]];

        if is_mint0 {
            // Transfer mint0 from sender to AMM
            let cpi_accounts_in = Transfer {
                from: ctx.accounts.senders_token_account0.to_account_info(),
                to: ctx.accounts.pdas_token_account0.to_account_info(),
                authority: ctx.accounts.sender.to_account_info(),
            };
            let cpi_ctx_in = CpiContext::new(ctx.accounts.token_program.to_account_info(), cpi_accounts_in);
            token::transfer(cpi_ctx_in, amount_in)?;

            // Transfer mint1 from AMM to sender
            let cpi_accounts_out = Transfer {
                from: ctx.accounts.pdas_token_account1.to_account_info(),
                to: ctx.accounts.senders_token_account1.to_account_info(),
                authority: amm_info.to_account_info(),
            };
            let cpi_ctx_out = CpiContext::new_with_signer(
                ctx.accounts.token_program.to_account_info(),
                cpi_accounts_out,
                signer,
            );
            token::transfer(cpi_ctx_out, amount_out)?;

            // Update reserves
            amm_info.reserve0 = amm_info.reserve0.checked_add(amount_in).ok_or(AmmError::MathOverflow)?;
            amm_info.reserve1 = amm_info.reserve1.checked_sub(amount_out).ok_or(AmmError::MathOverflow)?;
        } else {
            // Transfer mint1 from sender to AMM
            let cpi_accounts_in = Transfer {
                from: ctx.accounts.senders_token_account1.to_account_info(),
                to: ctx.accounts.pdas_token_account1.to_account_info(),
                authority: ctx.accounts.sender.to_account_info(),
            };
            let cpi_ctx_in = CpiContext::new(ctx.accounts.token_program.to_account_info(), cpi_accounts_in);
            token::transfer(cpi_ctx_in, amount_in)?;

            // Transfer mint0 from AMM to sender
            let cpi_accounts_out = Transfer {
                from: ctx.accounts.pdas_token_account0.to_account_info(),
                to: ctx.accounts.senders_token_account0.to_account_info(),
                authority: amm_info.to_account_info(),
            };
            let cpi_ctx_out = CpiContext::new_with_signer(
                ctx.accounts.token_program.to_account_info(),
                cpi_accounts_out,
                signer,
            );
            token::transfer(cpi_ctx_out, amount_out)?;

            // Update reserves
            amm_info.reserve1 = amm_info.reserve1.checked_add(amount_in).ok_or(AmmError::MathOverflow)?;
            amm_info.reserve0 = amm_info.reserve0.checked_sub(amount_out).ok_or(AmmError::MathOverflow)?;
        }

        Ok(())
    }
}

#[derive(Accounts)]
pub struct InitializeCtx<'info> {
    #[account(mut)]
    pub initializer: Signer<'info>,

    #[account(
        init,
        payer = initializer,
        space = 8 + AmmInfo::INIT_SPACE,
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
        bump
    )]
    pub amm_info: Account<'info, AmmInfo>,

    #[account(
        init_if_needed,
        payer = sender,
        space = 8 + MintedPda::INIT_SPACE,
        seeds = [b"minted", sender.key().as_ref()],
        bump
    )]
    pub minted_pda: Account<'info, MintedPda>,

    #[account(
        mut,
        associated_token::mint = mint0,
        associated_token::authority = sender
    )]
    pub senders_token_account0: Account<'info, TokenAccount>,

    #[account(
        mut,
        associated_token::mint = mint1,
        associated_token::authority = sender
    )]
    pub senders_token_account1: Account<'info, TokenAccount>,

    #[account(
        mut,
        constraint = pdas_token_account0.key() == amm_info.token_account0,
        constraint = pdas_token_account0.mint == mint0.key()
    )]
    pub pdas_token_account0: Account<'info, TokenAccount>,

    #[account(
        mut,
        constraint = pdas_token_account1.key() == amm_info.token_account1,
        constraint = pdas_token_account1.mint == mint1.key()
    )]
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
        bump
    )]
    pub amm_info: Account<'info, AmmInfo>,

    #[account(
        mut,
        seeds = [b"minted", sender.key().as_ref()],
        bump
    )]
    pub minted_pda: Account<'info, MintedPda>,

    #[account(
        mut,
        associated_token::mint = mint0,
        associated_token::authority = sender
    )]
    pub senders_token_account0: Account<'info, TokenAccount>,

    #[account(
        mut,
        associated_token::mint = mint1,
        associated_token::authority = sender
    )]
    pub senders_token_account1: Account<'info, TokenAccount>,

    #[account(
        mut,
        constraint = pdas_token_account0.key() == amm_info.token_account0,
        constraint = pdas_token_account0.mint == mint0.key()
    )]
    pub pdas_token_account0: Account<'info, TokenAccount>,

    #[account(
        mut,
        constraint = pdas_token_account1.key() == amm_info.token_account1,
        constraint = pdas_token_account1.mint == mint1.key()
    )]
    pub pdas_token_account1: Account<'info, TokenAccount>,

    pub token_program: Program<'info, Token>,
    pub system_program: Program<'info, System>,
    pub associated_token_program: Program<'info, AssociatedToken>,
}

#[account]
#[derive(InitSpace)]
pub struct AmmInfo {
    pub mint0: Pubkey,
    pub mint1: Pubkey,
    pub token_account0: Pubkey,
    pub token_account1: Pubkey,
    pub reserve0: u64,
    pub reserve1: u64,
    pub ever_deposited: bool,
    pub supply: u64,
    pub decimals0: u8,
    pub decimals1: u8,
}

#[account]
#[derive(InitSpace)]
pub struct MintedPda {
    pub minted: u64,
}

#[error_code]
pub enum AmmError {
    #[msg("Invalid amount provided")]
    InvalidAmount,
    #[msg("Math overflow occurred")]
    MathOverflow,
    #[msg("Insufficient liquidity")]
    InsufficientLiquidity,
    #[msg("Invalid reserves")]
    InvalidReserves,
    #[msg("Insufficient output amount")]
    InsufficientOutput,
    #[msg("Slippage tolerance exceeded")]
    SlippageExceeded,
    #[msg("Insufficient minted balance")]
    InsufficientMintedBalance,
}