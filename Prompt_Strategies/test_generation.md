1.  Overview
Purpose: To generate reusable tests based on the benchmark that can be easily adapted for similar Anchor programs.

```
Generate a universal TypeScript test suite based on name.rs that should work also for other contracts with the same functions for 'name' programs that implement this interface:

```

2. Function Signatures
Purpose: To emphasize the signatures of the functions to be tested.

```
function_name(ctx: Context<ContextName>, param: u64) -> Result<()>
function_name1(ctx: Context<ContextName>, param: u64) -> Result<()>
function_nam2(ctx: Context<ContextName>, param: u64) -> Result<()>
```

3. Additional Constraints 
Purpose: To generate tests that are as generic and independent from specific error messages as possible.

```
Implementation Guidelines:
  1. Use dynamic error handling (check failed transactions generically)
  2. Verify lamports changes instead of specific state
Make the tests:
  * Completely independent of error message strings
  * Focused on behavioral contracts
  * Reusable across similar programs
Include all necessary:
  * Anchor setup
  * PDA derivation
  * Lamports verification
  * Transaction error checking
  * Account state validation

```

