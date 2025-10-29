## Identified Hallucinations

### Context Deviation: Inconsistency
**Description:** 

cannot borrow ctx.accounts.lottery_info as immutable because it is also borrowed as mutable

**Code Example:**
```rust
let lottery = &mut ctx.accounts.lottery_info;
let lottery_ai = ctx.accounts.lottery_info.to_account_info();

```


### Knowledge Conflicting: API Knowledge
**Description:** 
use of undeclared crate or module sha2

**Code Example:**
```rust
use sha2::{Digest, Sha256};
```

**CrystalBLEU similarity: 0.137** 
