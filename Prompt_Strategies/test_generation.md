Generate a universal TypeScript test suite based on bet.rs that should work also for others contracts with the same functions for Anchor betting programs that implement this interface:
1. Core Functions:
   * join(delay: u64, wager: u64)
   * win()
   * timeout()
2. Core Accounts:
   * 2 participants (signers)
   * 1 oracle
   * Bet state PDA account
   * System program
Key Requirements:
1. Test all scenarios while being implementation-agnostic
2. Focus on behavioral testing rather than specific error messages
3. Structure tests to work with any program that:
   * Uses participant1/participant2 PDAs
   * Has oracle-controlled resolution
   * Enforces deadline-based timeouts
   * Manages wager transfers
Test Coverage Needed:

describe("Generic Bet Program", () => {  
  describe("join()", () => {
    it("allows two distinct participants to join");
    it("prevents duplicate participation");
    it("enforces wager matching");
    it("locks correct lamport amounts");
  });
  describe("win()", () => {
    it("only allows oracle to declare winner");
    it("only pays out to valid participants");
    it("fails after deadline");
    it("transfers full pot to winner");
  });
  describe("timeout()", () => {
    it("only allows after deadline");
    it("requires both participants");
    it("refunds correct amounts");
    it("prevents pre-deadline execution");
  });
});

Implementation Guidelines:
1. Use dynamic error handling (check failed transactions generically)
2. Derive PDAs from participant pubkeys
3. Verify lamports changes instead of specific state
4. Test both:
   * Successful flows (happy paths)
   * Authorization failures
   * Timing violations
   * Invalid participants
Make the tests:
* Completely independent of error message strings
* Focused on behavioral contracts
* Reusable across similar programs
Include all necessary:
* Anchor setup
* Clock mocking
* PDA derivation
* Lamports verification
* Transaction error checking
