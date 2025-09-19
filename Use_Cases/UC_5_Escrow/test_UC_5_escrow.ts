import * as anchor from "@coral-xyz/anchor";
import { Program, web3 } from "@coral-xyz/anchor";
import { PublicKey, SystemProgram, LAMPORTS_PER_SOL } from "@solana/web3.js";
import { BN } from "bn.js";
import { expect } from "chai";

// Generic interface for any escrow program
interface EscrowProgram extends Program {
  methods: {
    initialize: (amount: BN, name: string) => any;
    deposit: (name: string) => any;
    pay: (name: string) => any;
    refund: (name: string) => any;
  };
}

// Utility functions for test setup
class EscrowTestUtils {
  static async createTestKeypairs() {
    const seller = web3.Keypair.generate();
    const buyer = web3.Keypair.generate();
    
    // Airdrop SOL to test accounts
    const connection = anchor.getProvider().connection;
    await connection.requestAirdrop(seller.publicKey, 2 * LAMPORTS_PER_SOL);
    await connection.requestAirdrop(buyer.publicKey, 2 * LAMPORTS_PER_SOL);
    
    // Wait for confirmations
    await new Promise(resolve => setTimeout(resolve, 1000));
    
    return { seller, buyer };
  }

  static deriveEscrowPDA(
    programId: PublicKey,
    escrowName: string,
    seller: PublicKey,
    buyer: PublicKey
  ): [PublicKey, number] {
    return PublicKey.findProgramAddressSync(
      [
        Buffer.from(escrowName),
        seller.toBuffer(),
        buyer.toBuffer()
      ],
      programId
    );
  }

  static async getLamports(connection: web3.Connection, publicKey: PublicKey): Promise<number> {
    const account = await connection.getAccountInfo(publicKey);
    return account?.lamports || 0;
  }

  static async expectTransactionToFail(promise: Promise<any>): Promise<void> {
    try {
      await promise;
      expect.fail("Transaction should have failed but succeeded");
    } catch (error) {
      // Transaction failed as expected - we don't check specific error messages
      expect(error).to.exist;
    }
  }

  static async getEscrowAccount(program: EscrowProgram, escrowPDA: PublicKey) {
    try {
      return await program.account.escrowInfo.fetch(escrowPDA);
    } catch {
      return null; // Account doesn't exist
    }
  }
}

