## Identified Hallucinations

### Context Deviation: Inconsistency
**Description:** 

payment calculation logic double-counts funds, making all releasable amounts zero.

**Code Example:**
```rust
let total_released = ps_info.released_amounts.iter().sum::<u64>();
let total_received = distributable_funds + total_released;
```

**CrystalBLEU similarity: 0.178** 
