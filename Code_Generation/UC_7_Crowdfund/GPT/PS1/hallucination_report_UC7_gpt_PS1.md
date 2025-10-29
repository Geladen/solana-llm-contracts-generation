## Identified Hallucinations


### Context Deviation: Inconsistency
**Description:** 

Rent exemption handling mismatch in donated funds calculation

**Code Example:**
```rust
let donated = campaign_total.saturating_sub(rent_min);
```

### Knowledge Conflicting: API Knowledge
**Description:** 
unused import sysvar

**Code Example:**
```
use anchor_lang::solana_program::{clock::Clock, sysvar};
```

**CrystalBLEU similarity: 0.219** 
