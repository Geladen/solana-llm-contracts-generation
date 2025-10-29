## Identified Hallucinations

### Context Deviation: Dead Code
**Description:** 

The escrow_name parameter is declared in multiple function signatures but never utilized within their respective logic.

**Code Example:**
```rust
pub fn deposit(ctx: Context<DepositCtx>, escrow_name: String) -> Result<()> {
```

**CrystalBLEU similarity: 0.365** 
