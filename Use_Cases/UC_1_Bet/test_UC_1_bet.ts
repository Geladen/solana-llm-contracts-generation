import * as anchor from "@coral-xyz/anchor";
import { Program } from "@coral-xyz/anchor";
import { PublicKey, Keypair, LAMPORTS_PER_SOL, SystemProgram } from "@solana/web3.js";
import { BN } from "bn.js";
import { expect } from "chai";

// Generic interface for any betting program
interface BettingProgram extends Program {
  methods: {
    join(delay: BN, wager: BN): any;
    win(): any;
    timeout(): any;
  };
}

/**
 * Universal Test Suite for Anchor Betting Programs
 * 
 * This test suite is designed to work with any Anchor program that implements:
 * - join(delay: u64, wager: u64) function
 * - win() function with oracle-controlled resolution
 * - timeout() function with deadline enforcement
 * - Standard participant1/participant2 PDA structure
 */
describe("Generic Betting Program Test Suite", () => {
  // Test configuration - adjust these for your specific program
  const PROGRAM_ID_STRING = "7mMf8y3WnKREkqkUG96viUvsMfpwfaPHqxBSxbMUMJQN"; // Update this
  const PROGRAM_ID = new PublicKey(PROGRAM_ID_STRING);
  
  // Test setup
  const provider = anchor.AnchorProvider.env();
  anchor.setProvider(provider);
  
  // Program will be loaded in before() hook
  let program: BettingProgram;
  
  // Test accounts
  let participant1: Keypair;
  let participant2: Keypair;
  let oracle: Keypair;
  let betInfoPDA: PublicKey;
  let betInfoBump: number;
  
  // Test constants
  const WAGER_AMOUNT = new BN(LAMPORTS_PER_SOL * 0.1); // 0.1 SOL
  const DELAY_SLOTS = new BN(1000);
  const INITIAL_BALANCE = LAMPORTS_PER_SOL * 10; // 10 SOL

  // Setup program loading
  before(async () => {
    try {
      // Try to load from workspace first
      program = anchor.workspace.bet as BettingProgram;
      console.log("✓ Program loaded from workspace");
    } catch (workspaceError) {
      try {
        // Fallback: load program directly using IDL
        console.log("Workspace loading failed, trying direct IDL loading...");
        const fs = await import('fs');
        const idl = JSON.parse(fs.readFileSync('./target/idl/bet.json', 'utf8'));
        program = new anchor.Program(idl, PROGRAM_ID, provider) as BettingProgram;
        console.log("✓ Program loaded from IDL file");
      } catch (idlError) {
        console.error("Failed to load program from workspace:", workspaceError.message);
        console.error("Failed to load program from IDL:", idlError.message);
        console.error("\n🔧 Troubleshooting steps:");
        console.error("1. Run: anchor build");
        console.error("2. Check that target/idl/bet.json exists");
        console.error("3. Verify program name in Anchor.toml matches 'bet'");
        console.error("4. Make sure you're running tests from project root");
        throw new Error("Could not load program. Please run 'anchor build' first.");
      }
    }
  });

  beforeEach(async () => {
    // Generate fresh keypairs for each test
    participant1 = Keypair.generate();
    participant2 = Keypair.generate();
    oracle = Keypair.generate();

    // Fund test accounts
    await Promise.all([
      provider.connection.requestAirdrop(participant1.publicKey, INITIAL_BALANCE),
      provider.connection.requestAirdrop(participant2.publicKey, INITIAL_BALANCE),
      provider.connection.requestAirdrop(oracle.publicKey, INITIAL_BALANCE),
    ]);

    // Wait for airdrops to confirm
    await new Promise(resolve => setTimeout(resolve, 1000));

    // Derive PDA for bet info
    [betInfoPDA, betInfoBump] = PublicKey.findProgramAddressSync(
      [participant1.publicKey.toBuffer(), participant2.publicKey.toBuffer()],
      program.programId
    );
  });

  /**
   * Utility Functions
   */
  
  async function getAccountBalance(pubkey: PublicKey): Promise<number> {
    return await provider.connection.getBalance(pubkey);
  }

  async function expectTransactionToFail(transactionPromise: Promise<any>): Promise<boolean> {
    try {
      await transactionPromise;
      return false; // Transaction succeeded when it should have failed
    } catch (error) {
      return true; // Transaction failed as expected
    }
  }

  async function advanceSlots(slots: number): Promise<void> {
    // For local test validator, we can use warp_to_slot
    // This only works with solana-test-validator
    try {
      const currentSlot = await provider.connection.getSlot();
      const targetSlot = currentSlot + slots;
      
      // This is a workaround - in real testing you'd use test validator's warp capabilities
      // For now, we'll create a bet with 0 delay to simulate expired deadline
      console.log(`Current slot: ${currentSlot}, attempting to advance ${slots} slots`);
    } catch (error) {
      console.log("Slot advancement not available in this test environment");
    }
  }

  /**
   * JOIN FUNCTION TESTS
   */
  describe("join() - Bet Creation and Participation", () => {
    it("should allow two distinct participants to join with valid parameters", async () => {
      const p1BalanceBefore = await getAccountBalance(participant1.publicKey);
      const p2BalanceBefore = await getAccountBalance(participant2.publicKey);

      await program.methods
        .join(DELAY_SLOTS, WAGER_AMOUNT)
        .accounts({
          participant1: participant1.publicKey,
          participant2: participant2.publicKey,
          oracle: oracle.publicKey,
          betInfo: betInfoPDA,
          systemProgram: SystemProgram.programId,
        })
        .signers([participant1, participant2])
        .rpc();

      // Verify balances decreased by wager amount (plus rent and transaction fees)
      const p1BalanceAfter = await getAccountBalance(participant1.publicKey);
      const p2BalanceAfter = await getAccountBalance(participant2.publicKey);
      
      const p1BalanceChange = p1BalanceBefore - p1BalanceAfter;
      const p2BalanceChange = p2BalanceBefore - p2BalanceAfter;
      
      // Both participants should pay at least the wager amount
      expect(p1BalanceChange).to.be.at.least(
        WAGER_AMOUNT.toNumber(),
        `Participant1 should pay at least wager amount. Paid: ${p1BalanceChange}, Expected: ${WAGER_AMOUNT.toNumber()}`
      );
      expect(p2BalanceChange).to.be.at.least(
        WAGER_AMOUNT.toNumber(),
        `Participant2 should pay at least wager amount. Paid: ${p2BalanceChange}, Expected: ${WAGER_AMOUNT.toNumber()}`
      );
      
      // Neither should pay more than wager + reasonable overhead (0.1 SOL buffer)
      const maxExpected = WAGER_AMOUNT.toNumber() + LAMPORTS_PER_SOL * 0.1;
      expect(p1BalanceChange).to.be.at.most(maxExpected);
      expect(p2BalanceChange).to.be.at.most(maxExpected);

      // Verify bet info account was created and funded
      const betBalance = await getAccountBalance(betInfoPDA);
      expect(betBalance).to.be.at.least(
        WAGER_AMOUNT.toNumber() * 2 - LAMPORTS_PER_SOL * 0.01,
        `Bet account should contain approximately 2x wager amount. Balance: ${betBalance}, Expected: ${WAGER_AMOUNT.toNumber() * 2}`
      );
    });

    it("should prevent duplicate participation (same participant twice)", async () => {
      const failed = await expectTransactionToFail(
        program.methods
          .join(DELAY_SLOTS, WAGER_AMOUNT)
          .accounts({
            participant1: participant1.publicKey,
            participant2: participant1.publicKey, // Same participant
            oracle: oracle.publicKey,
            betInfo: betInfoPDA,
            systemProgram: SystemProgram.programId,
          })
          .signers([participant1])
          .rpc()
      );

      expect(failed).to.be.true;
    });

    it("should handle zero wager (implementation dependent)", async () => {
      // Some implementations may allow zero wagers, others may reject them
      // This test documents the behavior without enforcing a specific expectation
      try {
        await program.methods
          .join(DELAY_SLOTS, new BN(0))
          .accounts({
            participant1: participant1.publicKey,
            participant2: participant2.publicKey,
            oracle: oracle.publicKey,
            betInfo: betInfoPDA,
            systemProgram: SystemProgram.programId,
          })
          .signers([participant1, participant2])
          .rpc();
        
        console.log("✓ Program allows zero wager bets");
        
        // If zero wager is allowed, verify no funds were transferred
        const betBalance = await getAccountBalance(betInfoPDA);
        expect(betBalance).to.be.at.most(LAMPORTS_PER_SOL * 0.01); // Only rent
        
      } catch (error) {
        console.log("✓ Program rejects zero wager bets");
        // This is also valid behavior
      }
    });

    it("should prevent joining with insufficient funds", async () => {
      const poorParticipant = Keypair.generate();
      // Don't fund this participant
      
      const failed = await expectTransactionToFail(
        program.methods
          .join(DELAY_SLOTS, WAGER_AMOUNT)
          .accounts({
            participant1: poorParticipant.publicKey,
            participant2: participant2.publicKey,
            oracle: oracle.publicKey,
            betInfo: betInfoPDA,
            systemProgram: SystemProgram.programId,
          })
          .signers([poorParticipant, participant2])
          .rpc()
      );

      expect(failed).to.be.true;
    });

    it("should prevent creating duplicate bets with same participants", async () => {
      // Create first bet
      await program.methods
        .join(DELAY_SLOTS, WAGER_AMOUNT)
        .accounts({
          participant1: participant1.publicKey,
          participant2: participant2.publicKey,
          oracle: oracle.publicKey,
          betInfo: betInfoPDA,
          systemProgram: SystemProgram.programId,
        })
        .signers([participant1, participant2])
        .rpc();

      // Attempt to create second bet with same participants
      const failed = await expectTransactionToFail(
        program.methods
          .join(DELAY_SLOTS, WAGER_AMOUNT)
          .accounts({
            participant1: participant1.publicKey,
            participant2: participant2.publicKey,
            oracle: oracle.publicKey,
            betInfo: betInfoPDA,
            systemProgram: SystemProgram.programId,
          })
          .signers([participant1, participant2])
          .rpc()
      );

      expect(failed).to.be.true;
    });
  });

  /**
   * WIN FUNCTION TESTS
   */
  describe("win() - Oracle-Controlled Resolution", () => {
    beforeEach(async () => {
      // Create a bet for each test
      await program.methods
        .join(DELAY_SLOTS, WAGER_AMOUNT)
        .accounts({
          participant1: participant1.publicKey,
          participant2: participant2.publicKey,
          oracle: oracle.publicKey,
          betInfo: betInfoPDA,
          systemProgram: SystemProgram.programId,
        })
        .signers([participant1, participant2])
        .rpc();
    });

    it("should allow oracle to declare participant1 as winner", async () => {
      const winnerBalanceBefore = await getAccountBalance(participant1.publicKey);
      const expectedPayout = WAGER_AMOUNT.toNumber() * 2;

      await program.methods
        .win()
        .accounts({
          oracle: oracle.publicKey,
          winner: participant1.publicKey,
          betInfo: betInfoPDA,
          participant1: participant1.publicKey,
          participant2: participant2.publicKey,
          systemProgram: SystemProgram.programId,
        })
        .signers([oracle])
        .rpc();

      const winnerBalanceAfter = await getAccountBalance(participant1.publicKey);
      const balanceIncrease = winnerBalanceAfter - winnerBalanceBefore;
      
      expect(balanceIncrease).to.be.at.least(
        expectedPayout - LAMPORTS_PER_SOL * 0.01,
        `Winner should receive approximately the full pot. Received: ${balanceIncrease}, Expected: ${expectedPayout}`
      );
    });

    it("should allow oracle to declare participant2 as winner", async () => {
      const winnerBalanceBefore = await getAccountBalance(participant2.publicKey);
      const expectedPayout = WAGER_AMOUNT.toNumber() * 2;

      await program.methods
        .win()
        .accounts({
          oracle: oracle.publicKey,
          winner: participant2.publicKey,
          betInfo: betInfoPDA,
          participant1: participant1.publicKey,
          participant2: participant2.publicKey,
          systemProgram: SystemProgram.programId,
        })
        .signers([oracle])
        .rpc();

      const winnerBalanceAfter = await getAccountBalance(participant2.publicKey);
      const balanceIncrease = winnerBalanceAfter - winnerBalanceBefore;
      
      expect(balanceIncrease).to.be.at.least(
        expectedPayout - LAMPORTS_PER_SOL * 0.01,
        `Winner should receive approximately the full pot. Received: ${balanceIncrease}, Expected: ${expectedPayout}`
      );
    });

    it("should prevent non-oracle from declaring winner", async () => {
      const failed = await expectTransactionToFail(
        program.methods
          .win()
          .accounts({
            oracle: participant1.publicKey, // Wrong oracle
            winner: participant1.publicKey,
            betInfo: betInfoPDA,
            participant1: participant1.publicKey,
            participant2: participant2.publicKey,
            systemProgram: SystemProgram.programId,
          })
          .signers([participant1])
          .rpc()
      );

      expect(failed).to.be.true;
    });

    it("should prevent declaring non-participant as winner", async () => {
      const nonParticipant = Keypair.generate();
      await provider.connection.requestAirdrop(nonParticipant.publicKey, INITIAL_BALANCE);
      
      const failed = await expectTransactionToFail(
        program.methods
          .win()
          .accounts({
            oracle: oracle.publicKey,
            winner: nonParticipant.publicKey, // Not a participant
            betInfo: betInfoPDA,
            participant1: participant1.publicKey,
            participant2: participant2.publicKey,
            systemProgram: SystemProgram.programId,
          })
          .signers([oracle])
          .rpc()
      );

      expect(failed).to.be.true;
    });

    it("should prevent double resolution", async () => {
      // First resolution
      await program.methods
        .win()
        .accounts({
          oracle: oracle.publicKey,
          winner: participant1.publicKey,
          betInfo: betInfoPDA,
          participant1: participant1.publicKey,
          participant2: participant2.publicKey,
          systemProgram: SystemProgram.programId,
        })
        .signers([oracle])
        .rpc();

      // Attempt second resolution
      const failed = await expectTransactionToFail(
        program.methods
          .win()
          .accounts({
            oracle: oracle.publicKey,
            winner: participant2.publicKey,
            betInfo: betInfoPDA,
            participant1: participant1.publicKey,
            participant2: participant2.publicKey,
            systemProgram: SystemProgram.programId,
          })
          .signers([oracle])
          .rpc()
      );

      expect(failed).to.be.true;
    });
  });

  /**
   * TIMEOUT FUNCTION TESTS
   */
  describe("timeout() - Deadline-Based Resolution", () => {
    let expiredBetPDA: PublicKey;
    
    beforeEach(async () => {
      // For timeout testing, we need to create a bet and then wait for it to expire
      // Since we can't advance slots in tests, we'll create the bet and immediately
      // try to force a slot advancement by making multiple transactions
      expiredBetPDA = betInfoPDA;
    });

    it("should prevent timeout before deadline with non-zero delay", async () => {
      // Create a fresh bet with actual delay for this test
      const participant3 = Keypair.generate();
      const participant4 = Keypair.generate();
      
      await Promise.all([
        provider.connection.requestAirdrop(participant3.publicKey, INITIAL_BALANCE),
        provider.connection.requestAirdrop(participant4.publicKey, INITIAL_BALANCE),
      ]);
      
      await new Promise(resolve => setTimeout(resolve, 1000));
      
      const [betInfo2PDA] = PublicKey.findProgramAddressSync(
        [participant3.publicKey.toBuffer(), participant4.publicKey.toBuffer()],
        program.programId
      );
      
      await program.methods
        .join(new BN(1000), WAGER_AMOUNT) // Long delay
        .accounts({
          participant1: participant3.publicKey,
          participant2: participant4.publicKey,
          oracle: oracle.publicKey,
          betInfo: betInfo2PDA,
          systemProgram: SystemProgram.programId,
        })
        .signers([participant3, participant4])
        .rpc();
      
      const failed = await expectTransactionToFail(
        program.methods
          .timeout()
          .accounts({
            participant1: participant3.publicKey,
            participant2: participant4.publicKey,
            betInfo: betInfo2PDA,
            systemProgram: SystemProgram.programId,
          })
          .rpc()
      );

      expect(failed).to.be.true;
    });

    it("should document timeout behavior (requires manual slot advancement)", async () => {
      // This test documents the expected timeout behavior
      // In a real test environment with slot advancement, this would work
      
      console.log("⚠️  Timeout testing requires blockchain slot advancement");
      console.log("⚠️  In production, use: solana-test-validator with slot control");
      console.log("⚠️  Or use Bankrun/other test frameworks with time manipulation");
      
      // Create a bet with minimal delay
      await program.methods
        .join(new BN(1), WAGER_AMOUNT) // 1 slot delay
        .accounts({
          participant1: participant1.publicKey,
          participant2: participant2.publicKey,
          oracle: oracle.publicKey,
          betInfo: betInfoPDA,
          systemProgram: SystemProgram.programId,
        })
        .signers([participant1, participant2])
        .rpc();
      
      // Verify that timeout fails when deadline hasn't passed
      const failed = await expectTransactionToFail(
        program.methods
          .timeout()
          .accounts({
            participant1: participant1.publicKey,
            participant2: participant2.publicKey,
            betInfo: betInfoPDA,
            systemProgram: SystemProgram.programId,
          })
          .rpc()
      );

      expect(failed).to.be.true;
      console.log("✓ Confirmed: timeout properly rejects when deadline not reached");
      
      // In a real environment, you would:
      // 1. Advance slots past the deadline
      // 2. Call timeout() successfully
      // 3. Verify refunds
    });

    it("should prevent timeout with wrong participants", async () => {
      // Create a bet first
      await program.methods
        .join(new BN(1), WAGER_AMOUNT)
        .accounts({
          participant1: participant1.publicKey,
          participant2: participant2.publicKey,
          oracle: oracle.publicKey,
          betInfo: betInfoPDA,
          systemProgram: SystemProgram.programId,
        })
        .signers([participant1, participant2])
        .rpc();
        
      const wrongParticipant = Keypair.generate();
      
      const failed = await expectTransactionToFail(
        program.methods
          .timeout()
          .accounts({
            participant1: wrongParticipant.publicKey, // Wrong participant
            participant2: participant2.publicKey,
            betInfo: betInfoPDA,
            systemProgram: SystemProgram.programId,
          })
          .rpc()
      );

      expect(failed).to.be.true;
    });

    it("should prevent timeout after resolution", async () => {
      // Create a separate bet for this test
      const participant5 = Keypair.generate();
      const participant6 = Keypair.generate();
      
      await Promise.all([
        provider.connection.requestAirdrop(participant5.publicKey, INITIAL_BALANCE),
        provider.connection.requestAirdrop(participant6.publicKey, INITIAL_BALANCE),
      ]);
      
      await new Promise(resolve => setTimeout(resolve, 1000));
      
      const [betInfo3PDA] = PublicKey.findProgramAddressSync(
        [participant5.publicKey.toBuffer(), participant6.publicKey.toBuffer()],
        program.programId
      );
      
      // Create bet
      await program.methods
        .join(new BN(1000), WAGER_AMOUNT) // Long delay
        .accounts({
          participant1: participant5.publicKey,
          participant2: participant6.publicKey,
          oracle: oracle.publicKey,
          betInfo: betInfo3PDA,
          systemProgram: SystemProgram.programId,
        })
        .signers([participant5, participant6])
        .rpc();

      // First resolve the bet
      await program.methods
        .win()
        .accounts({
          oracle: oracle.publicKey,
          winner: participant5.publicKey,
          betInfo: betInfo3PDA,
          participant1: participant5.publicKey,
          participant2: participant6.publicKey,
          systemProgram: SystemProgram.programId,
        })
        .signers([oracle])
        .rpc();

      // Then try to timeout (should fail because account is drained)
      const failed = await expectTransactionToFail(
        program.methods
          .timeout()
          .accounts({
            participant1: participant5.publicKey,
            participant2: participant6.publicKey,
            betInfo: betInfo3PDA,
            systemProgram: SystemProgram.programId,
          })
          .rpc()
      );

      expect(failed).to.be.true;
    });
  });

  /**
   * INTEGRATION AND EDGE CASE TESTS
   */
  describe("Integration and Edge Cases", () => {
    it("should handle multiple concurrent bets between different participants", async () => {
      const participant3 = Keypair.generate();
      const participant4 = Keypair.generate();
      
      await Promise.all([
        provider.connection.requestAirdrop(participant3.publicKey, INITIAL_BALANCE),
        provider.connection.requestAirdrop(participant4.publicKey, INITIAL_BALANCE),
      ]);

      await new Promise(resolve => setTimeout(resolve, 1000));

      // Create first bet
      await program.methods
        .join(DELAY_SLOTS, WAGER_AMOUNT)
        .accounts({
          participant1: participant1.publicKey,
          participant2: participant2.publicKey,
          oracle: oracle.publicKey,
          betInfo: betInfoPDA,
          systemProgram: SystemProgram.programId,
        })
        .signers([participant1, participant2])
        .rpc();

      // Create second bet with different participants
      const [betInfo2PDA] = PublicKey.findProgramAddressSync(
        [participant3.publicKey.toBuffer(), participant4.publicKey.toBuffer()],
        program.programId
      );

      await program.methods
        .join(DELAY_SLOTS, WAGER_AMOUNT)
        .accounts({
          participant1: participant3.publicKey,
          participant2: participant4.publicKey,
          oracle: oracle.publicKey,
          betInfo: betInfo2PDA,
          systemProgram: SystemProgram.programId,
        })
        .signers([participant3, participant4])
        .rpc();

      // Both bets should exist independently
      const bet1Balance = await getAccountBalance(betInfoPDA);
      const bet2Balance = await getAccountBalance(betInfo2PDA);
      
      expect(bet1Balance).to.be.at.least(
        WAGER_AMOUNT.toNumber() * 2 - LAMPORTS_PER_SOL * 0.01,
        `Bet1 should contain approximately 2x wager. Balance: ${bet1Balance}, Expected: ${WAGER_AMOUNT.toNumber() * 2}`
      );
      expect(bet2Balance).to.be.at.least(
        WAGER_AMOUNT.toNumber() * 2 - LAMPORTS_PER_SOL * 0.01,
        `Bet2 should contain approximately 2x wager. Balance: ${bet2Balance}, Expected: ${WAGER_AMOUNT.toNumber() * 2}`
      );
    });

    it("should handle minimum viable wager amounts", async () => {
      const minWager = new BN(1000); // Very small amount
      
      await program.methods
        .join(DELAY_SLOTS, minWager)
        .accounts({
          participant1: participant1.publicKey,
          participant2: participant2.publicKey,
          oracle: oracle.publicKey,
          betInfo: betInfoPDA,
          systemProgram: SystemProgram.programId,
        })
        .signers([participant1, participant2])
        .rpc();

      const betBalance = await getAccountBalance(betInfoPDA);
      expect(betBalance).to.be.at.least(
        minWager.toNumber() * 2 - 1000,
        `Min wager bet should contain approximately 2x wager. Balance: ${betBalance}, Expected: ${minWager.toNumber() * 2}`
      );
    });

    it("should handle large wager amounts", async () => {
      const largeWager = new BN(LAMPORTS_PER_SOL * 5); // 5 SOL
      
      await program.methods
        .join(DELAY_SLOTS, largeWager)
        .accounts({
          participant1: participant1.publicKey,
          participant2: participant2.publicKey,
          oracle: oracle.publicKey,
          betInfo: betInfoPDA,
          systemProgram: SystemProgram.programId,
        })
        .signers([participant1, participant2])
        .rpc();

      const betBalance = await getAccountBalance(betInfoPDA);
      expect(betBalance).to.be.at.least(
        largeWager.toNumber() * 2 - LAMPORTS_PER_SOL * 0.01,
        `Large wager bet should contain approximately 2x wager. Balance: ${betBalance}, Expected: ${largeWager.toNumber() * 2}`
      );
    });
  });
});

/**
 * USAGE NOTES:
 * 
 * 1. Update PROGRAM_ID_STRING with your actual program ID
 * 2. Update the program workspace name in the anchor.workspace line
 * 3. Install required dependencies:
 *    npm install --save-dev @coral-xyz/anchor @solana/web3.js bn.js chai mocha @types/chai @types/mocha
 * 
 * 4. Run tests with: anchor test
 * 
 * 5. For advanced slot manipulation, consider using a test validator with:
 *    - solana-test-validator with --slots-per-epoch for faster slot advancement
 *    - Custom RPC calls to advance slots in test environments
 * 
 * 6. This test suite focuses on behavioral contracts rather than implementation details,
 *    making it reusable across different betting program implementations that follow
 *    the same interface pattern.
 * 
 * 7. All tests use lamports balance verification instead of inspecting account data,
 *    making them resilient to different internal state representations.
 */
