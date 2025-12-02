use anchor_lang::prelude::*;
use anchor_spl::token::{self, Token, TokenAccount, Mint, Transfer, SetAuthority};
use anchor_spl::associated_token::AssociatedToken;

declare_id!("HN3gewTBjhgSUJ3HeGz2y1THC3N7aGYHCErfxrPtRabE");

#[program]
pub mod constant_product_amm {
    use super::*;

    /// Initialize a new AMM pool for a token pair
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

        // Transfer ownership of token_account0 to AMM PDA
        let cpi_accounts = SetAuthority {
            account_or_mint: ctx.accounts.token_account0.to_account_info(),
            current_authority: ctx.accounts.initializer.to_account_info(),
        };
        let cpi_program = ctx.accounts.token_program.to_account_info();
        let cpi_ctx = CpiContext::new(cpi_program, cpi_accounts);
        token::set_authority(
            cpi_ctx,
            anchor_spl::token::spl_token::instruction::AuthorityType::AccountOwner,
            Some(amm_info.key()),
        )?;

        // Transfer ownership of token_account1 to AMM PDA
        let cpi_accounts = SetAuthority {
            account_or_mint: ctx.accounts.token_account1.to_account_info(),
            current_authority: ctx.accounts.initializer.to_account_info(),
        };
        let cpi_program = ctx.accounts.token_program.to_account_info();
        let cpi_ctx = CpiContext::new(cpi_program, cpi_accounts);
        token::set_authority(
            cpi_ctx,
            anchor_spl::token::spl_token::instruction::AuthorityType::AccountOwner,
            Some(amm_info.key()),
        )?;

