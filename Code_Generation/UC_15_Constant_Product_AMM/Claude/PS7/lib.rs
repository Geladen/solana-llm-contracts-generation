use anchor_lang::prelude::*;
use anchor_spl::associated_token::AssociatedToken;
use anchor_spl::token::{self, Mint, SetAuthority, Token, TokenAccount, Transfer};
use anchor_spl::token::spl_token;

declare_id!("Brw1opJiqfMBEDN842G733c2zV6iMGe6BqAkVSpTcvrj");

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
        amm_info.ever_deposited = 0;
        amm_info.supply = 0;

        // Transfer ownership of token accounts to AMM PDA
        let mint0_key = ctx.accounts.mint0.key();
        let mint1_key = ctx.accounts.mint1.key();
        let seeds = &[
            b"amm",
            mint0_key.as_ref(),
            mint1_key.as_ref(),
            &[ctx.bumps.amm_info],
        ];
        let signer_seeds = &[&seeds[..]];

        // Set authority for token_account0
        let cpi_accounts0 = SetAuthority {
            account_or_mint: ctx.accounts.token_account0.to_account_info(),
            current_authority: ctx.accounts.initializer.to_account_info(),
        };
        let cpi_ctx0 = CpiContext::new(
            ctx.accounts.token_program.to_account_info(),
            cpi_accounts0,
        );
        token::set_authority(
            cpi_ctx0,
            spl_token::instruction::AuthorityType::AccountOwner,
            Some(ctx.accounts.amm_info.key()),
        )?;

        // Set authority for token_account1
        let cpi_accounts1 = SetAuthority {
            account_or_mint: ctx.accounts.token_account1.to_account_info(),
            current_authority: ctx.accounts.initializer.to_account_info(),
        };
        let cpi_ctx1 = CpiContext::new(
            ctx.accounts.token_program.to_account_info(),
            cpi_accounts1,
        );
        token::set_authority(
            cpi_ctx1,
            spl_token::instruction::AuthorityType::AccountOwner,
            Some(ctx.accounts.amm_info.key()),
        )?;

        Ok(())
    }

    pub fn deposit(ctx: Context<DepositCtx>, amount0: u64, amount1: u64) -> Result<()> {
        require!(amount0 > 0 && amount1 > 0, AmmError::InvalidAmount);

        let amm_info = &mut ctx.accounts.amm_info;
        let minted_pda = &mut ctx.accounts.minted_pda;

        // Calculate liquidity tokens to mint BEFORE transfer
        let liquidity: u64;
        
        if amm_info.supply == 0 {
            // First deposit: liquidity = sqrt(amount0 * amount1)
            liquidity = (amount0 as u128)
                .checked_mul(amount1 as u128)
                .ok_or(AmmError::MathOverflow)?
                .integer_sqrt() as u64;
            
            require!(liquidity > 0, AmmError::InsufficientLiquidity);
        } else {
            // Subsequent deposits: maintain current ratio
            // Use u128 for intermediate calculations to avoid overflow
            let liquidity0 = (amount0 as u128)
                .checked_mul(amm_info.supply as u128)
                .ok_or(AmmError::MathOverflow)?
                .checked_div(amm_info.reserve0 as u128)
                .ok_or(AmmError::DivisionByZero)?;
            
            let liquidity1 = (amount1 as u128)
                .checked_mul(amm_info.supply as u128)
                .ok_or(AmmError::MathOverflow)?
                .checked_div(amm_info.reserve1 as u128)
                .ok_or(AmmError::DivisionByZero)?;
            
            // Take minimum to maintain ratio
            liquidity = liquidity0.min(liquidity1) as u64;
            require!(liquidity > 0, AmmError::InsufficientLiquidity);
        }

        // Transfer tokens from sender to AMM
        let cpi_accounts0 = Transfer {
            from: ctx.accounts.senders_token_account0.to_account_info(),
            to: ctx.accounts.pdas_token_account0.to_account_info(),
            authority: ctx.accounts.sender.to_account_info(),
        };
        let cpi_ctx0 = CpiContext::new(
            ctx.accounts.token_program.to_account_info(),
            cpi_accounts0,
        );
        token::transfer(cpi_ctx0, amount0)?;

        let cpi_accounts1 = Transfer {
            from: ctx.accounts.senders_token_account1.to_account_info(),
            to: ctx.accounts.pdas_token_account1.to_account_info(),
            authority: ctx.accounts.sender.to_account_info(),
        };
        let cpi_ctx1 = CpiContext::new(
            ctx.accounts.token_program.to_account_info(),
            cpi_accounts1,
        );
        token::transfer(cpi_ctx1, amount1)?;

        // Reload token accounts to get actual balances
        ctx.accounts.pdas_token_account0.reload()?;
        ctx.accounts.pdas_token_account1.reload()?;

        // Update reserves to match actual token account balances
        amm_info.reserve0 = ctx.accounts.pdas_token_account0.amount;
        amm_info.reserve1 = ctx.accounts.pdas_token_account1.amount;
        amm_info.supply = amm_info.supply
            .checked_add(liquidity)
            .ok_or(AmmError::MathOverflow)?;
        amm_info.ever_deposited = amm_info.ever_deposited
            .checked_add(1)
            .ok_or(AmmError::MathOverflow)?;

        // Update user's minted liquidity
        minted_pda.minted = minted_pda.minted
            .checked_add(liquidity)
            .ok_or(AmmError::MathOverflow)?;

        Ok(())
    }

    pub fn redeem(ctx: Context<RedeemOrSwapCtx>, amount: u64) -> Result<()> {
        require!(amount > 0, AmmError::InvalidAmount);

        require!(ctx.accounts.minted_pda.minted >= amount, AmmError::InsufficientLiquidity);
        require!(ctx.accounts.amm_info.supply >= amount, AmmError::InsufficientLiquidity);

        // Calculate token amounts to return using u128 for precision
        let amount0 = ((amount as u128)
            .checked_mul(ctx.accounts.amm_info.reserve0 as u128)
            .ok_or(AmmError::MathOverflow)?
            .checked_div(ctx.accounts.amm_info.supply as u128)
            .ok_or(AmmError::DivisionByZero)?) as u64;

        let amount1 = ((amount as u128)
            .checked_mul(ctx.accounts.amm_info.reserve1 as u128)
            .ok_or(AmmError::MathOverflow)?
            .checked_div(ctx.accounts.amm_info.supply as u128)
            .ok_or(AmmError::DivisionByZero)?) as u64;

        require!(amount0 > 0 && amount1 > 0, AmmError::InsufficientLiquidity);

        // Prepare PDA signer
        let mint0_key = ctx.accounts.mint0.key();
        let mint1_key = ctx.accounts.mint1.key();
        let seeds = &[
            b"amm",
            mint0_key.as_ref(),
            mint1_key.as_ref(),
            &[ctx.bumps.amm_info],
        ];
        let signer_seeds = &[&seeds[..]];

        // Transfer tokens from AMM to sender
        let cpi_accounts0 = Transfer {
            from: ctx.accounts.pdas_token_account0.to_account_info(),
            to: ctx.accounts.senders_token_account0.to_account_info(),
            authority: ctx.accounts.amm_info.to_account_info(),
        };
        let cpi_ctx0 = CpiContext::new_with_signer(
            ctx.accounts.token_program.to_account_info(),
            cpi_accounts0,
            signer_seeds,
        );
        token::transfer(cpi_ctx0, amount0)?;

        let cpi_accounts1 = Transfer {
            from: ctx.accounts.pdas_token_account1.to_account_info(),
            to: ctx.accounts.senders_token_account1.to_account_info(),
            authority: ctx.accounts.amm_info.to_account_info(),
        };
        let cpi_ctx1 = CpiContext::new_with_signer(
            ctx.accounts.token_program.to_account_info(),
            cpi_accounts1,
            signer_seeds,
        );
        token::transfer(cpi_ctx1, amount1)?;

        // Update reserves and supply
        let amm_info = &mut ctx.accounts.amm_info;
        amm_info.reserve0 = amm_info.reserve0
            .checked_sub(amount0)
            .ok_or(AmmError::MathOverflow)?;
        amm_info.reserve1 = amm_info.reserve1
            .checked_sub(amount1)
            .ok_or(AmmError::MathOverflow)?;
        amm_info.supply = amm_info.supply
            .checked_sub(amount)
            .ok_or(AmmError::MathOverflow)?;

        // Update user's minted liquidity
        let minted_pda = &mut ctx.accounts.minted_pda;
        minted_pda.minted = minted_pda.minted
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
        require!(amount_in > 0, AmmError::InvalidAmount);

        // Calculate output amount using constant product formula
        // x * y = k (constant)
        // amount_out = (reserve_out * amount_in) / (reserve_in + amount_in)
        let (reserve_in, reserve_out) = if is_mint0 {
            (ctx.accounts.amm_info.reserve0, ctx.accounts.amm_info.reserve1)
        } else {
            (ctx.accounts.amm_info.reserve1, ctx.accounts.amm_info.reserve0)
        };

        let amount_out = ((reserve_out as u128)
            .checked_mul(amount_in as u128)
            .ok_or(AmmError::MathOverflow)?
            .checked_div(
                (reserve_in as u128)
                    .checked_add(amount_in as u128)
                    .ok_or(AmmError::MathOverflow)?
            )
            .ok_or(AmmError::DivisionByZero)?) as u64;

        require!(amount_out >= min_out_amount, AmmError::SlippageExceeded);
        require!(amount_out > 0, AmmError::InsufficientOutputAmount);

        // Prepare PDA signer
        let mint0_key = ctx.accounts.mint0.key();
        let mint1_key = ctx.accounts.mint1.key();
        let seeds = &[
            b"amm",
            mint0_key.as_ref(),
            mint1_key.as_ref(),
            &[ctx.bumps.amm_info],
        ];
        let signer_seeds = &[&seeds[..]];

        if is_mint0 {
            // Swap mint0 for mint1
            // Transfer mint0 from sender to AMM
            let cpi_accounts_in = Transfer {
                from: ctx.accounts.senders_token_account0.to_account_info(),
                to: ctx.accounts.pdas_token_account0.to_account_info(),
                authority: ctx.accounts.sender.to_account_info(),
            };
            let cpi_ctx_in = CpiContext::new(
                ctx.accounts.token_program.to_account_info(),
                cpi_accounts_in,
            );
            token::transfer(cpi_ctx_in, amount_in)?;

            // Transfer mint1 from AMM to sender
            let cpi_accounts_out = Transfer {
                from: ctx.accounts.pdas_token_account1.to_account_info(),
                to: ctx.accounts.senders_token_account1.to_account_info(),
                authority: ctx.accounts.amm_info.to_account_info(),
            };
            let cpi_ctx_out = CpiContext::new_with_signer(
                ctx.accounts.token_program.to_account_info(),
                cpi_accounts_out,
                signer_seeds,
            );
            token::transfer(cpi_ctx_out, amount_out)?;

            // Update reserves
            let amm_info = &mut ctx.accounts.amm_info;
            amm_info.reserve0 = amm_info.reserve0
                .checked_add(amount_in)
                .ok_or(AmmError::MathOverflow)?;
            amm_info.reserve1 = amm_info.reserve1
                .checked_sub(amount_out)
                .ok_or(AmmError::MathOverflow)?;
        } else {
            // Swap mint1 for mint0
            // Transfer mint1 from sender to AMM
            let cpi_accounts_in = Transfer {
                from: ctx.accounts.senders_token_account1.to_account_info(),
                to: ctx.accounts.pdas_token_account1.to_account_info(),
                authority: ctx.accounts.sender.to_account_info(),
            };
            let cpi_ctx_in = CpiContext::new(
                ctx.accounts.token_program.to_account_info(),
                cpi_accounts_in,
            );
            token::transfer(cpi_ctx_in, amount_in)?;

            // Transfer mint0 from AMM to sender
            let cpi_accounts_out = Transfer {
                from: ctx.accounts.pdas_token_account0.to_account_info(),
                to: ctx.accounts.senders_token_account0.to_account_info(),
                authority: ctx.accounts.amm_info.to_account_info(),
            };
            let cpi_ctx_out = CpiContext::new_with_signer(
                ctx.accounts.token_program.to_account_info(),
                cpi_accounts_out,
                signer_seeds,
            );
            token::transfer(cpi_ctx_out, amount_out)?;

            // Update reserves
            let amm_info = &mut ctx.accounts.amm_info;
            amm_info.reserve1 = amm_info.reserve1
                .checked_add(amount_in)
                .ok_or(AmmError::MathOverflow)?;
            amm_info.reserve0 = amm_info.reserve0
                .checked_sub(amount_out)
                .ok_or(AmmError::MathOverflow)?;
        }

        Ok(())
    }
}

