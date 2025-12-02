use anchor_lang::prelude::*;
use anchor_spl::associated_token::AssociatedToken;
use anchor_spl::token::{self, Mint, Token, TokenAccount, Transfer};

declare_id!("HNstJu1Z5ib6envsyjmfwLJUDRp78tnnygJ1HcinsiNS");

#[program]
pub mod amm_program {
    use super::*;

    pub fn initialize(ctx: Context<InitializeCtx>) -> Result<()> {
        let amm_info = &mut ctx.accounts.amm_info;
        
        amm_info.mint0 = ctx.accounts.mint0.key();
        amm_info.mint1 = ctx.accounts.mint1.key();
        amm_info.token_account0 = ctx.accounts.token_account0.key();
        amm_info.token_account1 = ctx.accounts.token_account1.key();
        amm_info.reserve0 = 0;
        amm_info.reserve1 = 0;
        amm_info.ever_deposited = false;
        amm_info.supply = 0;
        
        Ok(())
    }

    pub fn deposit(
        ctx: Context<DepositCtx>,
        amount0: u64,
        amount1: u64,
    ) -> Result<()> {
        require!(amount0 > 0 && amount1 > 0, AmmError::InvalidDepositAmount);

        let amm_info = &mut ctx.accounts.amm_info;
        
        // Transfer token0 from sender to AMM
        let cpi_accounts0 = Transfer {
            from: ctx.accounts.senders_token_account0.to_account_info(),
            to: ctx.accounts.pdas_token_account0.to_account_info(),
            authority: ctx.accounts.sender.to_account_info(),
        };
        let cpi_program0 = ctx.accounts.token_program.to_account_info();
        let cpi_ctx0 = CpiContext::new(cpi_program0, cpi_accounts0);
        token::transfer(cpi_ctx0, amount0)?;

        // Transfer token1 from sender to AMM
        let cpi_accounts1 = Transfer {
            from: ctx.accounts.senders_token_account1.to_account_info(),
            to: ctx.accounts.pdas_token_account1.to_account_info(),
            authority: ctx.accounts.sender.to_account_info(),
        };
        let cpi_program1 = ctx.accounts.token_program.to_account_info();
        let cpi_ctx1 = CpiContext::new(cpi_program1, cpi_accounts1);
        token::transfer(cpi_ctx1, amount1)?;

        // Calculate liquidity tokens to mint with better precision
        let liquidity = if !amm_info.ever_deposited {
            amm_info.ever_deposited = true;
            // Initial liquidity = sqrt(amount0 * amount1)
            let product = (amount0 as u128)
                .checked_mul(amount1 as u128)
                .ok_or(AmmError::MathOverflow)?;
            integer_sqrt(product).try_into().map_err(|_| AmmError::MathOverflow)?
        } else {
            let total_supply = amm_info.supply;
            let reserve0 = amm_info.reserve0;
            let reserve1 = amm_info.reserve1;
            
            // Calculate proportional liquidity with better precision
            let liquidity0 = (amount0 as u128)
                .checked_mul(total_supply as u128)
                .ok_or(AmmError::MathOverflow)?
                .checked_div(reserve0 as u128)
                .ok_or(AmmError::DivisionByZero)?;
                
            let liquidity1 = (amount1 as u128)
                .checked_mul(total_supply as u128)
                .ok_or(AmmError::MathOverflow)?
                .checked_div(reserve1 as u128)
                .ok_or(AmmError::DivisionByZero)?;
            
            // Use the minimum to maintain ratio, with proper rounding
            let min_liquidity = std::cmp::min(liquidity0, liquidity1);
            min_liquidity.try_into().map_err(|_| AmmError::MathOverflow)?
        };

        require!(liquidity > 0, AmmError::InsufficientLiquidityMinted);

        // Update reserves and supply
        amm_info.reserve0 = amm_info
            .reserve0
            .checked_add(amount0)
            .ok_or(AmmError::MathOverflow)?;
        amm_info.reserve1 = amm_info
            .reserve1
            .checked_add(amount1)
            .ok_or(AmmError::MathOverflow)?;
        amm_info.supply = amm_info
            .supply
            .checked_add(liquidity)
            .ok_or(AmmError::MathOverflow)?;

        // Update minted PDA
        let minted_pda = &mut ctx.accounts.minted_pda;
        minted_pda.minted = minted_pda
            .minted
            .checked_add(liquidity)
            .ok_or(AmmError::MathOverflow)?;

        Ok(())
    }