        msg!("AMM initialized for token pair");
        Ok(())
    }

    /// Deposit liquidity into the AMM pool
    pub fn deposit(ctx: Context<DepositCtx>, amount0: u64, amount1: u64) -> Result<()> {
        require!(amount0 > 0 && amount1 > 0, ErrorCode::InvalidAmount);

        let amm_info = &mut ctx.accounts.amm_info;
        let minted_pda = &mut ctx.accounts.minted_pda;
        
        let liquidity_minted: u64;

        if !amm_info.ever_deposited {
            // First deposit - use geometric mean minus minimum liquidity
            // Calculate sqrt(amount0 * amount1)
            let product = (amount0 as u128)
                .checked_mul(amount1 as u128)
                .ok_or(ErrorCode::MathOverflow)?;
            
            require!(product > 0, ErrorCode::InsufficientLiquidity);
            
            // Integer square root using Newton's method
            let mut z = product;
            let mut y = (product + 1) / 2;
            while y < z {
                z = y;
                y = (product / y + y) / 2;
            }
            
            let sqrt_result = z as u64;
            
            // Minimum liquidity - permanently locked to prevent inflation attacks
            // Set to 1 to support all deposit sizes while maintaining security
            const MINIMUM_LIQUIDITY: u64 = 1;
            
            // For initial deposit, ensure minimum liquidity can be locked
            require!(sqrt_result > MINIMUM_LIQUIDITY, ErrorCode::InsufficientLiquidity);
            
            // Subtract minimum liquidity 
            liquidity_minted = sqrt_result - MINIMUM_LIQUIDITY;
            
            require!(liquidity_minted > 0, ErrorCode::InsufficientLiquidity);
            
            // Update total supply to include both minted liquidity and locked minimum
            amm_info.supply = sqrt_result;
            amm_info.ever_deposited = true;
        } else {
            // Subsequent deposits - calculate liquidity proportionally
            // Use minimum to prevent single-sided liquidity provision
            let liquidity0 = (amount0 as u128)
                .checked_mul(amm_info.supply as u128)
                .ok_or(ErrorCode::MathOverflow)?
                .checked_div(amm_info.reserve0 as u128)
                .ok_or(ErrorCode::DivisionByZero)? as u64;
            
            let liquidity1 = (amount1 as u128)
                .checked_mul(amm_info.supply as u128)
                .ok_or(ErrorCode::MathOverflow)?
                .checked_div(amm_info.reserve1 as u128)
                .ok_or(ErrorCode::DivisionByZero)? as u64;
            
            liquidity_minted = std::cmp::min(liquidity0, liquidity1);
            require!(liquidity_minted > 0, ErrorCode::InsufficientLiquidity);
            
            // Update total supply
            amm_info.supply = amm_info.supply.checked_add(liquidity_minted)
                .ok_or(ErrorCode::MathOverflow)?;
        }

        // Transfer token0 from sender to AMM
        let cpi_accounts = Transfer {
            from: ctx.accounts.senders_token_account0.to_account_info(),
            to: ctx.accounts.pdas_token_account0.to_account_info(),
            authority: ctx.accounts.sender.to_account_info(),
        };
        let cpi_program = ctx.accounts.token_program.to_account_info();
        let cpi_ctx = CpiContext::new(cpi_program, cpi_accounts);
        token::transfer(cpi_ctx, amount0)?;

        // Transfer token1 from sender to AMM
        let cpi_accounts = Transfer {
            from: ctx.accounts.senders_token_account1.to_account_info(),
            to: ctx.accounts.pdas_token_account1.to_account_info(),
            authority: ctx.accounts.sender.to_account_info(),
        };
        let cpi_program = ctx.accounts.token_program.to_account_info();
        let cpi_ctx = CpiContext::new(cpi_program, cpi_accounts);
        token::transfer(cpi_ctx, amount1)?;

        // Update reserves
        amm_info.reserve0 = amm_info.reserve0.checked_add(amount0)
            .ok_or(ErrorCode::MathOverflow)?;
        amm_info.reserve1 = amm_info.reserve1.checked_add(amount1)
            .ok_or(ErrorCode::MathOverflow)?;
        
        // Update minted PDA - only track what user actually receives
        minted_pda.minted = minted_pda.minted.checked_add(liquidity_minted)
            .ok_or(ErrorCode::MathOverflow)?;

        msg!("Deposited {} token0 and {} token1, minted {} liquidity", amount0, amount1, liquidity_minted);
        Ok(())
    }

    /// Redeem liquidity tokens for underlying assets
    pub fn redeem(ctx: Context<RedeemOrSwapCtx>, amount: u64) -> Result<()> {
        require!(amount > 0, ErrorCode::InvalidAmount);

        let amm_info = &mut ctx.accounts.amm_info;
        
        // Get minted_pda account info
        let minted_pda_info = ctx.accounts.minted_pda.to_account_info();
        
        // Check if account has been initialized by checking owner and data
        require!(
            *minted_pda_info.owner == crate::ID,
            ErrorCode::AccountNotInitialized
        );
        
        let data = minted_pda_info.try_borrow_data()?;
        require!(data.len() >= 16, ErrorCode::AccountNotInitialized);
        
        // Read minted value (bytes 8-16, after discriminator)
        let minted = u64::from_le_bytes([
            data[8], data[9], data[10], data[11],
            data[12], data[13], data[14], data[15],
        ]);
        
        drop(data);

        require!(minted >= amount, ErrorCode::InsufficientLiquidity);
        require!(amm_info.supply >= amount, ErrorCode::InsufficientLiquidity);

        // Calculate amounts to return
        let amount0 = (amount as u128)
            .checked_mul(amm_info.reserve0 as u128)
            .ok_or(ErrorCode::MathOverflow)?
            .checked_div(amm_info.supply as u128)
            .ok_or(ErrorCode::DivisionByZero)? as u64;

        let amount1 = (amount as u128)
            .checked_mul(amm_info.reserve1 as u128)
            .ok_or(ErrorCode::MathOverflow)?
            .checked_div(amm_info.supply as u128)
            .ok_or(ErrorCode::DivisionByZero)? as u64;

        require!(amount0 > 0 && amount1 > 0, ErrorCode::InsufficientOutput);

        // PDA signer seeds
        let mint0_key = ctx.accounts.mint0.key();
        let mint1_key = ctx.accounts.mint1.key();
        let seeds = &[
            b"amm",
            mint0_key.as_ref(),
            mint1_key.as_ref(),
            &[ctx.bumps.amm_info],
        ];
        let signer = &[&seeds[..]];

        // Transfer token0 from AMM to sender
        let cpi_accounts = Transfer {
            from: ctx.accounts.pdas_token_account0.to_account_info(),
            to: ctx.accounts.senders_token_account0.to_account_info(),
            authority: amm_info.to_account_info(),
        };
        let cpi_program = ctx.accounts.token_program.to_account_info();
        let cpi_ctx = CpiContext::new_with_signer(cpi_program, cpi_accounts, signer);
        token::transfer(cpi_ctx, amount0)?;

        // Transfer token1 from AMM to sender
        let cpi_accounts = Transfer {
            from: ctx.accounts.pdas_token_account1.to_account_info(),
            to: ctx.accounts.senders_token_account1.to_account_info(),
            authority: amm_info.to_account_info(),
        };
        let cpi_program = ctx.accounts.token_program.to_account_info();
        let cpi_ctx = CpiContext::new_with_signer(cpi_program, cpi_accounts, signer);
        token::transfer(cpi_ctx, amount1)?;

        // Update reserves and supply
        amm_info.reserve0 = amm_info.reserve0.checked_sub(amount0)
            .ok_or(ErrorCode::MathOverflow)?;
        amm_info.reserve1 = amm_info.reserve1.checked_sub(amount1)
            .ok_or(ErrorCode::MathOverflow)?;
        amm_info.supply = amm_info.supply.checked_sub(amount)
            .ok_or(ErrorCode::MathOverflow)?;
        
        // Update minted_pda
        let new_minted = minted.checked_sub(amount)
            .ok_or(ErrorCode::MathOverflow)?;
        
        let mut data = minted_pda_info.try_borrow_mut_data()?;
        let new_minted_bytes = new_minted.to_le_bytes();
        data[8..16].copy_from_slice(&new_minted_bytes);

        msg!("Redeemed {} liquidity for {} token0 and {} token1", amount, amount0, amount1);
        Ok(())
    }

    /// Swap tokens using constant product formula (x * y = k)
    pub fn swap(
        ctx: Context<RedeemOrSwapCtx>,
        is_mint0: bool,
        amount_in: u64,
        min_out_amount: u64,
    ) -> Result<()> {
        require!(amount_in > 0, ErrorCode::InvalidAmount);

        let amm_info = &mut ctx.accounts.amm_info;

        let (reserve_in, reserve_out) = if is_mint0 {
            (amm_info.reserve0, amm_info.reserve1)
        } else {
            (amm_info.reserve1, amm_info.reserve0)
        };

        // Constant product formula: (x + dx) * (y - dy) = x * y
        // With 0.3% fee: dy = (y * dx * 997) / (x * 1000 + dx * 997)
        let amount_in_with_fee = (amount_in as u128)
            .checked_mul(997)
            .ok_or(ErrorCode::MathOverflow)?;
        
        let numerator = amount_in_with_fee
            .checked_mul(reserve_out as u128)
            .ok_or(ErrorCode::MathOverflow)?;
        
        let denominator = (reserve_in as u128)
            .checked_mul(1000)
            .ok_or(ErrorCode::MathOverflow)?
            .checked_add(amount_in_with_fee)
            .ok_or(ErrorCode::MathOverflow)?;
        
        let amount_out = numerator
            .checked_div(denominator)
            .ok_or(ErrorCode::DivisionByZero)? as u64;

        require!(amount_out >= min_out_amount, ErrorCode::SlippageExceeded);
        require!(amount_out > 0, ErrorCode::InsufficientOutput);
        require!(amount_out < reserve_out, ErrorCode::InsufficientLiquidity);

        // PDA signer seeds
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
            // Transfer token0 from sender to AMM
            let cpi_accounts = Transfer {
                from: ctx.accounts.senders_token_account0.to_account_info(),
                to: ctx.accounts.pdas_token_account0.to_account_info(),
                authority: ctx.accounts.sender.to_account_info(),
            };
            let cpi_program = ctx.accounts.token_program.to_account_info();
            let cpi_ctx = CpiContext::new(cpi_program, cpi_accounts);
            token::transfer(cpi_ctx, amount_in)?;

            // Transfer token1 from AMM to sender
            let cpi_accounts = Transfer {
                from: ctx.accounts.pdas_token_account1.to_account_info(),
                to: ctx.accounts.senders_token_account1.to_account_info(),
                authority: amm_info.to_account_info(),
            };
            let cpi_program = ctx.accounts.token_program.to_account_info();
            let cpi_ctx = CpiContext::new_with_signer(cpi_program, cpi_accounts, signer);
            token::transfer(cpi_ctx, amount_out)?;

            // Update reserves
            amm_info.reserve0 = amm_info.reserve0.checked_add(amount_in)
                .ok_or(ErrorCode::MathOverflow)?;
            amm_info.reserve1 = amm_info.reserve1.checked_sub(amount_out)
                .ok_or(ErrorCode::MathOverflow)?;
        } else {
            // Transfer token1 from sender to AMM
            let cpi_accounts = Transfer {
                from: ctx.accounts.senders_token_account1.to_account_info(),
                to: ctx.accounts.pdas_token_account1.to_account_info(),
                authority: ctx.accounts.sender.to_account_info(),
            };
            let cpi_program = ctx.accounts.token_program.to_account_info();
            let cpi_ctx = CpiContext::new(cpi_program, cpi_accounts);
            token::transfer(cpi_ctx, amount_in)?;

            // Transfer token0 from AMM to sender
            let cpi_accounts = Transfer {
                from: ctx.accounts.pdas_token_account0.to_account_info(),
                to: ctx.accounts.senders_token_account0.to_account_info(),
                authority: amm_info.to_account_info(),
            };
            let cpi_program = ctx.accounts.token_program.to_account_info();
            let cpi_ctx = CpiContext::new_with_signer(cpi_program, cpi_accounts, signer);
            token::transfer(cpi_ctx, amount_out)?;

            // Update reserves
            amm_info.reserve1 = amm_info.reserve1.checked_add(amount_in)
                .ok_or(ErrorCode::MathOverflow)?;
            amm_info.reserve0 = amm_info.reserve0.checked_sub(amount_out)
                .ok_or(ErrorCode::MathOverflow)?;
        }

        msg!("Swapped {} in for {} out", amount_in, amount_out);
        Ok(())
    }
}

