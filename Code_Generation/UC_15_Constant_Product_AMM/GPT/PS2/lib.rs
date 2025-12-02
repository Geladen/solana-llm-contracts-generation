use anchor_lang::prelude::*;
use anchor_spl::token::{self, Token, TokenAccount, Mint, SetAuthority};
use anchor_spl::associated_token::AssociatedToken;


declare_id!("6iWaDuUiGdAWfuXcStzETqD7eZ5ct8feikR9frcghaSj");

#[program]
pub mod constant_product_amm {
    use super::*;

    // Initialize AMM
    pub fn initialize(ctx: Context<InitializeCtx>) -> Result<()> {
        let initializer = &ctx.accounts.initializer;
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

        // Bump is stored automatically by Anchor
        amm_info.bump = ctx.bumps["amm_info"];

        // Transfer ownership of token accounts to PDA
        let cpi_ctx0 = CpiContext::new(
            ctx.accounts.token_program.to_account_info(),
            SetAuthority {
                account_or_mint: ctx.accounts.token_account0.to_account_info(),
                current_authority: initializer.to_account_info(),
            },
        );
        token::set_authority(cpi_ctx0, token::spl_token::instruction::AuthorityType::AccountOwner, Some(amm_info.key()))?;

        let cpi_ctx1 = CpiContext::new(
            ctx.accounts.token_program.to_account_info(),
            SetAuthority {
                account_or_mint: ctx.accounts.token_account1.to_account_info(),
                current_authority: initializer.to_account_info(),
            },
        );
        token::set_authority(cpi_ctx1, token::spl_token::instruction::AuthorityType::AccountOwner, Some(amm_info.key()))?;

        Ok(())
    }

    // Deposit liquidity
    pub fn deposit(ctx: Context<DepositCtx>, amount0: u64, amount1: u64) -> Result<()> {
        let amm_info = &mut ctx.accounts.amm_info;
        let minted_pda = &mut ctx.accounts.minted_pda;

        // Transfer tokens from sender to AMM PDA
        let cpi_accounts0 = Transfer {
            from: ctx.accounts.senders_token_account0.to_account_info(),
            to: ctx.accounts.pdas_token_account0.to_account_info(),
            authority: ctx.accounts.sender.to_account_info(),
        };
        token::transfer(
            CpiContext::new(ctx.accounts.token_program.to_account_info(), cpi_accounts0),
            amount0,
        )?;

        let cpi_accounts1 = Transfer {
            from: ctx.accounts.senders_token_account1.to_account_info(),
            to: ctx.accounts.pdas_token_account1.to_account_info(),
            authority: ctx.accounts.sender.to_account_info(),
        };
        token::transfer(
            CpiContext::new(ctx.accounts.token_program.to_account_info(), cpi_accounts1),
            amount1,
        )?;

        // Calculate minted LP tokens
        let minted = if !amm_info.ever_deposited {
            let minted_float = ((amount0 as u128) * (amount1 as u128)).integer_sqrt();
            amm_info.ever_deposited = true;
            minted_float as u64
        } else {
            let minted0 = (amount0 as u128)
                .checked_mul(amm_info.supply as u128)
                .unwrap()
                .checked_div(amm_info.reserve0 as u128)
                .unwrap();
            let minted1 = (amount1 as u128)
                .checked_mul(amm_info.supply as u128)
                .unwrap()
                .checked_div(amm_info.reserve1 as u128)
                .unwrap();
            std::cmp::min(minted0, minted1) as u64
        };

        // Update reserves and supply
        amm_info.reserve0 = amm_info.reserve0.checked_add(amount0).unwrap();
        amm_info.reserve1 = amm_info.reserve1.checked_add(amount1).unwrap();
        amm_info.supply = amm_info.supply.checked_add(minted).unwrap();

        // Update minted PDA
        minted_pda.minted = minted_pda.minted.checked_add(minted).unwrap();

        Ok(())
    }