    pub fn redeem(ctx: Context<RedeemOrSwapCtx>, amount: u64) -> Result<()> {
        require!(amount > 0, AmmError::InvalidRedeemAmount);

        let amm_info = &mut ctx.accounts.amm_info;
        let minted_pda = &mut ctx.accounts.minted_pda;

        // Check if user has enough liquidity tokens
        require!(
            minted_pda.minted >= amount,
            AmmError::InsufficientLiquidity
        );

        let total_supply = amm_info.supply;
        let reserve0 = amm_info.reserve0;
        let reserve1 = amm_info.reserve1;

        // Calculate token amounts to return with better precision
        let amount0 = (reserve0 as u128)
            .checked_mul(amount as u128)
            .ok_or(AmmError::MathOverflow)?
            .checked_div(total_supply as u128)
            .ok_or(AmmError::DivisionByZero)?
            .try_into()
            .map_err(|_| AmmError::MathOverflow)?;

        let amount1 = (reserve1 as u128)
            .checked_mul(amount as u128)
            .ok_or(AmmError::MathOverflow)?
            .checked_div(total_supply as u128)
            .ok_or(AmmError::DivisionByZero)?
            .try_into()
            .map_err(|_| AmmError::MathOverflow)?;

        require!(amount0 > 0 && amount1 > 0, AmmError::InsufficientLiquidityBurned);

        // Update reserves and supply
        amm_info.reserve0 = amm_info
            .reserve0
            .checked_sub(amount0)
            .ok_or(AmmError::InsufficientReserves)?;
        amm_info.reserve1 = amm_info
            .reserve1
            .checked_sub(amount1)
            .ok_or(AmmError::InsufficientReserves)?;
        amm_info.supply = amm_info
            .supply
            .checked_sub(amount)
            .ok_or(AmmError::MathOverflow)?;

        // Update minted PDA
        minted_pda.minted = minted_pda
            .minted
            .checked_sub(amount)
            .ok_or(AmmError::MathOverflow)?;

        // Extract necessary data before mutable borrow ends
        let mint0 = amm_info.mint0;
        let mint1 = amm_info.mint1;
        let bump = ctx.bumps.amm_info;

        // Drop mutable references
        drop(amm_info);
        drop(minted_pda);

        // For redeem, we need to use the AMM PDA as signer for transfers
        let seeds = &[
            b"amm",
            mint0.as_ref(),
            mint1.as_ref(),
            &[bump],
        ];
        let signer = &[&seeds[..]];

        // Transfer token0 from AMM to sender
        let cpi_accounts0 = Transfer {
            from: ctx.accounts.pdas_token_account0.to_account_info(),
            to: ctx.accounts.senders_token_account0.to_account_info(),
            authority: ctx.accounts.amm_info.to_account_info(),
        };
        let cpi_program0 = ctx.accounts.token_program.to_account_info();
        let cpi_ctx0 = CpiContext::new_with_signer(cpi_program0, cpi_accounts0, signer);
        token::transfer(cpi_ctx0, amount0)?;

        // Transfer token1 from AMM to sender
        let cpi_accounts1 = Transfer {
            from: ctx.accounts.pdas_token_account1.to_account_info(),
            to: ctx.accounts.senders_token_account1.to_account_info(),
            authority: ctx.accounts.amm_info.to_account_info(),
        };
        let cpi_program1 = ctx.accounts.token_program.to_account_info();
        let cpi_ctx1 = CpiContext::new_with_signer(cpi_program1, cpi_accounts1, signer);
        token::transfer(cpi_ctx1, amount1)?;

        Ok(())
    }

