## Identified Hallucinations

### Context Devition: Dead Code
**Description:** 

The code declares multiple variables that are never used, resulting in dead code.

**Code Example:**
```rust
let owner = &ctx.accounts.owner;

let system_program = &ctx.accounts.system_program;

let system_program = &ctx.accounts.system_program;
```

**CrystalBLEU similarity: 0.344** 

