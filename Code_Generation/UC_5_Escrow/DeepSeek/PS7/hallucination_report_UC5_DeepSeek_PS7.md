## Identified Hallucinations

### Context Deviation: Dead Code
**Description:** 

The escrow_name parameter is declared in multiple function signatures but never utilized within their respective logic.

**Code Example:**
```rust
    pub fn initialize(
        ctx: Context<InitializeCtx>,
        amount_in_lamports: u64,
        escrow_name: String,
    ) -> Result<()> {
```

**CrystalBLEU similarity: 0.342** 
