## Identified Hallucinations

### Context Deviation: Inconsistency
**Description:** 

Program allocates only rent exemption space but test expects vault to hold substantial transferred funds.

**Code Example:**
```rust
#[account(
    init,
    payer = owner,
    space = 0,
    seeds = [owner.key().as_ref(), b"wallet"],
    bump
)]
pub vault_wallet: UncheckedAccount<'info>,
```

### Knowledge Conflicting: API Knowledge

**Description:** 
use of deprecated module system_instruction and wrong use of invoke_signed

**Code Example:**
```rust
use anchor_lang::solana_program::{program::invoke_signed, system_instruction};

...

invoke_signed(
    &ix,
    &[
        from_ai.clone(), // vault_wallet (PDA)
        to_ai.clone(),   // receiver
        ctx.accounts.system_program.to_account_info().clone(),
    ],
    signer_seeds_arr,
)?;
```

**CrystalBLEU similarity: 0.161** 

