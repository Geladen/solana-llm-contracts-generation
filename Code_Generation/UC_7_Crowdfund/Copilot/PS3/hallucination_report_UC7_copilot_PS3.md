## Identified Hallucinations

### Context Deviation: Dead Code
**Description:** 

The campaign_name parameter is declared in the function signature but never utilized within the function's logic.

**Code Example:**
```rust
pub fn donate(
        ctx: Context<DonateCtx>,
        campaign_name: String,
        donated_lamports: u64,
    ) -> Result<()> {
```

**CrystalBLEU similarity: 0.341** 

