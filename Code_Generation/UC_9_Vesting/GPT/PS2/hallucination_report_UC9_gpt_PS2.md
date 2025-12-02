## Identified Hallucinations

### Knowledge Conflicting: API Knowledge
**Description:** 
use of undeclared module

**Code Example:**
```rust
let cpi_ctx = CpiContext::new(
    ctx.accounts.system_program.to_account_info(),
    anchor_lang::system_program::Transfer {
        from: ctx.accounts.funder.to_account_info(),
        to: ctx.accounts.vesting_info.to_account_info(),
    },
);
anchor_lang::system_program::transfer(cpi_ctx, lamports_amount)?;

```

### Intent Conflicting
**Description:** 
The contract does not follow the prompt's specific directives regarding the data structure.

**Code Example:**
```rust
pub struct VestingInfo {
    pub beneficiary: Pubkey,
    pub start_slot: u64,
    pub duration: u64,
    pub released: u64,
}
```

**CrystalBLEU similarity: 0.205** 