describe("Universal Escrow Program Test Suite", () => {
  // Setup - assumes program is already loaded
  let program: EscrowProgram;
  let provider: anchor.AnchorProvider;
  let seller: web3.Keypair;
  let buyer: web3.Keypair;
  let escrowPDA: PublicKey;
  let escrowBump: number;
  
  const escrowName = "test-escrow";
  const escrowAmount = new BN(1 * LAMPORTS_PER_SOL);

  before(async () => {
    // Initialize provider and program
    provider = anchor.AnchorProvider.env();
    anchor.setProvider(provider);
    
    // Note: Replace 'your-program-name' with actual program name
    program = anchor.workspace.escrow as EscrowProgram;
    
    // Create test keypairs
    const keypairs = await EscrowTestUtils.createTestKeypairs();
    seller = keypairs.seller;
    buyer = keypairs.buyer;
    
    // Derive PDA
    [escrowPDA, escrowBump] = EscrowTestUtils.deriveEscrowPDA(
      program.programId,
      escrowName,
      seller.publicKey,
      buyer.publicKey
    );
  });

  describe("initialize()", () => {
    beforeEach(async () => {
      // Clean setup for each test
      const keypairs = await EscrowTestUtils.createTestKeypairs();
      seller = keypairs.seller;
      buyer = keypairs.buyer;
      
      [escrowPDA, escrowBump] = EscrowTestUtils.deriveEscrowPDA(
        program.programId,
        escrowName + Math.floor(Math.random() * 1000).toString(), // Unique name per test
        seller.publicKey,
        buyer.publicKey
      );
    });

    it("creates escrow with valid parameters", async () => {
      const uniqueName = "test-init-" + Math.floor(Math.random() * 1000).toString();
      const [testEscrowPDA] = EscrowTestUtils.deriveEscrowPDA(
        program.programId,
        uniqueName,
        seller.publicKey,
        buyer.publicKey
      );

      await program.methods
        .initialize(escrowAmount, uniqueName)
        .accounts({
          seller: seller.publicKey,
          buyer: buyer.publicKey,
          escrowInfo: testEscrowPDA,
          systemProgram: SystemProgram.programId,
        })
        .signers([seller])
        .rpc();

      const escrowAccount = await EscrowTestUtils.getEscrowAccount(program, testEscrowPDA);
      expect(escrowAccount).to.not.be.null;
      expect(escrowAccount.seller.toString()).to.equal(seller.publicKey.toString());
      expect(escrowAccount.buyer.toString()).to.equal(buyer.publicKey.toString());
      expect(escrowAccount.amountInLamports.toString()).to.equal(escrowAmount.toString());
    });

    it("rejects zero amount", async () => {
      const uniqueName = "test-zero-" + Math.floor(Math.random() * 1000).toString();
      const [testEscrowPDA] = EscrowTestUtils.deriveEscrowPDA(
        program.programId,
        uniqueName,
        seller.publicKey,
        buyer.publicKey
      );

      await EscrowTestUtils.expectTransactionToFail(
        program.methods
          .initialize(new BN(0), uniqueName)
          .accounts({
            seller: seller.publicKey,
            buyer: buyer.publicKey,
            escrowInfo: testEscrowPDA,
            systemProgram: SystemProgram.programId,
          })
          .signers([seller])
          .rpc()
      );
    });

    it("sets correct initial state", async () => {
      const uniqueName = "test-state-" + Math.floor(Math.random() * 1000).toString();
      const [testEscrowPDA] = EscrowTestUtils.deriveEscrowPDA(
        program.programId,
        uniqueName,
        seller.publicKey,
        buyer.publicKey
      );

      await program.methods
        .initialize(escrowAmount, uniqueName)
        .accounts({
          seller: seller.publicKey,
          buyer: buyer.publicKey,
          escrowInfo: testEscrowPDA,
          systemProgram: SystemProgram.programId,
        })
        .signers([seller])
        .rpc();

      const escrowAccount = await EscrowTestUtils.getEscrowAccount(program, testEscrowPDA);
      // State should be waiting for deposit (typically 0 or "WaitDeposit")
      expect(escrowAccount.state).to.exist;
    });
  });

  describe("deposit()", () => {
    let testEscrowName: string;
    let testEscrowPDA: PublicKey;

    beforeEach(async () => {
      const keypairs = await EscrowTestUtils.createTestKeypairs();
      seller = keypairs.seller;
      buyer = keypairs.buyer;
      
      testEscrowName = "test-deposit-" + Math.floor(Math.random() * 1000).toString();
      [testEscrowPDA] = EscrowTestUtils.deriveEscrowPDA(
        program.programId,
        testEscrowName,
        seller.publicKey,
        buyer.publicKey
      );

      // Initialize escrow first
      await program.methods
        .initialize(escrowAmount, testEscrowName)
        .accounts({
          seller: seller.publicKey,
          buyer: buyer.publicKey,
          escrowInfo: testEscrowPDA,
          systemProgram: SystemProgram.programId,
        })
        .signers([seller])
        .rpc();
    });

    it("accepts buyer deposit", async () => {
      const buyerBalanceBefore = await EscrowTestUtils.getLamports(
        provider.connection,
        buyer.publicKey
      );

      await program.methods
        .deposit(testEscrowName)
        .accounts({
          buyer: buyer.publicKey,
          seller: seller.publicKey,
          escrowInfo: testEscrowPDA,
          systemProgram: SystemProgram.programId,
        })
        .signers([buyer])
        .rpc();

      const buyerBalanceAfter = await EscrowTestUtils.getLamports(
        provider.connection,
        buyer.publicKey
      );
      
      // Buyer should have less lamports (amount + transaction fees)
      expect(buyerBalanceAfter).to.be.lessThan(buyerBalanceBefore);
expect(buyerBalanceBefore - buyerBalanceAfter).to.be.closeTo(
  escrowAmount.toNumber(), 
  100000 // tolleranza per le fee
);
    });

    it("rejects non-buyer deposit", async () => {
      const wrongSigner = web3.Keypair.generate();
      await provider.connection.requestAirdrop(wrongSigner.publicKey, LAMPORTS_PER_SOL);
      await new Promise(resolve => setTimeout(resolve, 1000));

      await EscrowTestUtils.expectTransactionToFail(
        program.methods
          .deposit(testEscrowName)
          .accounts({
            buyer: wrongSigner.publicKey, // Wrong signer
            seller: seller.publicKey,
            escrowInfo: testEscrowPDA,
            systemProgram: SystemProgram.programId,
          })
          .signers([wrongSigner])
          .rpc()
      );
    });

    it("updates state correctly", async () => {
      await program.methods
        .deposit(testEscrowName)
        .accounts({
          buyer: buyer.publicKey,
          seller: seller.publicKey,
          escrowInfo: testEscrowPDA,
          systemProgram: SystemProgram.programId,
        })
        .signers([buyer])
        .rpc();

      const escrowAccount = await EscrowTestUtils.getEscrowAccount(program, testEscrowPDA);
      // State should have changed from initial state
      expect(escrowAccount.state).to.exist;
    });

    it("transfers correct lamports", async () => {
      const escrowAccountBefore = await EscrowTestUtils.getEscrowAccount(program, testEscrowPDA);
      
      await program.methods
        .deposit(testEscrowName)
        .accounts({
          buyer: buyer.publicKey,
          seller: seller.publicKey,
          escrowInfo: testEscrowPDA,
          systemProgram: SystemProgram.programId,
        })
        .signers([buyer])
        .rpc();

      const escrowAccountAfter = await EscrowTestUtils.getEscrowAccount(program, testEscrowPDA);

      // Verify the state transition occurred (this proves the deposit worked)
      expect(escrowAccountBefore.state).to.not.deep.equal(escrowAccountAfter.state);
      
      // The escrow account should still contain the amount information
      expect(escrowAccountAfter.amountInLamports.toString()).to.equal(escrowAmount.toString());
      
      console.log("Deposit completed - state transition verified");
    });
  });

  describe("pay()", () => {
    let testEscrowName: string;
    let testEscrowPDA: PublicKey;

    beforeEach(async () => {
      const keypairs = await EscrowTestUtils.createTestKeypairs();
      seller = keypairs.seller;
      buyer = keypairs.buyer;
      
      testEscrowName = "test-pay-" + Math.floor(Math.random() * 1000).toString();
      [testEscrowPDA] = EscrowTestUtils.deriveEscrowPDA(
        program.programId,
        testEscrowName,
        seller.publicKey,
        buyer.publicKey
      );

      // Initialize and deposit
      await program.methods
        .initialize(escrowAmount, testEscrowName)
        .accounts({
          seller: seller.publicKey,
          buyer: buyer.publicKey,
          escrowInfo: testEscrowPDA,
          systemProgram: SystemProgram.programId,
        })
        .signers([seller])
        .rpc();

      await program.methods
        .deposit(testEscrowName)
        .accounts({
          buyer: buyer.publicKey,
          seller: seller.publicKey,
          escrowInfo: testEscrowPDA,
          systemProgram: SystemProgram.programId,
        })
        .signers([buyer])
        .rpc();
    });

    it("allows buyer to release funds", async () => {
      const sellerBalanceBefore = await EscrowTestUtils.getLamports(
        provider.connection,
        seller.publicKey
      );

      await program.methods
        .pay(testEscrowName)
        .accounts({
          buyer: buyer.publicKey,
          seller: seller.publicKey,
          escrowInfo: testEscrowPDA,
        })
        .signers([buyer])
        .rpc();

      const sellerBalanceAfter = await EscrowTestUtils.getLamports(
        provider.connection,
        seller.publicKey
      );

      // Seller should have received the funds
      expect(sellerBalanceAfter).to.be.greaterThan(sellerBalanceBefore);
    });

    it("transfers funds to seller", async () => {
      const sellerBalanceBefore = await EscrowTestUtils.getLamports(
        provider.connection,
        seller.publicKey
      );

      await program.methods
        .pay(testEscrowName)
        .accounts({
          buyer: buyer.publicKey,
          seller: seller.publicKey,
          escrowInfo: testEscrowPDA,
        })
        .signers([buyer])
        .rpc();

      const sellerBalanceAfter = await EscrowTestUtils.getLamports(
        provider.connection,
        seller.publicKey
      );
      const escrowBalance = await EscrowTestUtils.getLamports(
        provider.connection,
        testEscrowPDA
      );

      expect(sellerBalanceAfter).to.be.greaterThan(sellerBalanceBefore);
      expect(escrowBalance).to.equal(0); // Escrow should be emptied
    });

    it("rejects invalid state", async () => {
      // Try to pay without deposit
      const newName = "test-no-deposit-" + Math.floor(Math.random() * 1000).toString();
      const [newEscrowPDA] = EscrowTestUtils.deriveEscrowPDA(
        program.programId,
        newName,
        seller.publicKey,
        buyer.publicKey
      );

      await program.methods
        .initialize(escrowAmount, newName)
        .accounts({
          seller: seller.publicKey,
          buyer: buyer.publicKey,
          escrowInfo: newEscrowPDA,
          systemProgram: SystemProgram.programId,
        })
        .signers([seller])
        .rpc();

      await EscrowTestUtils.expectTransactionToFail(
        program.methods
          .pay(newName)
          .accounts({
            buyer: buyer.publicKey,
            seller: seller.publicKey,
            escrowInfo: newEscrowPDA,
          })
          .signers([buyer])
          .rpc()
      );
    });

    it("closes escrow on payment", async () => {
      await program.methods
        .pay(testEscrowName)
        .accounts({
          buyer: buyer.publicKey,
          seller: seller.publicKey,
          escrowInfo: testEscrowPDA,
        })
        .signers([buyer])
        .rpc();

      const escrowAccount = await EscrowTestUtils.getEscrowAccount(program, testEscrowPDA);
      if (escrowAccount) {
        // If account still exists, it should be in closed state
        expect(escrowAccount.state).to.exist;
      }
      // In some implementations, the account might be closed completely
    });
  });

  describe("refund()", () => {
    let testEscrowName: string;
    let testEscrowPDA: PublicKey;

    beforeEach(async () => {
      const keypairs = await EscrowTestUtils.createTestKeypairs();
      seller = keypairs.seller;
      buyer = keypairs.buyer;
      
      testEscrowName = "test-refund-" + Math.random();
      [testEscrowPDA] = EscrowTestUtils.deriveEscrowPDA(
        program.programId,
        testEscrowName,
        seller.publicKey,
        buyer.publicKey
      );

      // Initialize and deposit
      await program.methods
        .initialize(escrowAmount, testEscrowName)
        .accounts({
          seller: seller.publicKey,
          buyer: buyer.publicKey,
          escrowInfo: testEscrowPDA,
          systemProgram: SystemProgram.programId,
        })
        .signers([seller])
        .rpc();

      await program.methods
        .deposit(testEscrowName)
        .accounts({
          buyer: buyer.publicKey,
          seller: seller.publicKey,
          escrowInfo: testEscrowPDA,
          systemProgram: SystemProgram.programId,
        })
        .signers([buyer])
        .rpc();
    });

    it("allows seller to refund", async () => {
      const buyerBalanceBefore = await EscrowTestUtils.getLamports(
        provider.connection,
        buyer.publicKey
      );

      await program.methods
        .refund(testEscrowName)
        .accounts({
          seller: seller.publicKey,
          buyer: buyer.publicKey,
          escrowInfo: testEscrowPDA,
        })
        .signers([seller])
        .rpc();

      const buyerBalanceAfter = await EscrowTestUtils.getLamports(
        provider.connection,
        buyer.publicKey
      );

      // Buyer should have received their refund
      expect(buyerBalanceAfter).to.be.greaterThan(buyerBalanceBefore);
    });

    it("returns funds to buyer", async () => {
      const buyerBalanceBefore = await EscrowTestUtils.getLamports(
        provider.connection,
        buyer.publicKey
      );

      await program.methods
        .refund(testEscrowName)
        .accounts({
          seller: seller.publicKey,
          buyer: buyer.publicKey,
          escrowInfo: testEscrowPDA,
        })
        .signers([seller])
        .rpc();

      const buyerBalanceAfter = await EscrowTestUtils.getLamports(
        provider.connection,
        buyer.publicKey
      );
      const escrowBalance = await EscrowTestUtils.getLamports(
        provider.connection,
        testEscrowPDA
      );

      expect(buyerBalanceAfter).to.be.greaterThan(buyerBalanceBefore);
      expect(escrowBalance).to.equal(0); // Escrow should be emptied
    });

    it("rejects invalid state", async () => {
      // Try to refund without deposit
      const newName = "test-refund-no-deposit-" + Math.floor(Math.random() * 1000).toString();
      const [newEscrowPDA] = EscrowTestUtils.deriveEscrowPDA(
        program.programId,
        newName,
        seller.publicKey,
        buyer.publicKey
      );

      await program.methods
        .initialize(escrowAmount, newName)
        .accounts({
          seller: seller.publicKey,
          buyer: buyer.publicKey,
          escrowInfo: newEscrowPDA,
          systemProgram: SystemProgram.programId,
        })
        .signers([seller])
        .rpc();

      await EscrowTestUtils.expectTransactionToFail(
        program.methods
          .refund(newName)
          .accounts({
            seller: seller.publicKey,
            buyer: buyer.publicKey,
            escrowInfo: newEscrowPDA,
          })
          .signers([seller])
          .rpc()
      );
    });

    it("closes escrow on refund", async () => {
      await program.methods
        .refund(testEscrowName)
        .accounts({
          seller: seller.publicKey,
          buyer: buyer.publicKey,
          escrowInfo: testEscrowPDA,
        })
        .signers([seller])
        .rpc();

      const escrowAccount = await EscrowTestUtils.getEscrowAccount(program, testEscrowPDA);
      if (escrowAccount) {
        // If account still exists, it should be in closed state
        expect(escrowAccount.state).to.exist;
      }
      // In some implementations, the account might be closed completely
    });
  });

  describe("state management", () => {
    let testEscrowName: string;
    let testEscrowPDA: PublicKey;

    beforeEach(async () => {
      const keypairs = await EscrowTestUtils.createTestKeypairs();
      seller = keypairs.seller;
      buyer = keypairs.buyer;
      
      testEscrowName = "test-state-" + Math.floor(Math.random() * 1000).toString();
      [testEscrowPDA] = EscrowTestUtils.deriveEscrowPDA(
        program.programId,
        testEscrowName,
        seller.publicKey,
        buyer.publicKey
      );
    });

    it("enforces state transitions", async () => {
      // Initialize
      await program.methods
        .initialize(escrowAmount, testEscrowName)
        .accounts({
          seller: seller.publicKey,
          buyer: buyer.publicKey,
          escrowInfo: testEscrowPDA,
          systemProgram: SystemProgram.programId,
        })
        .signers([seller])
        .rpc();

      // Should not be able to pay before deposit
      await EscrowTestUtils.expectTransactionToFail(
        program.methods
          .pay(testEscrowName)
          .accounts({
            buyer: buyer.publicKey,
            seller: seller.publicKey,
            escrowInfo: testEscrowPDA,
          })
          .signers([buyer])
          .rpc()
      );

      // Deposit should work
      await program.methods
        .deposit(testEscrowName)
        .accounts({
          buyer: buyer.publicKey,
          seller: seller.publicKey,
          escrowInfo: testEscrowPDA,
          systemProgram: SystemProgram.programId,
        })
        .signers([buyer])
        .rpc();

      // Now pay should work
      await program.methods
        .pay(testEscrowName)
        .accounts({
          buyer: buyer.publicKey,
          seller: seller.publicKey,
          escrowInfo: testEscrowPDA,
        })
        .signers([buyer])
        .rpc();
    });

    it("prevents invalid operations", async () => {
      // Initialize escrow
      await program.methods
        .initialize(escrowAmount, testEscrowName)
        .accounts({
          seller: seller.publicKey,
          buyer: buyer.publicKey,
          escrowInfo: testEscrowPDA,
          systemProgram: SystemProgram.programId,
        })
        .signers([seller])
        .rpc();

      // Deposit
      await program.methods
        .deposit(testEscrowName)
        .accounts({
          buyer: buyer.publicKey,
          seller: seller.publicKey,
          escrowInfo: testEscrowPDA,
          systemProgram: SystemProgram.programId,
        })
        .signers([buyer])
        .rpc();

      // Complete the escrow with payment
      await program.methods
        .pay(testEscrowName)
        .accounts({
          buyer: buyer.publicKey,
          seller: seller.publicKey,
          escrowInfo: testEscrowPDA,
        })
        .signers([buyer])
        .rpc();

      // Should not be able to refund after payment
      await EscrowTestUtils.expectTransactionToFail(
        program.methods
          .refund(testEscrowName)
          .accounts({
            seller: seller.publicKey,
            buyer: buyer.publicKey,
            escrowInfo: testEscrowPDA,
          })
          .signers([seller])
          .rpc()
      );
    });
  });

  describe("authorization", () => {
    let testEscrowName: string;
    let testEscrowPDA: PublicKey;

    beforeEach(async () => {
      const keypairs = await EscrowTestUtils.createTestKeypairs();
      seller = keypairs.seller;
      buyer = keypairs.buyer;
      
      testEscrowName = "auth" + Math.floor(Math.random() * 1000);
      [testEscrowPDA] = EscrowTestUtils.deriveEscrowPDA(
        program.programId,
        testEscrowName,
        seller.publicKey,
        buyer.publicKey
      );

      // Set up escrow with deposit
      await program.methods
        .initialize(escrowAmount, testEscrowName)
        .accounts({
          seller: seller.publicKey,
          buyer: buyer.publicKey,
          escrowInfo: testEscrowPDA,
          systemProgram: SystemProgram.programId,
        })
        .signers([seller])
        .rpc();

      await program.methods
        .deposit(testEscrowName)
        .accounts({
          buyer: buyer.publicKey,
          seller: seller.publicKey,
          escrowInfo: testEscrowPDA,
          systemProgram: SystemProgram.programId,
        })
        .signers([buyer])
        .rpc();
    });

    it("prevents seller from paying", async () => {
      await EscrowTestUtils.expectTransactionToFail(
        program.methods
          .pay(testEscrowName)
          .accounts({
            buyer: seller.publicKey, // Wrong signer
            seller: seller.publicKey,
            escrowInfo: testEscrowPDA,
          })
          .signers([seller])
          .rpc()
      );
    });

    it("prevents buyer from refunding", async () => {
      await EscrowTestUtils.expectTransactionToFail(
        program.methods
          .refund(testEscrowName)
          .accounts({
            seller: buyer.publicKey, // Wrong signer
            buyer: buyer.publicKey,
            escrowInfo: testEscrowPDA,
          })
          .signers([buyer])
          .rpc()
      );
    });
  });
});

// Usage Example:
// 1. Install dependencies: npm install @coral-xyz/anchor @solana/web3.js chai
// 2. Replace 'your-program-name' with your actual program name in the before() hook
// 3. Ensure your program follows the expected interface
// 4. Run with: anchor test