    pub fn swap(
        ctx: Context<RedeemOrSwapCtx>,
        is_mint0: bool,
        amount_in: u64,
        min_out_amount: u64,
    ) -> Result<()> {
        require!(amount_in > 0, AmmError::InvalidSwapAmount);

        let amm_info = &mut ctx.accounts.amm_info;

        let (reserve_in, reserve_out) = if is_mint0 {
            (amm_info.reserve0, amm_info.reserve1)
        } else {
            (amm_info.reserve1, amm_info.reserve0)
        };

        // Calculate output amount using constant product formula
        let amount_in_with_fee = (amount_in as u128)
            .checked_mul(997)
            .ok_or(AmmError::MathOverflow)?; // 0.3% fee
        let numerator = amount_in_with_fee
            .checked_mul(reserve_out as u128)
            .ok_or(AmmError::MathOverflow)?;
        let denominator = (reserve_in as u128)
            .checked_mul(1000)
            .ok_or(AmmError::MathOverflow)?
            .checked_add(amount_in_with_fee)
            .ok_or(AmmError::MathOverflow)?;
        
        let amount_out = numerator
            .checked_div(denominator)
            .ok_or(AmmError::DivisionByZero)?
            .try_into()
            .map_err(|_| AmmError::MathOverflow)?;

        require!(amount_out >= min_out_amount, AmmError::SlippageExceeded);
        require!(amount_out <= reserve_out, AmmError::InsufficientLiquidity);

        // Update reserves
        if is_mint0 {
            amm_info.reserve0 = amm_info
                .reserve0
                .checked_add(amount_in)
                .ok_or(AmmError::MathOverflow)?;
            amm_info.reserve1 = amm_info
                .reserve1
                .checked_sub(amount_out)
                .ok_or(AmmError::InsufficientReserves)?;
        } else {
            amm_info.reserve0 = amm_info
                .reserve0
                .checked_sub(amount_out)
                .ok_or(AmmError::InsufficientReserves)?;
            amm_info.reserve1 = amm_info
                .reserve1
                .checked_add(amount_in)
                .ok_or(AmmError::MathOverflow)?;
        }

        // Extract necessary data before mutable borrow ends
        let mint0 = amm_info.mint0;
        let mint1 = amm_info.mint1;
        let bump = ctx.bumps.amm_info;

        // Drop mutable reference
        drop(amm_info);

        // Transfer input tokens from sender to AMM
        let (sender_token_in, pda_token_in) = if is_mint0 {
            (
                ctx.accounts.senders_token_account0.to_account_info(),
                ctx.accounts.pdas_token_account0.to_account_info(),
            )
        } else {
            (
                ctx.accounts.senders_token_account1.to_account_info(),
                ctx.accounts.pdas_token_account1.to_account_info(),
            )
        };

        let cpi_accounts_in = Transfer {
            from: sender_token_in,
            to: pda_token_in,
            authority: ctx.accounts.sender.to_account_info(),
        };
        let cpi_program_in = ctx.accounts.token_program.to_account_info();
        let cpi_ctx_in = CpiContext::new(cpi_program_in, cpi_accounts_in);
        token::transfer(cpi_ctx_in, amount_in)?;

        // For output transfer, use AMM PDA as signer
        let seeds = &[
            b"amm",
            mint0.as_ref(),
            mint1.as_ref(),
            &[bump],
        ];
        let signer = &[&seeds[..]];

        let (pda_token_out, sender_token_out) = if is_mint0 {
            (
                ctx.accounts.pdas_token_account1.to_account_info(),
                ctx.accounts.senders_token_account1.to_account_info(),
            )
        } else {
            (
                ctx.accounts.pdas_token_account0.to_account_info(),
                ctx.accounts.senders_token_account0.to_account_info(),
            )
        };

        let cpi_accounts_out = Transfer {
            from: pda_token_out,
            to: sender_token_out,
            authority: ctx.accounts.amm_info.to_account_info(),
        };
        let cpi_program_out = ctx.accounts.token_program.to_account_info();
        let cpi_ctx_out = CpiContext::new_with_signer(cpi_program_out, cpi_accounts_out, signer);
        token::transfer(cpi_ctx_out, amount_out)?;

        Ok(())
    }
}

// Custom integer square root implementation
fn integer_sqrt(n: u128) -> u128 {
    if n == 0 {
        return 0;
    }
    
    let mut x = n;
    let mut y = (x + 1) / 2;
    while y < x {
        x = y;
        y = (x + n / x) / 2;
    }
    x
}

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

    // The test creates these token accounts
    #[account(
        mut,
        token::mint = mint0,
    )]
    pub token_account0: Account<'info, TokenAccount>,

    #[account(
        mut,
        token::mint = mint1,
    )]
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
        space = 8 + MintedPDA::LEN,
        seeds = [b"minted", sender.key().as_ref()],
        bump
    )]
    pub minted_pda: Account<'info, MintedPDA>,

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

    // These should be the AMM's token accounts
    #[account(
        mut,
        token::mint = mint0,
        constraint = pdas_token_account0.key() == amm_info.token_account0 @ AmmError::InvalidTokenAccount
    )]
    pub pdas_token_account0: Account<'info, TokenAccount>,

    #[account(
        mut,
        token::mint = mint1,
        constraint = pdas_token_account1.key() == amm_info.token_account1 @ AmmError::InvalidTokenAccount
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
    pub minted_pda: Account<'info, MintedPDA>,

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

    // These should be the AMM's token accounts
    #[account(
        mut,
        token::mint = mint0,
        constraint = pdas_token_account0.key() == amm_info.token_account0 @ AmmError::InvalidTokenAccount
    )]
    pub pdas_token_account0: Account<'info, TokenAccount>,

    #[account(
        mut,
        token::mint = mint1,
        constraint = pdas_token_account1.key() == amm_info.token_account1 @ AmmError::InvalidTokenAccount
    )]
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
}

impl AmmInfo {
    pub const LEN: usize = 32 * 4 + 8 * 3 + 1;
}

#[account]
pub struct MintedPDA {
    pub minted: u64,
}

impl MintedPDA {
    pub const LEN: usize = 8;
}

#[error_code]
pub enum AmmError {
    #[msg("Invalid deposit amount")]
    InvalidDepositAmount,
    #[msg("Invalid redeem amount")]
    InvalidRedeemAmount,
    #[msg("Invalid swap amount")]
    InvalidSwapAmount,
    #[msg("Math overflow")]
    MathOverflow,
    #[msg("Division by zero")]
    DivisionByZero,
    #[msg("Insufficient liquidity minted")]
    InsufficientLiquidityMinted,
    #[msg("Insufficient liquidity burned")]
    InsufficientLiquidityBurned,
    #[msg("Insufficient liquidity")]
    InsufficientLiquidity,
    #[msg("Insufficient reserves")]
    InsufficientReserves,
    #[msg("Slippage exceeded")]
    SlippageExceeded,
    #[msg("Invalid token account")]
    InvalidTokenAccount,
}