1. Core Requirements + Signature
Purpose: To unambiguously define the security model by specifying which actors must authorize each action. This is critical for preventing unauthorized access and to ensure compatibility with tests.

```
[Instruction Name]: Requires signatures from [list of signers]
[Instruction Name]: Requires signature from [signer]
[Instruction Name]: Requires signature from [either signer A or signer B]
```

2. Account + Seed Specification 
Purpose: To ensure compatibility with tests and client-side code. By explicitly declaring all accounts, and exact PDA seeds, you guarantee the generated code will derive the same addresses as your tests expect.

```
Account Restrictions:

For [Struct Name] structure:
-account_name (signer)
-another_account (PDA)
-system_program (program, reference)

For [Another Structure] structure:
-account_name (signer)
-pda_account (PDA)
-reference_account (reference)

All PDAs must use seeds structured exactly as:
seeds = [first_account.key().as_ref(), second_account.key().as_ref()]
```

3. Function Signature Technique
Purpose: To ensure compatibility with tests and force the general logic of the smart contract, its instructions, its parameters.
How to do it: For each function, specify its Context, parameters, and the key actions it must perform.

```

function_name(ctx: Context<ContextName>, param: u64) -> Result<()>
-Must [key requirement]
-Transfers [amount] from [account] to [account]

```

4. Additional Constraints 
Purpose: Additional constraints can be added if necessary.

```
Reject transactions where:
-A required state condition is not met (e.g., deadline not reached)
-The state is already finalized (e.g., already resolved)
```

5. Adapted Few-shot Technique
Purpose: To provide behavioral examples that test both the "happy path" (successful operations) and edge cases (error conditions). This helps the AI understand the desired input/output behavior concretely.
How to do it: Provide 4-5 short scenarios. Typically, use 3 positive examples and 2 negative examples.

```
Example Scenarios:
1: Successful [Action] scenario
Input: [Accounts and parameters used]
Output: [State change and transfers that occur]

...

5: Error scenario - [Error Type]
Input: [Invalid accounts or parameters]
Output: Transaction is reverted with an "[ErrorName]" error.
```

6. List of Packages Technique
Purpose: To limit the scope of the AI's knowledge to specific crates and versions, ensuring the code is generated with the correct syntax and available methods.
How to do it: Explicitly state which crates and modules the contract can use.

```
The smart contract has access to the following packages:
anchor_lang::prelude::*
anchor_spl::token::*
```
