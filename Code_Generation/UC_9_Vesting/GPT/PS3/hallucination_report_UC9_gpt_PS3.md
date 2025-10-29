## Identified Hallucinations

### Context Deviation: Inconsistency
**Description:** 
Borrowing rules violation in account access

**Code Example:**
```rust
let vesting_ai = ctx.accounts.vesting_info.to_account_info();
```

### Context Deviation: Dead Code
**Description:** 
unused signer_seeds

**Code Example:**
```rust
let signer_seeds = &[&seeds[..]];
```


**CrystalBLEU similarity: 0.236** 
