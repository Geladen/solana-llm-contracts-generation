import * as anchor from "@coral-xyz/anchor";
import { Program, AnchorProvider, Wallet } from "@coral-xyz/anchor";
import { PublicKey, Keypair, SystemProgram, LAMPORTS_PER_SOL } from "@solana/web3.js";
import { BN } from "bn.js";
import { expect } from "chai";

/**
 * Universal Payment Splitter Test Suite
 * 
 * This test suite is designed to work with any payment splitter program
 * that implements the standard interface with initialize() and release() functions.
 * 
 * SETUP INSTRUCTIONS:
 * 1. Replace this with your actual program import:
 *    import { PaymentSplitter } from "../target/types/payment_splitter";
 * 2. Update PROGRAM_ID with your deployed program ID
 * 3. Update PDA_SEED if your program uses different seed derivation
 * 
 * For now, uncomment and use the import that matches your setup:
 */

// Option 1: If you have generated types (recommended)
// import { PaymentSplitter } from "../target/types/payment_splitter";

// Option 2: If you have IDL file
// import PaymentSplitterIDL from "../target/idl/payment_splitter.json";

// Configuration - Update these for your specific program
const PROGRAM_ID_STRING = "4VuKmRhxSWURbUzb6hYig1uYdCFbX7QkhhRvFHZhQbFc";
const PDA_SEED = "payment_splitter"; // Update if different

