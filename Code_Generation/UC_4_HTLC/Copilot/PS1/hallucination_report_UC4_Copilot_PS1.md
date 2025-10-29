## Identified Hallucinations

### Context Deviation: Inconsistency
**Description:** 

The timeout function incorrectly requires strict inequality while reveal uses inclusive comparison, creating a dead slot where neither function succeeds.

**Code Example:**
```rust
// allow reveal up to and including the stored deadline
require!(
    clock.slot <= htlc.reveal_timeout,
    HtlcError::RevealAfterTimeout
);
```

**CrystalBLEU similarity: 0.229** 
