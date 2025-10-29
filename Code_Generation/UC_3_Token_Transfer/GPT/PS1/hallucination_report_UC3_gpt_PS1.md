## Identified Hallucinations

### Context Deviation: Inconsistency
**Description:** 

Mismatch between amount interpretation in the program versus tests.
Wrong account closure logic.

**Code Example:**
```rust
    token::transfer(cpi_ctx, amount_to_withdraw)?;

    // Close temp_ata if full balance withdrawn
    if amount_to_withdraw == temp_ata_balance {
        let cpi_accounts = CloseAccount {
            account: ctx.accounts.temp_ata.to_account_info(),
            destination: ctx.accounts.sender.to_account_info(),
            authority: ctx.accounts.atas_holder_pda.to_account_info(),
        };
        let cpi_ctx = CpiContext::new_with_signer(cpi_program.clone(), cpi_accounts, signer_seeds);
        token::close_account(cpi_ctx)?;

        // Close deposit_info and return lamports
        **ctx.accounts.sender.lamports.borrow_mut() += ctx.accounts.deposit_info.to_account_info().lamports();
        **ctx.accounts.deposit_info.to_account_info().lamports.borrow_mut() = 0;
    }
```

**CrystalBLEU similarity: 0.295** 