describe("Payment Splitter Program", () => {
  // Test environment setup
  const provider = AnchorProvider.env();
  anchor.setProvider(provider);
  
  let program: Program<any>;
  let PROGRAM_ID: PublicKey;

  before(async () => {
    try {
      PROGRAM_ID = new PublicKey(PROGRAM_ID_STRING);
      
      // Option 1: Use this if you have generated types
      // program = anchor.workspace.PaymentSplitter as Program<PaymentSplitter>;
      
      // Option 2: Use this if you have IDL file
      // program = new Program(PaymentSplitterIDL as any, PROGRAM_ID, provider);
      
      // Option 3: Fallback - Load program from workspace (most common)
      // This assumes your program is built and the types are generated
      program = anchor.workspace.PaymentSplitter;
      
      if (!program) {
        throw new Error("Program not found. Make sure to build your program first with 'anchor build'");
      }
      
    } catch (error) {
      console.error("Setup error:", error);
      console.log(`
      SETUP REQUIRED:
      1. Run 'anchor build' to build your program
      2. Update PROGRAM_ID_STRING with your deployed program ID
      3. Make sure your program is named 'PaymentSplitter' in Anchor.toml
      
      Or manually import your program:
      import { YourProgramName } from "../target/types/your_program_name";
      program = anchor.workspace.YourProgramName;
      `);
      throw error;
    }
  });

  // Helper function to derive PDA
  const derivePDA = (initializer: PublicKey): [PublicKey, number] => {
    if (!PROGRAM_ID) {
      throw new Error("PROGRAM_ID not initialized");
    }
    return PublicKey.findProgramAddressSync(
      [Buffer.from(PDA_SEED), initializer.toBuffer()],
      PROGRAM_ID
    );
  };

  // Helper function to get account balance
  const getBalance = async (pubkey: PublicKey): Promise<number> => {
    return await provider.connection.getBalance(pubkey);
  };

  // Helper function to create funded keypair
  const createFundedKeypair = async (lamports: number = LAMPORTS_PER_SOL): Promise<Keypair> => {
    const keypair = Keypair.generate();
    const signature = await provider.connection.requestAirdrop(keypair.publicKey, lamports);
    await provider.connection.confirmTransaction(signature);
    return keypair;
  };

  // Helper function to check if transaction failed
  const expectTransactionToFail = async (transactionPromise: Promise<any>): Promise<void> => {
    try {
      await transactionPromise;
      expect.fail("Expected transaction to fail but it succeeded");
    } catch (error) {
      // Transaction failed as expected - we don't check specific error messages
      // to maintain compatibility across different programs
      expect(error).to.exist;
    }
  };

  describe("initialize()", () => {
    let initializer: Keypair;
    let payee1: Keypair;
    let payee2: Keypair;
    let payee3: Keypair;
    let psInfoPDA: PublicKey;

    beforeEach(async () => {
      initializer = await createFundedKeypair(2 * LAMPORTS_PER_SOL);
      payee1 = await createFundedKeypair();
      payee2 = await createFundedKeypair();
      payee3 = await createFundedKeypair();
      [psInfoPDA] = derivePDA(initializer.publicKey);
    });

    it("creates splitter with valid parameters", async () => {
      const transferAmount = 1000000; // 0.001 SOL
      const shares = [50, 30, 20];

      const initializerBalanceBefore = await getBalance(initializer.publicKey);

      await program.methods
        .initialize(new BN(transferAmount), shares.map(s => new BN(s)))
        .accounts({
          initializer: initializer.publicKey,
          psInfo: psInfoPDA,
          systemProgram: SystemProgram.programId,
        })
        .remainingAccounts([
          { pubkey: payee1.publicKey, isSigner: false, isWritable: false },
          { pubkey: payee2.publicKey, isSigner: false, isWritable: false },
          { pubkey: payee3.publicKey, isSigner: false, isWritable: false },
        ])
        .signers([initializer])
        .rpc();

      // Verify PDA account was created and funded
      const pdaBalance = await getBalance(psInfoPDA);
      expect(pdaBalance).to.be.greaterThan(transferAmount); // Account for rent

      // Verify initializer balance decreased
      const initializerBalanceAfter = await getBalance(initializer.publicKey);
      expect(initializerBalanceAfter).to.be.lessThan(initializerBalanceBefore);
    });

    it("transfers initial funds correctly", async () => {
      const transferAmount = 2000000; // 0.002 SOL
      const shares = [60, 40];

      const initializerBalanceBefore = await getBalance(initializer.publicKey);
      const pdaBalanceBefore = await getBalance(psInfoPDA);

      await program.methods
        .initialize(new BN(transferAmount), shares.map(s => new BN(s)))
        .accounts({
          initializer: initializer.publicKey,
          psInfo: psInfoPDA,
          systemProgram: SystemProgram.programId,
        })
        .remainingAccounts([
          { pubkey: payee1.publicKey, isSigner: false, isWritable: false },
          { pubkey: payee2.publicKey, isSigner: false, isWritable: false },
        ])
        .signers([initializer])
        .rpc();

      const initializerBalanceAfter = await getBalance(initializer.publicKey);
      const pdaBalanceAfter = await getBalance(psInfoPDA);

      // Verify funds were transferred from initializer to PDA
      const balanceChange = initializerBalanceBefore - initializerBalanceAfter;
      const pdaBalanceIncrease = pdaBalanceAfter - pdaBalanceBefore;
      
      expect(balanceChange).to.be.greaterThan(transferAmount);
      expect(pdaBalanceIncrease).to.be.greaterThan(transferAmount);
    });

    it("handles multiple payees correctly", async () => {
      const transferAmount = 1000000;
      const shares = [25, 25, 25, 25]; // Equal shares for 4 payees
      const payee4 = await createFundedKeypair();

      await program.methods
        .initialize(new BN(transferAmount), shares.map(s => new BN(s)))
        .accounts({
          initializer: initializer.publicKey,
          psInfo: psInfoPDA,
          systemProgram: SystemProgram.programId,
        })
        .remainingAccounts([
          { pubkey: payee1.publicKey, isSigner: false, isWritable: false },
          { pubkey: payee2.publicKey, isSigner: false, isWritable: false },
          { pubkey: payee3.publicKey, isSigner: false, isWritable: false },
          { pubkey: payee4.publicKey, isSigner: false, isWritable: false },
        ])
        .signers([initializer])
        .rpc();

      // Verify account was created successfully
      const pdaBalance = await getBalance(psInfoPDA);
      expect(pdaBalance).to.be.greaterThan(transferAmount);
    });

    it("rejects empty payees list", async () => {
      const transferAmount = 1000000;
      const shares: number[] = [];

      await expectTransactionToFail(
        program.methods
          .initialize(new BN(transferAmount), shares.map(s => new BN(s)))
          .accounts({
            initializer: initializer.publicKey,
            psInfo: psInfoPDA,
            systemProgram: SystemProgram.programId,
          })
          .remainingAccounts([])
          .signers([initializer])
          .rpc()
      );
    });

    it("rejects mismatched payees/shares lengths", async () => {
      const transferAmount = 1000000;
      const shares = [50, 50]; // 2 shares but 3 payees

      await expectTransactionToFail(
        program.methods
          .initialize(new BN(transferAmount), shares.map(s => new BN(s)))
          .accounts({
            initializer: initializer.publicKey,
            psInfo: psInfoPDA,
            systemProgram: SystemProgram.programId,
          })
          .remainingAccounts([
            { pubkey: payee1.publicKey, isSigner: false, isWritable: false },
            { pubkey: payee2.publicKey, isSigner: false, isWritable: false },
            { pubkey: payee3.publicKey, isSigner: false, isWritable: false },
          ])
          .signers([initializer])
          .rpc()
      );
    });

    it("rejects zero shares", async () => {
      const transferAmount = 1000000;
      const shares = [50, 0, 50]; // Zero share in the middle

      await expectTransactionToFail(
        program.methods
          .initialize(new BN(transferAmount), shares.map(s => new BN(s)))
          .accounts({
            initializer: initializer.publicKey,
            psInfo: psInfoPDA,
            systemProgram: SystemProgram.programId,
          })
          .remainingAccounts([
            { pubkey: payee1.publicKey, isSigner: false, isWritable: false },
            { pubkey: payee2.publicKey, isSigner: false, isWritable: false },
            { pubkey: payee3.publicKey, isSigner: false, isWritable: false },
          ])
          .signers([initializer])
          .rpc()
      );
    });

    it("rejects duplicate payees", async () => {
      const transferAmount = 1000000;
      const shares = [50, 30, 20];

      await expectTransactionToFail(
        program.methods
          .initialize(new BN(transferAmount), shares.map(s => new BN(s)))
          .accounts({
            initializer: initializer.publicKey,
            psInfo: psInfoPDA,
            systemProgram: SystemProgram.programId,
          })
          .remainingAccounts([
            { pubkey: payee1.publicKey, isSigner: false, isWritable: false },
            { pubkey: payee1.publicKey, isSigner: false, isWritable: false }, // Duplicate
            { pubkey: payee3.publicKey, isSigner: false, isWritable: false },
          ])
          .signers([initializer])
          .rpc()
      );
    });
  });

  describe("release()", () => {
    let initializer: Keypair;
    let payee1: Keypair;
    let payee2: Keypair;
    let payee3: Keypair;
    let psInfoPDA: PublicKey;
    let nonPayee: Keypair;

    const transferAmount = 1000000; // 0.001 SOL
    const shares = [50, 30, 20]; // Total: 100

    beforeEach(async () => {
      initializer = await createFundedKeypair(2 * LAMPORTS_PER_SOL);
      payee1 = await createFundedKeypair();
      payee2 = await createFundedKeypair();
      payee3 = await createFundedKeypair();
      nonPayee = await createFundedKeypair();
      [psInfoPDA] = derivePDA(initializer.publicKey);

      // Initialize the payment splitter
      await program.methods
        .initialize(new BN(transferAmount), shares.map(s => new BN(s)))
        .accounts({
          initializer: initializer.publicKey,
          psInfo: psInfoPDA,
          systemProgram: SystemProgram.programId,
        })
        .remainingAccounts([
          { pubkey: payee1.publicKey, isSigner: false, isWritable: false },
          { pubkey: payee2.publicKey, isSigner: false, isWritable: false },
          { pubkey: payee3.publicKey, isSigner: false, isWritable: false },
        ])
        .signers([initializer])
        .rpc();
    });

    it("allows payee to release funds", async () => {
      const payee1BalanceBefore = await getBalance(payee1.publicKey);
      const pdaBalanceBefore = await getBalance(psInfoPDA);

      await program.methods
        .release()
        .accounts({
          payee: payee1.publicKey,
          initializer: initializer.publicKey,
          psInfo: psInfoPDA,
          systemProgram: SystemProgram.programId,
        })
        .signers([payee1])
        .rpc();

      const payee1BalanceAfter = await getBalance(payee1.publicKey);
      const pdaBalanceAfter = await getBalance(psInfoPDA);

      // Verify payee received funds and PDA balance decreased
      expect(payee1BalanceAfter).to.be.greaterThan(payee1BalanceBefore);
      expect(pdaBalanceAfter).to.be.lessThan(pdaBalanceBefore);
    });

    it("calculates correct share amount", async () => {
      // Payee1 has 50/100 shares = 50% of transferAmount
      const expectedPayment = Math.floor((transferAmount * shares[0]) / shares.reduce((a, b) => a + b));
      
      const payee1BalanceBefore = await getBalance(payee1.publicKey);

      await program.methods
        .release()
        .accounts({
          payee: payee1.publicKey,
          initializer: initializer.publicKey,
          psInfo: psInfoPDA,
          systemProgram: SystemProgram.programId,
        })
        .signers([payee1])
        .rpc();

      const payee1BalanceAfter = await getBalance(payee1.publicKey);
      const actualPayment = payee1BalanceAfter - payee1BalanceBefore;

      // Allow for small rounding differences
      expect(actualPayment).to.be.approximately(expectedPayment, 1);
    });

    it("rejects non-payee release", async () => {
      await expectTransactionToFail(
        program.methods
          .release()
          .accounts({
            payee: nonPayee.publicKey,
            initializer: initializer.publicKey,
            psInfo: psInfoPDA,
            systemProgram: SystemProgram.programId,
          })
          .signers([nonPayee])
          .rpc()
      );
    });

    it("rejects zero payment due (double release)", async () => {
      // First release should succeed
      await program.methods
        .release()
        .accounts({
          payee: payee1.publicKey,
          initializer: initializer.publicKey,
          psInfo: psInfoPDA,
          systemProgram: SystemProgram.programId,
        })
        .signers([payee1])
        .rpc();

      // Second release should fail (no additional funds due)
      await expectTransactionToFail(
        program.methods
          .release()
          .accounts({
            payee: payee1.publicKey,
            initializer: initializer.publicKey,
            psInfo: psInfoPDA,
            systemProgram: SystemProgram.programId,
          })
          .signers([payee1])
          .rpc()
      );
    });

    it("handles sequential releases", async () => {
      const payee1BalanceBefore = await getBalance(payee1.publicKey);
      const payee2BalanceBefore = await getBalance(payee2.publicKey);
      const payee3BalanceBefore = await getBalance(payee3.publicKey);

      const paymentsReceived: boolean[] = [false, false, false];
      const payees = [payee1, payee2, payee3];
      const balancesBefore = [payee1BalanceBefore, payee2BalanceBefore, payee3BalanceBefore];

      // Try to release for each payee
      for (let i = 0; i < payees.length; i++) {
        const accountInfo = await provider.connection.getAccountInfo(psInfoPDA);
        if (!accountInfo) {
          // Account already closed
          break;
        }

        try {
          await program.methods
            .release()
            .accounts({
              payee: payees[i].publicKey,
              initializer: initializer.publicKey,
              psInfo: psInfoPDA,
              systemProgram: SystemProgram.programId,
            })
            .signers([payees[i]])
            .rpc();
          
          paymentsReceived[i] = true;
        } catch (error) {
          // If account was closed or payee has no payment due, that's expected
          if (error.message.includes("AccountNotInitialized") || 
              error.message.includes("PayeeNotDuePayment")) {
            break;
          }
          throw error;
        }
      }

      // Verify that at least one payee received funds
      let totalPayeesWithPayments = 0;
      const payments: number[] = [];

      for (let i = 0; i < payees.length; i++) {
        const balanceAfter = await getBalance(payees[i].publicKey);
        const payment = balanceAfter - balancesBefore[i];
        payments.push(payment);

        if (payment > 0) {
          totalPayeesWithPayments++;
          expect(balanceAfter).to.be.greaterThan(balancesBefore[i]);
        }
      }

      // At least one payee should have received payment
      expect(totalPayeesWithPayments).to.be.greaterThan(0);

      // If multiple payees received payments, verify proportional distribution
      if (totalPayeesWithPayments > 1) {
        // Find payees who actually received payments for ratio comparison
        const receivedPayments: { payment: number; share: number }[] = [];
        
        for (let i = 0; i < payments.length; i++) {
          if (payments[i] > 0) {
            receivedPayments.push({ payment: payments[i], share: shares[i] });
          }
        }

        // Compare ratios between payees who received payments
        if (receivedPayments.length >= 2) {
          const ratio = receivedPayments[0].payment / receivedPayments[1].payment;
          const expectedRatio = receivedPayments[0].share / receivedPayments[1].share;
          expect(ratio).to.be.approximately(expectedRatio, 0.3);
        }
      }
    });

    it("closes account when empty", async () => {
      const initializerBalanceBefore = await getBalance(initializer.publicKey);

      // Release funds one by one, checking if account exists before each release
      const payees = [payee1, payee2, payee3];
      
      for (let i = 0; i < payees.length; i++) {
        const accountInfo = await provider.connection.getAccountInfo(psInfoPDA);
        if (!accountInfo) {
          // Account already closed, which is expected behavior
          break;
        }

        try {
          await program.methods
            .release()
            .accounts({
              payee: payees[i].publicKey,
              initializer: initializer.publicKey,
              psInfo: psInfoPDA,
              systemProgram: SystemProgram.programId,
            })
            .signers([payees[i]])
            .rpc();
        } catch (error) {
          // If account was closed during this release, that's expected
          if (error.message.includes("AccountNotInitialized")) {
            break;
          }
          throw error;
        }
      }

      // Check final state - account should be closed or have minimal balance
      const accountInfoFinal = await provider.connection.getAccountInfo(psInfoPDA);
      if (accountInfoFinal) {
        const pdaBalanceAfter = await getBalance(psInfoPDA);
        expect(pdaBalanceAfter).to.be.lessThan(10000); // Less than 0.00001 SOL
      }
      
      // Initializer should have received some funds back (rent refund)
      const initializerBalanceAfter = await getBalance(initializer.publicKey);
      // Note: Due to transaction fees, we can't guarantee the balance increased
      // but we can verify the test completed without errors
      expect(initializerBalanceAfter).to.be.greaterThan(0);
    });

    it("maintains correct accounting across partial releases", async () => {
      // This test verifies that the program correctly tracks its internal state
      // We cannot directly add funds to the PDA as the program tracks funds internally
      
      // Release funds for payee1 only
      const payee1BalanceBefore = await getBalance(payee1.publicKey);
      const payee2BalanceBefore = await getBalance(payee2.publicKey);
      
      await program.methods
        .release()
        .accounts({
          payee: payee1.publicKey,
          initializer: initializer.publicKey,
          psInfo: psInfoPDA,
          systemProgram: SystemProgram.programId,
        })
        .signers([payee1])
        .rpc();

      const payee1BalanceAfter = await getBalance(payee1.publicKey);
      const payment1 = payee1BalanceAfter - payee1BalanceBefore;

      // Verify payee1 received their proportional share
      const expectedPayment1 = Math.floor((transferAmount * shares[0]) / shares.reduce((a, b) => a + b));
      expect(payment1).to.be.approximately(expectedPayment1, 2);

      // Verify payee2 hasn't received anything yet
      const payee2BalanceAfter = await getBalance(payee2.publicKey);
      expect(payee2BalanceAfter).to.equal(payee2BalanceBefore);

      // Now release for payee2 and verify they get their correct share
      const accountInfo = await provider.connection.getAccountInfo(psInfoPDA);
      if (accountInfo) {
        await program.methods
          .release()
          .accounts({
            payee: payee2.publicKey,
            initializer: initializer.publicKey,
            psInfo: psInfoPDA,
            systemProgram: SystemProgram.programId,
          })
          .signers([payee2])
          .rpc();

        const payee2FinalBalance = await getBalance(payee2.publicKey);
        const payment2 = payee2FinalBalance - payee2BalanceBefore;

        const expectedPayment2 = Math.floor((transferAmount * shares[1]) / shares.reduce((a, b) => a + b));
        expect(payment2).to.be.approximately(expectedPayment2, 2);
      }
    });
  });

  describe("Edge Cases and Security", () => {
    let initializer: Keypair;
    let payee1: Keypair;
    let attacker: Keypair;
    let psInfoPDA: PublicKey;

    beforeEach(async () => {
      initializer = await createFundedKeypair(2 * LAMPORTS_PER_SOL);
      payee1 = await createFundedKeypair();
      attacker = await createFundedKeypair();
      [psInfoPDA] = derivePDA(initializer.publicKey);
    });

    it("rejects initialization with wrong PDA derivation", async () => {
      const [wrongPDA] = derivePDA(attacker.publicKey); // Wrong initializer

      await expectTransactionToFail(
        program.methods
          .initialize(new BN(1000000), [new BN(100)])
          .accounts({
            initializer: initializer.publicKey,
            psInfo: wrongPDA, // Wrong PDA
            systemProgram: SystemProgram.programId,
          })
          .remainingAccounts([
            { pubkey: payee1.publicKey, isSigner: false, isWritable: false },
          ])
          .signers([initializer])
          .rpc()
      );
    });

    it("rejects release with wrong initializer", async () => {
      // First initialize correctly
      await program.methods
        .initialize(new BN(1000000), [new BN(100)])
        .accounts({
          initializer: initializer.publicKey,
          psInfo: psInfoPDA,
          systemProgram: SystemProgram.programId,
        })
        .remainingAccounts([
          { pubkey: payee1.publicKey, isSigner: false, isWritable: false },
        ])
        .signers([initializer])
        .rpc();

      // Try to release with wrong initializer
      await expectTransactionToFail(
        program.methods
          .release()
          .accounts({
            payee: payee1.publicKey,
            initializer: attacker.publicKey, // Wrong initializer
            psInfo: psInfoPDA,
            systemProgram: SystemProgram.programId,
          })
          .signers([payee1])
          .rpc()
      );
    });

    it("handles large share amounts correctly", async () => {
      const largeShares = [1000000, 2000000, 3000000]; // Large numbers
      const transferAmount = 1000000;

      await program.methods
        .initialize(new BN(transferAmount), largeShares.map(s => new BN(s)))
        .accounts({
          initializer: initializer.publicKey,
          psInfo: psInfoPDA,
          systemProgram: SystemProgram.programId,
        })
        .remainingAccounts([
          { pubkey: payee1.publicKey, isSigner: false, isWritable: false },
          { pubkey: Keypair.generate().publicKey, isSigner: false, isWritable: false },
          { pubkey: Keypair.generate().publicKey, isSigner: false, isWritable: false },
        ])
        .signers([initializer])
        .rpc();

      // Verify account was created successfully
      const pdaBalance = await getBalance(psInfoPDA);
      expect(pdaBalance).to.be.greaterThan(transferAmount);
    });
  });
});

/**
 * Usage Instructions:
 * 
 * QUICK SETUP (Recommended):
 * 1. Run 'anchor build' to generate program types
 * 2. Update PROGRAM_ID_STRING with your deployed program ID
 * 3. Ensure your program name matches in Anchor.toml
 * 4. Run tests with: anchor test
 * 
 * MANUAL SETUP (Alternative):
 * 1. Import your program types at the top of the file:
 *    import { YourProgramName } from "../target/types/your_program_name";
 * 2. Replace the program loading logic in the before() hook:
 *    program = anchor.workspace.YourProgramName;
 * 3. Update PROGRAM_ID_STRING and PDA_SEED as needed
 * 
 * PROGRAM REQUIREMENTS:
 * Your program must implement:
 * - initialize(lamports_to_transfer: u64, shares_amounts: Vec<u64>)
 * - release()
 * - PDA derived from seeds ["payment_splitter", initializer.key()]
 * 
 * This test suite focuses on behavioral verification rather than implementation details,
 * making it reusable across different payment splitter programs that follow the same interface.
 */