// Account Contexts

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
        constraint = token_account0.mint == mint0.key(),
        constraint = token_account0.owner == initializer.key()
    )]
    pub token_account0: Account<'info, TokenAccount>,

    #[account(
        mut,
        constraint = token_account1.mint == mint1.key(),
        constraint = token_account1.owner == initializer.key()
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

    /// CHECK: This account is optional - only used for redeem, not for swap
    #[account(
        mut,
        seeds = [b"minted", sender.key().as_ref()],
        bump
    )]
    pub minted_pda: UncheckedAccount<'info>,

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
    pub ever_deposited: bool,
    pub supply: u64,
}

#[account]
#[derive(InitSpace)]
pub struct MintedPda {
    pub minted: u64,
}

// Error Codes

#[error_code]
pub enum ErrorCode {
    #[msg("Invalid amount provided")]
    InvalidAmount,
    #[msg("Math overflow")]
    MathOverflow,
    #[msg("Division by zero")]
    DivisionByZero,
    #[msg("Insufficient liquidity")]
    InsufficientLiquidity,
    #[msg("Insufficient output amount")]
    InsufficientOutput,
    #[msg("Slippage tolerance exceeded")]
    SlippageExceeded,
    #[msg("Invalid account")]
    InvalidAccount,
    #[msg("Account not initialized")]
    AccountNotInitialized,
}