// Context Structures

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

    #[account(
        mut,
        token::mint = mint0,
        token::authority = initializer
    )]
    pub token_account0: Account<'info, TokenAccount>,

    #[account(
        mut,
        token::mint = mint1,
        token::authority = initializer
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
        space = 8 + MintedPDA::INIT_SPACE,
        seeds = [b"minted", sender.key().as_ref()],
        bump
    )]
    pub minted_pda: Account<'info, MintedPDA>,

    #[account(
        mut,
        token::mint = mint0,
        token::authority = sender
    )]
    pub senders_token_account0: Account<'info, TokenAccount>,

    #[account(
        mut,
        token::mint = mint1,
        token::authority = sender
    )]
    pub senders_token_account1: Account<'info, TokenAccount>,

    #[account(
        mut,
        constraint = pdas_token_account0.key() == amm_info.token_account0 @ AmmError::InvalidTokenAccount,
        token::mint = mint0,
        token::authority = amm_info
    )]
    pub pdas_token_account0: Account<'info, TokenAccount>,

    #[account(
        mut,
        constraint = pdas_token_account1.key() == amm_info.token_account1 @ AmmError::InvalidTokenAccount,
        token::mint = mint1,
        token::authority = amm_info
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
        token::mint = mint0,
        token::authority = sender
    )]
    pub senders_token_account0: Account<'info, TokenAccount>,

    #[account(
        mut,
        token::mint = mint1,
        token::authority = sender
    )]
    pub senders_token_account1: Account<'info, TokenAccount>,

    #[account(
        mut,
        constraint = pdas_token_account0.key() == amm_info.token_account0 @ AmmError::InvalidTokenAccount,
        token::mint = mint0,
        token::authority = amm_info
    )]
    pub pdas_token_account0: Account<'info, TokenAccount>,

    #[account(
        mut,
        constraint = pdas_token_account1.key() == amm_info.token_account1 @ AmmError::InvalidTokenAccount,
        token::mint = mint1,
        token::authority = amm_info
    )]
    pub pdas_token_account1: Account<'info, TokenAccount>,

    pub token_program: Program<'info, Token>,
    pub system_program: Program<'info, System>,
    pub associated_token_program: Program<'info, AssociatedToken>,
}

