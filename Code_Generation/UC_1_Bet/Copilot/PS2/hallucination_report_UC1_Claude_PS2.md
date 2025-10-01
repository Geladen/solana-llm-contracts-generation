## Identified Hallucinations

### [Knowledge Conflicting]
**Description:** 
The anchor program uses the obsolete `#[account(signer)]` attribute instead of `Signer<'info>` type for account validation.

**Code Example:**
```rust
/// CHECK: funds their wager
#[account(mut, signer)]
participant1: AccountInfo<'info>,

/// CHECK: funds their wager  
#[account(mut, signer)] 
participant2: AccountInfo<'info>,

/// CHECK: must sign; key is verified in handler
#[account(signer)]
oracle: AccountInfo<'info>,
