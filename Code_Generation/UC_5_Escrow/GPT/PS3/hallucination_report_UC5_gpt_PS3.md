## Identified Hallucinations

### Intent Conflicting
**Description:** 

Model reordered function parameters despite explicit signature provided in prompt, causing serialization errors.

**Code Example:**
```rust
pub fn initialize(
    ctx: Context<Initialize>,
    escrow_name: String,
    amount_in_lamports: u64,
) -> Result<()> {
```

**CrystalBLEU similarity: 0.280** 