    // Redeem liquidity
    // Redeem liquidity
    pub fn redeem(ctx: Context<RedeemOrSwapCtx>, amount: u64) -> Result<()> {
        let minted_pda = &mut ctx.accounts.minted_pda;

        require!(minted_pda.minted >= amount, ErrorCode::InsufficientLPTokens);

        // Compute withdraw amounts
        let (amount0, amount1) = {
            let amm_info = &ctx.accounts.amm_info; // immutable borrow just for calculation
            let amount0 = (amount as u128)
                .checked_mul(amm_info.reserve0 as u128)
                .unwrap()
                .checked_div(amm_info.supply as u128)
                .unwrap() as u64;
            let amount1 = (amount as u128)
                .checked_mul(amm_info.reserve1 as u128)
                .unwrap()
                .checked_div(amm_info.supply as u128)
                .unwrap() as u64;
            (amount0, amount1)
        };

        // Bind keys for PDA seeds
        let mint0_key = ctx.accounts.mint0.key();
        let mint1_key = ctx.accounts.mint1.key();
        let seeds = &[
            b"amm",
            mint0_key.as_ref(),
            mint1_key.as_ref(),
            &[ctx.accounts.amm_info.bump],
        ];

        // Transfer tokens from AMM to sender
        let cpi_accounts0 = Transfer {
            from: ctx.accounts.pdas_token_account0.to_account_info(),
            to: ctx.accounts.senders_token_account0.to_account_info(),
            authority: ctx.accounts.amm_info.to_account_info(),
        };
        token::transfer(
            CpiContext::new_with_signer(ctx.accounts.token_program.to_account_info(), cpi_accounts0, &[seeds]),
            amount0,
        )?;

        let cpi_accounts1 = Transfer {
            from: ctx.accounts.pdas_token_account1.to_account_info(),
            to: ctx.accounts.senders_token_account1.to_account_info(),
            authority: ctx.accounts.amm_info.to_account_info(),
        };
        token::transfer(
            CpiContext::new_with_signer(ctx.accounts.token_program.to_account_info(), cpi_accounts1, &[seeds]),
            amount1,
        )?;

        // Update mutable state in separate scope
        {
            let amm_info = &mut ctx.accounts.amm_info;
            amm_info.reserve0 = amm_info.reserve0.checked_sub(amount0).unwrap();
            amm_info.reserve1 = amm_info.reserve1.checked_sub(amount1).unwrap();
            amm_info.supply = amm_info.supply.checked_sub(amount).unwrap();
        }

        // Update minted PDA
        minted_pda.minted = minted_pda.minted.checked_sub(amount).unwrap();

        Ok(())
    }

