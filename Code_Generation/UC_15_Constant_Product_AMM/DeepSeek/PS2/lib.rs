use anchor_lang::prelude::*;
use anchor_spl::{
    associated_token::AssociatedToken,
    token::{self, Mint, Token, TokenAccount, Transfer},
};

declare_id!("FyQ42rXXBbkfzbGrm2USZUpBRQV94srb96jRzG1CqP7N");

#[program]
pub mod constant_product_amm {
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
        
        // Transfer tokens from sender to AMM PDA
        let cpi_accounts0 = Transfer {
            from: ctx.accounts.senders_token_account0.to_account_info(),
            to: ctx.accounts.pdas_token_account0.to_account_info(),
            authority: ctx.accounts.sender.to_account_info(),
        };
        let cpi_program0 = ctx.accounts.token_program.to_account_info();
        let cpi_ctx0 = CpiContext::new(cpi_program0, cpi_accounts0);
        token::transfer(cpi_ctx0, amount0)?;

        let cpi_accounts1 = Transfer {
            from: ctx.accounts.senders_token_account1.to_account_info(),
            to: ctx.accounts.pdas_token_account1.to_account_info(),
            authority: ctx.accounts.sender.to_account_info(),
        };
        let cpi_program1 = ctx.accounts.token_program.to_account_info();
        let cpi_ctx1 = CpiContext::new(cpi_program1, cpi_accounts1);
        token::transfer(cpi_ctx1, amount1)?;

        // Calculate liquidity tokens to mint
        let liquidity_tokens = if !amm_info.ever_deposited {
            amm_info.ever_deposited = true;
            // Initial deposit: sqrt(amount0 * amount1) using integer square root
            let product = (amount0 as u128)
                .checked_mul(amount1 as u128)
                .ok_or(AmmError::MathOverflow)?;
            integer_sqrt(product).try_into().map_err(|_| AmmError::MathOverflow)?
        } else {
            // Calculate proportional to existing reserves
            let total_supply = amm_info.supply;
            let liquidity_from_0 = (amount0 as u128)
                .checked_mul(total_supply as u128)
                .ok_or(AmmError::MathOverflow)?
                .checked_div(amm_info.reserve0 as u128)
                .ok_or(AmmError::MathOverflow)?;
            
            let liquidity_from_1 = (amount1 as u128)
                .checked_mul(total_supply as u128)
                .ok_or(AmmError::MathOverflow)?
                .checked_div(amm_info.reserve1 as u128)
                .ok_or(AmmError::MathOverflow)?;
            
            // Take minimum to maintain ratio
            std::cmp::min(
                liquidity_from_0.try_into().map_err(|_| AmmError::MathOverflow)?,
                liquidity_from_1.try_into().map_err(|_| AmmError::MathOverflow)?,
            )
        };

        require!(liquidity_tokens > 0, AmmError::InsufficientLiquidityMinted);

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
            .checked_add(liquidity_tokens)
            .ok_or(AmmError::MathOverflow)?;

        // Update minted PDA
        let minted_pda = &mut ctx.accounts.minted_pda;
        minted_pda.minted = minted_pda
            .minted
            .checked_add(liquidity_tokens)
            .ok_or(AmmError::MathOverflow)?;

