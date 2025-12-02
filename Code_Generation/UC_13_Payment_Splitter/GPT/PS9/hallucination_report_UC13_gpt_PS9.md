## Identified Hallucinations

### Context Deviation: Inconsistency
**Description:** 

the code attempts to use both mutable and immutable references to the same account simultaneously

**Code Example:**
```rust
ps_info.current_lamports = **ctx.accounts.ps_info.to_account_info().lamports.borrow();
```

**CrystalBLEU similarity: 0.113**  