    // Swap tokens
    pub fn swap(ctx: Context<RedeemOrSwapCtx>, is_mint0: bool, amount_in: u64, min_out_amount: u64) -> Result<()> {
        // Compute swap amounts using immutable borrow
        let (amount_out, new_reserves) = {
            let amm_info = &ctx.accounts.amm_info;
            let (reserve_in, reserve_out) = if is_mint0 {
                (amm_info.reserve0 as u128, amm_info.reserve1 as u128)
            } else {
                (amm_info.reserve1 as u128, amm_info.reserve0 as u128)
            };
            let amount_in_u128 = amount_in as u128;
            let k = reserve_in.checked_mul(reserve_out).unwrap();
            let new_reserve_in = reserve_in.checked_add(amount_in_u128).unwrap();
            let new_reserve_out = k.checked_div(new_reserve_in).unwrap();
            let amount_out = reserve_out.checked_sub(new_reserve_out).unwrap() as u64;

            require!(amount_out >= min_out_amount, ErrorCode::SlippageExceeded);

            if is_mint0 {
                (amount_out, (reserve_in + amount_in_u128, reserve_out - new_reserve_out))
            } else {
                (amount_out, (reserve_out - new_reserve_out, reserve_in + amount_in_u128))
            }
        };

        // Bind PDA seeds
        let mint0_key = ctx.accounts.mint0.key();
        let mint1_key = ctx.accounts.mint1.key();
        let seeds = &[
            b"amm",
            mint0_key.as_ref(),
            mint1_key.as_ref(),
            &[ctx.accounts.amm_info.bump],
        ];

        // Transfer input tokens from sender to AMM
        let cpi_accounts_in = Transfer {
            from: if is_mint0 {
                ctx.accounts.senders_token_account0.to_account_info()
            } else {
                ctx.accounts.senders_token_account1.to_account_info()
            },
            to: if is_mint0 {
                ctx.accounts.pdas_token_account0.to_account_info()
            } else {
                ctx.accounts.pdas_token_account1.to_account_info()
            },
            authority: ctx.accounts.sender.to_account_info(),
        };
        token::transfer(
            CpiContext::new(ctx.accounts.token_program.to_account_info(), cpi_accounts_in),
            amount_in,
        )?;

        // Transfer output tokens from AMM to sender
        let cpi_accounts_out = Transfer {
            from: if is_mint0 {
                ctx.accounts.pdas_token_account1.to_account_info()
            } else {
                ctx.accounts.pdas_token_account0.to_account_info()
            },
            to: if is_mint0 {
                ctx.accounts.senders_token_account1.to_account_info()
            } else {
                ctx.accounts.senders_token_account0.to_account_info()
            },
            authority: ctx.accounts.amm_info.to_account_info(),
        };
        token::transfer(
            CpiContext::new_with_signer(ctx.accounts.token_program.to_account_info(), cpi_accounts_out, &[seeds]),
            amount_out,
        )?;

        // Update mutable state in separate scope
        {
            let amm_info = &mut ctx.accounts.amm_info;
            if is_mint0 {
                amm_info.reserve0 = amm_info.reserve0.checked_add(amount_in).unwrap();
                amm_info.reserve1 = amm_info.reserve1.checked_sub(amount_out).unwrap();
            } else {
                amm_info.reserve1 = amm_info.reserve1.checked_add(amount_in).unwrap();
                amm_info.reserve0 = amm_info.reserve0.checked_sub(amount_out).unwrap();
            }
        }

        Ok(())
    }
}

// --------------------------- Contexts ---------------------------

#[derive(Accounts)]
pub struct InitializeCtx<'info> {
    #[account(mut)]
    pub initializer: Signer<'info>,

    #[account(
        init,
        seeds = [b"amm", mint0.key().as_ref(), mint1.key().as_ref()],
        bump,
        payer = initializer,
        space = 8 + 176
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
    pub sender: Signer<'info>, // signer

    pub mint0: Account<'info, Mint>,
    pub mint1: Account<'info, Mint>,

    #[account(mut, seeds = [b"amm", mint0.key().as_ref(), mint1.key().as_ref()], bump)]
    pub amm_info: Account<'info, AmmInfo>,

    #[account(mut, seeds = [b"minted", sender.key().as_ref()], bump)]
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
    pub sender: Signer<'info>, // signer

    pub mint0: Account<'info, Mint>,
    pub mint1: Account<'info, Mint>,

    #[account(mut, seeds = [b"amm", mint0.key().as_ref(), mint1.key().as_ref()], bump)]
    pub amm_info: Account<'info, AmmInfo>,

    #[account(mut, seeds = [b"minted", sender.key().as_ref()], bump)]
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


// --------------------------- Accounts ---------------------------

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
    pub bump: u8,
}

// --------------------------- Errors ---------------------------

#[error_code]
pub enum ErrorCode {
    #[msg("Not enough LP tokens")]
    InsufficientLPTokens,
    #[msg("Slippage exceeded")]
    SlippageExceeded,
}

// --------------------------- Helpers ---------------------------

trait IntegerSquareRoot {
    fn integer_sqrt(self) -> Self;
}

impl IntegerSquareRoot for u128 {
    fn integer_sqrt(self) -> Self {
        let mut x = self;
        let mut y = (x + 1) / 2;
        while y < x {
            x = y;
            y = (x + self / x) / 2;
        }
        x
    }
}
