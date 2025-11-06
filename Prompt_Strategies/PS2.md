1. Persona Tecnique + Overview
Purpose: To set the context and expertise level, ensuring the AI adopts the correct role and understands the project's scope.
How to do it: Start by defining the AI's role and the primary objective.

```
You are an expert Solana smart contract developer specializing in the Anchor framework. Your task is to create a complete, secure, and efficient Anchor smart contract for a [describe system purpose] with the following strict specifications:
```

2. Core Requirements + Signature
Purpose: To unambiguously define the security model by specifying which actors must authorize each action. This is critical for preventing unauthorized access and to ensure compatibility with tests.

```
[Instruction Name]: Requires signatures from [list of signers]
[Instruction Name]: Requires signature from [signer]
[Instruction Name]: Requires signature from [either signer A or signer B]
```

3. Account + Seed Specification 
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

4. Function Signature Technique
Purpose: To ensure compatibility with tests and force the general logic of the smart contract, its instructions, its parameters.
How to do it: For each function, specify its Context, parameters, and the key actions it must perform.

```

function_name(ctx: Context<ContextName>, param: u64) -> Result<()>
-Must [key requirement]
-Transfers [amount] from [account] to [account]

```

5. Additional Constraints 
Purpose: Additional constraints can be added if necessary.

```
Reject transactions where:
-A required state condition is not met (e.g., deadline not reached)
-The state is already finalized (e.g., already resolved)
```