        Ok(())
    }

    pub fn redeem(ctx: Context<RedeemOrSwapCtx>, amount: u64) -> Result<()> {
        require!(amount > 0, AmmError::InvalidRedeemAmount);

        // Extract all needed data BEFORE mutable borrows
        let mint0_key = ctx.accounts.amm_info.mint0;
        let mint1_key = ctx.accounts.amm_info.mint1;
        let reserve0 = ctx.accounts.amm_info.reserve0;
        let reserve1 = ctx.accounts.amm_info.reserve1;
        let total_supply = ctx.accounts.amm_info.supply;
        let bump = ctx.bumps.amm_info;

        let minted_pda = &mut ctx.accounts.minted_pda;

        // Check if user has enough liquidity tokens
        require!(
            minted_pda.minted >= amount,
            AmmError::InsufficientLiquidityTokens
        );

        // Calculate token amounts to return
        let amount0 = (reserve0 as u128)
            .checked_mul(amount as u128)
            .ok_or(AmmError::MathOverflow)?
            .checked_div(total_supply as u128)
            .ok_or(AmmError::MathOverflow)?
            .try_into()
            .map_err(|_| AmmError::MathOverflow)?;
        
        let amount1 = (reserve1 as u128)
            .checked_mul(amount as u128)
            .ok_or(AmmError::MathOverflow)?
            .checked_div(total_supply as u128)
            .ok_or(AmmError::MathOverflow)?
            .try_into()
            .map_err(|_| AmmError::MathOverflow)?;

        require!(amount0 > 0 && amount1 > 0, AmmError::InsufficientLiquidity);

        // Transfer tokens from AMM to sender
        let seeds = &[
            b"amm",
            mint0_key.as_ref(),
            mint1_key.as_ref(),
            &[bump],
        ];
        let signer = &[&seeds[..]];

        let cpi_accounts0 = Transfer {
            from: ctx.accounts.pdas_token_account0.to_account_info(),
            to: ctx.accounts.senders_token_account0.to_account_info(),
            authority: ctx.accounts.amm_info.to_account_info(),
        };
        let cpi_program0 = ctx.accounts.token_program.to_account_info();
        let cpi_ctx0 = CpiContext::new_with_signer(cpi_program0, cpi_accounts0, signer);
        token::transfer(cpi_ctx0, amount0)?;

        let cpi_accounts1 = Transfer {
            from: ctx.accounts.pdas_token_account1.to_account_info(),
            to: ctx.accounts.senders_token_account1.to_account_info(),
            authority: ctx.accounts.amm_info.to_account_info(),
        };
        let cpi_program1 = ctx.accounts.token_program.to_account_info();
        let cpi_ctx1 = CpiContext::new_with_signer(cpi_program1, cpi_accounts1, signer);
        token::transfer(cpi_ctx1, amount1)?;

        // Now update the amm_info after all immutable accesses are done
        let amm_info = &mut ctx.accounts.amm_info;
        amm_info.reserve0 = amm_info
            .reserve0
            .checked_sub(amount0)
            .ok_or(AmmError::MathOverflow)?;
        amm_info.reserve1 = amm_info
            .reserve1
            .checked_sub(amount1)
            .ok_or(AmmError::MathOverflow)?;
        amm_info.supply = amm_info
            .supply
            .checked_sub(amount)
            .ok_or(AmmError::MathOverflow)?;

        // Update minted PDA
        minted_pda.minted = minted_pda
            .minted
            .checked_sub(amount)
            .ok_or(AmmError::MathOverflow)?;

        Ok(())
    }

    pub fn swap(
        ctx: Context<RedeemOrSwapCtx>,
        is_mint0: bool,
        amount_in: u64,
        min_out_amount: u64,
    ) -> Result<()> {
        require!(amount_in > 0, AmmError::InvalidSwapAmount);

        // Extract all needed data BEFORE mutable borrows
        let mint0_key = ctx.accounts.amm_info.mint0;
        let mint1_key = ctx.accounts.amm_info.mint1;
        let reserve0 = ctx.accounts.amm_info.reserve0;
        let reserve1 = ctx.accounts.amm_info.reserve1;
        let bump = ctx.bumps.amm_info;

        let (reserve_in, reserve_out) = if is_mint0 {
            (reserve0, reserve1)
        } else {
            (reserve1, reserve0)
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
            .ok_or(AmmError::MathOverflow)?
            .try_into()
            .map_err(|_| AmmError::MathOverflow)?;

        require!(amount_out >= min_out_amount, AmmError::SlippageExceeded);
        require!(amount_out <= reserve_out, AmmError::InsufficientLiquidity);

        // Transfer input tokens from sender to AMM
        let (senders_token_in, senders_token_out) = if is_mint0 {
            (
                &ctx.accounts.senders_token_account0,
                &ctx.accounts.senders_token_account1,
            )
        } else {
            (
                &ctx.accounts.senders_token_account1,
                &ctx.accounts.senders_token_account0,
            )
        };

        let (token_account_in, token_account_out) = if is_mint0 {
            (
                &ctx.accounts.pdas_token_account0,
                &ctx.accounts.pdas_token_account1,
            )
        } else {
            (
                &ctx.accounts.pdas_token_account1,
                &ctx.accounts.pdas_token_account0,
            )
        };

        let cpi_accounts_in = Transfer {
            from: senders_token_in.to_account_info(),
            to: token_account_in.to_account_info(),
            authority: ctx.accounts.sender.to_account_info(),
        };
        let cpi_program_in = ctx.accounts.token_program.to_account_info();
        let cpi_ctx_in = CpiContext::new(cpi_program_in, cpi_accounts_in);
        token::transfer(cpi_ctx_in, amount_in)?;

        // Transfer output tokens from AMM to sender
        let seeds = &[
            b"amm",
            mint0_key.as_ref(),
            mint1_key.as_ref(),
            &[bump],
        ];
        let signer = &[&seeds[..]];

        let cpi_accounts_out = Transfer {
            from: token_account_out.to_account_info(),
            to: senders_token_out.to_account_info(),
            authority: ctx.accounts.amm_info.to_account_info(),
        };
        let cpi_program_out = ctx.accounts.token_program.to_account_info();
        let cpi_ctx_out = CpiContext::new_with_signer(cpi_program_out, cpi_accounts_out, signer);
        token::transfer(cpi_ctx_out, amount_out)?;

        // Now update the amm_info after all immutable accesses are done
        let amm_info = &mut ctx.accounts.amm_info;
        if is_mint0 {
            amm_info.reserve0 = amm_info
                .reserve0
                .checked_add(amount_in)
                .ok_or(AmmError::MathOverflow)?;
            amm_info.reserve1 = amm_info
                .reserve1
                .checked_sub(amount_out)
                .ok_or(AmmError::MathOverflow)?;
        } else {
            amm_info.reserve0 = amm_info
                .reserve0
                .checked_sub(amount_out)
                .ok_or(AmmError::MathOverflow)?;
            amm_info.reserve1 = amm_info
                .reserve1
                .checked_add(amount_in)
                .ok_or(AmmError::MathOverflow)?;
        }

        Ok(())
    }
}

