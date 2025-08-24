You are an expert Solana smart contract developer specializing in Anchor framework. Your task is to create a complete, secure, and efficient Anchor smart contract implementing a two-party betting system with the following strict specifications:
Core Requirements:
1. Simultaneous Participation:
   * Both players must join the bet in the same transaction
   * The join function must require both participants' signatures
2. Signature Requirements:
   * join: Requires signatures from both participant1 and participant2
   * win: Requires signature from the designated oracle
   * timeout: Requires signature from either participant
3. Account Restrictions:
   * Only explicitly defined accounts in the contexts may be used
   * Any usage of undeclared accounts should be rejected
   * Strictly use these accounts:
	For join instruction:
		-participant1 (signer, mutable)
		-participant2 (signer, mutable)
		-oracle (reference, for storage)
		-bet_info (PDA, to be initialized)
		-system_program (program)
	For win instruction:	
		-oracle (signer)
		-winner (winner account, mutable)
		-bet_info (PDA, mutable)
		-participant1 (reference, for PDA derivation)
		-participant2 (reference, for PDA derivation)
		-system_program (program)
	For timeout instruction:
		-participant1 (mutable)
		-participant2 (mutable)
		-bet_info (PDA, mutable)
		-system_program (program)
      All PDAs must use seeds structured exactly as:
      seeds = [participant1.key().as_ref(), participant2.key().as_ref()]
4. State Management:
   * Enforce that both participants have deposited before any resolution
5. Security Constraints:
   * Prevent front-running with proper state checks
   * Ensure all transfers are atomic within the same transaction
   * Include comprehensive error checks for all operations
Function Specifications:
1. join(ctx: Context<JoinCtx>, delay: u64, wager: u64) -> Result<()>
   * Must be called by both participants in same transaction
   * Transfers equal wagers from both participants to PDA
   * Sets deadline as current slot + delay
2. win(ctx: Context<WinCtx>) -> Result<()>
   * Only callable by pre-designated oracle
   * Transfers entire pot to winner
   * Marks bet as resolved
3. timeout(ctx: Context<TimeoutCtx>) -> Result<()>
   * Only callable after deadline
   * Returns original wagers to participants
   * Marks bet as resolved
Validation Requirements:
* Reject transactions where:
   * Not all required parties have signed
   * The oracle is invalid
   * The deadline hasn't been reached (for timeout)
   * The deadline has passed (for win)
   * The bet is already resolved
Example Flow:
1. Both participants sign join transaction with equal wagers
2. Oracle signs win before deadline to award pot
3. If no oracle action, either participant can call timeout after deadline
The contract must use only the explicitly declared accounts and follow Anchor best practices for security and efficiency. Include all necessary account validations and state checks to prevent invalid operations.