// Account Structures

#[account]
#[derive(InitSpace)]
pub struct AmmInfo {
    pub mint0: Pubkey,
    pub mint1: Pubkey,
    pub token_account0: Pubkey,
    pub token_account1: Pubkey,
    pub reserve0: u64,
    pub reserve1: u64,
    pub ever_deposited: u64,
    pub supply: u64,
}

#[account]
#[derive(InitSpace)]
pub struct MintedPDA {
    pub minted: u64,
}

// Error Codes

#[error_code]
pub enum AmmError {
    #[msg("Invalid amount provided")]
    InvalidAmount,
    #[msg("Insufficient liquidity")]
    InsufficientLiquidity,
    #[msg("Math overflow")]
    MathOverflow,
    #[msg("Division by zero")]
    DivisionByZero,
    #[msg("Slippage tolerance exceeded")]
    SlippageExceeded,
    #[msg("Insufficient output amount")]
    InsufficientOutputAmount,
    #[msg("Invalid token account")]
    InvalidTokenAccount,
}

// Helper trait for integer square root
trait IntegerSqrt {
    fn integer_sqrt(self) -> Self;
}

impl IntegerSqrt for u128 {
    fn integer_sqrt(self) -> Self {
        if self < 2 {
            return self;
        }
        
        let mut x = self;
        let mut y = (x + 1) / 2;
        
        while y < x {
            x = y;
            y = (x + self / x) / 2;
        }
        
        x
    }
}