// Integer square root implementation (Babylonian method)
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
        space = 8 + AmmInfo::INIT_SPACE,
        seeds = [b"amm", mint0.key().as_ref(), mint1.key().as_ref()],
        bump
    )]
    pub amm_info: Account<'info, AmmInfo>,

    pub mint0: Account<'info, Mint>,
    pub mint1: Account<'info, Mint>,

    // Create token accounts with AMM PDA as authority from the start
    #[account(
        init,
        payer = initializer,
        token::mint = mint0,
        token::authority = amm_info
    )]
    pub token_account0: Account<'info, TokenAccount>,

    #[account(
        init,
        payer = initializer,
        token::mint = mint1,
        token::authority = amm_info
    )]
    pub token_account1: Account<'info, TokenAccount>,

    pub token_program: Program<'info, Token>,
    pub system_program: Program<'info, System>,
    pub rent: Sysvar<'info, Rent>,
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
        space = 8 + MintedPDA::INIT_SPACE,
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

    #[account(
        mut,
        associated_token::mint = mint0,
        associated_token::authority = amm_info
    )]
    pub pdas_token_account0: Account<'info, TokenAccount>,

    #[account(
        mut,
        associated_token::mint = mint1,
        associated_token::authority = amm_info
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

    #[account(
        mut,
        associated_token::mint = mint0,
        associated_token::authority = amm_info
    )]
    pub pdas_token_account0: Account<'info, TokenAccount>,

    #[account(
        mut,
        associated_token::mint = mint1,
        associated_token::authority = amm_info
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
}

#[account]
#[derive(InitSpace)]
pub struct MintedPDA {
    pub minted: u64,
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
    #[msg("Insufficient liquidity minted")]
    InsufficientLiquidityMinted,
    #[msg("Insufficient liquidity tokens")]
    InsufficientLiquidityTokens,
    #[msg("Insufficient liquidity")]
    InsufficientLiquidity,
    #[msg("Slippage exceeded")]
    SlippageExceeded,
}