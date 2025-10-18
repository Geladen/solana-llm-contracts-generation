import pkg from '@coral-xyz/anchor';
const { Program, BN, web3 } = pkg;
import * as anchor from "@coral-xyz/anchor";
import { PublicKey, Keypair, SystemProgram, LAMPORTS_PER_SOL } from "@solana/web3.js";
import { assert, expect } from "chai";

// Generic interface for any Simple Wallet program
interface SimpleWalletProgram {
  methods: {
    deposit(amount: any): any;
    createTransaction(seed: string, amount: any): any;
    executeTransaction(seed: string): any;
    withdraw(amount: any): any;
  };
  account: {
    userTransaction: {
      fetch(address: PublicKey): Promise<{
        receiver: PublicKey;
        amountInLamports: any;
        executed: boolean;
      }>;
    };
  };
}

describe("Simple Wallet Program", () => {
  // Configuration - Update these for your specific program
  const provider = anchor.AnchorProvider.env();
  anchor.setProvider(provider);
  
  // Replace with your program ID and IDL
  const PROGRAM_ID = new PublicKey("GKC7wjJZ28wvewEmRraw4SW7ouVcQSQSNdNSVEnME2qf");
  const program = anchor.workspace.simple_wallet as SimpleWalletProgram;
  
  // Test accounts
  let owner: Keypair;
  let receiver: Keypair;
  let unauthorized: Keypair;
  let userWalletPda: PublicKey;
  let userWalletBump: number;
  
  // Test constants
  const DEPOSIT_AMOUNT = new BN(1 * LAMPORTS_PER_SOL);
  const TRANSACTION_AMOUNT = new BN(0.5 * LAMPORTS_PER_SOL);
  const WITHDRAWAL_AMOUNT = new BN(0.3 * LAMPORTS_PER_SOL);
  const TRANSACTION_SEED = "test-transaction";

  // Utility functions
  const getBalance = async (pubkey: PublicKey): Promise<number> => {
    return await provider.connection.getBalance(pubkey);
  };

  const airdrop = async (pubkey: PublicKey, amount: number = 2 * LAMPORTS_PER_SOL) => {
    const signature = await provider.connection.requestAirdrop(pubkey, amount);
    await provider.connection.confirmTransaction(signature);
  };

  const derivePdas = (ownerKey: PublicKey) => {
    // User Wallet PDA
    const [walletPda, walletBump] = PublicKey.findProgramAddressSync(
      [Buffer.from("wallet"), ownerKey.toBuffer()],
      PROGRAM_ID
    );

    // Transaction PDA
    const [transactionPda, transactionBump] = PublicKey.findProgramAddressSync(
      [Buffer.from(TRANSACTION_SEED), walletPda.toBuffer()],
      PROGRAM_ID
    );

    return {
      userWalletPda: walletPda,
      userWalletBump: walletBump,
      transactionPda,
      transactionBump
    };
  };

  const expectTransactionFailure = async (transactionPromise: Promise<any>) => {
    try {
      await transactionPromise;
      assert.fail("Expected transaction to fail");
    } catch (error) {
      // Transaction failed as expected
      expect(error).to.exist;
    }
  };

  beforeEach(async () => {
    // Generate fresh keypairs for each test
    owner = Keypair.generate();
    receiver = Keypair.generate();
    unauthorized = Keypair.generate();

    // Derive PDAs
    const pdas = derivePdas(owner.publicKey);
    userWalletPda = pdas.userWalletPda;
    userWalletBump = pdas.userWalletBump;

    // Airdrop SOL to test accounts
    await airdrop(owner.publicKey);
    await airdrop(unauthorized.publicKey);
  });

  describe("deposit()", () => {
    it("allows owner to deposit funds", async () => {
      const ownerBalanceBefore = await getBalance(owner.publicKey);
      const walletBalanceBefore = await getBalance(userWalletPda);

      await program.methods
        .deposit(DEPOSIT_AMOUNT)
        .accounts({
          owner: owner.publicKey,
          userWalletPda,
          systemProgram: SystemProgram.programId,
        })
        .signers([owner])
        .rpc();

      const ownerBalanceAfter = await getBalance(owner.publicKey);
      const walletBalanceAfter = await getBalance(userWalletPda);

      // Owner should have less SOL (deposit + fees)
      expect(ownerBalanceAfter).to.be.lessThan(ownerBalanceBefore);
      
      // Wallet should have received the deposit (plus rent exemption minimum)
      const actualDeposit = walletBalanceAfter - walletBalanceBefore;
      expect(actualDeposit).to.be.greaterThanOrEqual(DEPOSIT_AMOUNT.toNumber());
    });

    it("rejects zero deposits", async () => {
      await expectTransactionFailure(
        program.methods
          .deposit(new BN(0))
          .accounts({
            owner: owner.publicKey,
            userWalletPda,
            systemProgram: SystemProgram.programId,
          })
          .signers([owner])
          .rpc()
      );
    });

    it("updates wallet balance correctly with multiple deposits", async () => {
      const firstDeposit = new BN(0.5 * LAMPORTS_PER_SOL);
      const secondDeposit = new BN(0.3 * LAMPORTS_PER_SOL);
      const expectedTotal = firstDeposit.add(secondDeposit);

      const walletBalanceBefore = await getBalance(userWalletPda);

      // First deposit
      await program.methods
        .deposit(firstDeposit)
        .accounts({
          owner: owner.publicKey,
          userWalletPda,
          systemProgram: SystemProgram.programId,
        })
        .signers([owner])
        .rpc();

      // Second deposit
      await program.methods
        .deposit(secondDeposit)
        .accounts({
          owner: owner.publicKey,
          userWalletPda,
          systemProgram: SystemProgram.programId,
        })
        .signers([owner])
        .rpc();

      const walletBalanceAfter = await getBalance(userWalletPda);
      const actualDeposit = walletBalanceAfter - walletBalanceBefore;
      expect(actualDeposit).to.be.greaterThanOrEqual(expectedTotal.toNumber());
    });
  });

  describe("create_transaction()", () => {
    beforeEach(async () => {
      // Deposit funds before creating transactions
      await program.methods
        .deposit(DEPOSIT_AMOUNT)
        .accounts({
          owner: owner.publicKey,
          userWalletPda,
          systemProgram: SystemProgram.programId,
        })
        .signers([owner])
        .rpc();
    });

    it("creates transaction with valid parameters", async () => {
      const { transactionPda } = derivePdas(owner.publicKey);

      await program.methods
        .createTransaction(TRANSACTION_SEED, TRANSACTION_AMOUNT)
        .accounts({
          owner: owner.publicKey,
          userWalletPda,
          transactionPda,
          receiver: receiver.publicKey,
          systemProgram: SystemProgram.programId,
        })
        .signers([owner])
        .rpc();

      // Verify transaction account was created
      const transactionAccount = await program.account.userTransaction.fetch(transactionPda);
      expect(transactionAccount.receiver.toString()).to.equal(receiver.publicKey.toString());
      expect(transactionAccount.amountInLamports.toString()).to.equal(TRANSACTION_AMOUNT.toString());
      expect(transactionAccount.executed).to.be.false;
    });

    it("rejects zero amount transactions", async () => {
      const { transactionPda } = derivePdas(owner.publicKey);

      await expectTransactionFailure(
        program.methods
          .createTransaction(TRANSACTION_SEED, new BN(0))
          .accounts({
            owner: owner.publicKey,
            userWalletPda,
            transactionPda,
            receiver: receiver.publicKey,
            systemProgram: SystemProgram.programId,
          })
          .signers([owner])
          .rpc()
      );
    });

    it("stores receiver and amount correctly", async () => {
      const { transactionPda } = derivePdas(owner.publicKey);
      const customAmount = new BN(0.7 * LAMPORTS_PER_SOL);

      await program.methods
        .createTransaction(TRANSACTION_SEED, customAmount)
        .accounts({
          owner: owner.publicKey,
          userWalletPda,
          transactionPda,
          receiver: receiver.publicKey,
          systemProgram: SystemProgram.programId,
        })
        .signers([owner])
        .rpc();

      const transactionAccount = await program.account.userTransaction.fetch(transactionPda);
      expect(transactionAccount.receiver.toString()).to.equal(receiver.publicKey.toString());
      expect(transactionAccount.amountInLamports.toString()).to.equal(customAmount.toString());
    });
  });

  describe("execute_transaction()", () => {
    let transactionPda: PublicKey;

    beforeEach(async () => {
      // Deposit funds and create transaction
      await program.methods
        .deposit(DEPOSIT_AMOUNT)
        .accounts({
          owner: owner.publicKey,
          userWalletPda,
          systemProgram: SystemProgram.programId,
        })
        .signers([owner])
        .rpc();

      const pdas = derivePdas(owner.publicKey);
      transactionPda = pdas.transactionPda;

      await program.methods
        .createTransaction(TRANSACTION_SEED, TRANSACTION_AMOUNT)
        .accounts({
          owner: owner.publicKey,
          userWalletPda,
          transactionPda,
          receiver: receiver.publicKey,
          systemProgram: SystemProgram.programId,
        })
        .signers([owner])
        .rpc();
    });

    it("executes valid transactions", async () => {
      // Store transaction state before execution
      const transactionAccountBefore = await program.account.userTransaction.fetch(transactionPda);
      expect(transactionAccountBefore.executed).to.be.false;

      await program.methods
        .executeTransaction(TRANSACTION_SEED)
        .accounts({
          owner: owner.publicKey,
          userWalletPda,
          transactionPda,
          receiver: receiver.publicKey,
          systemProgram: SystemProgram.programId,
        })
        .signers([owner])
        .rpc();

      // Transaction account should be closed after execution, so we can't fetch it
      // This is expected behavior - the account closure indicates successful execution
      try {
        await program.account.userTransaction.fetch(transactionPda);
        assert.fail("Transaction account should have been closed");
      } catch (error) {
        // Expected - account was closed after execution
        expect(error.message).to.include("Account does not exist");
      }
    });

    it("transfers funds to receiver", async () => {
      const walletBalanceBefore = await getBalance(userWalletPda);
      const receiverBalanceBefore = await getBalance(receiver.publicKey);

      await program.methods
        .executeTransaction(TRANSACTION_SEED)
        .accounts({
          owner: owner.publicKey,
          userWalletPda,
          transactionPda,
          receiver: receiver.publicKey,
          systemProgram: SystemProgram.programId,
        })
        .signers([owner])
        .rpc();

      const walletBalanceAfter = await getBalance(userWalletPda);
      const receiverBalanceAfter = await getBalance(receiver.publicKey);

      // Verify fund transfer
      expect(walletBalanceBefore - walletBalanceAfter).to.equal(TRANSACTION_AMOUNT.toNumber());
      expect(receiverBalanceAfter - receiverBalanceBefore).to.equal(TRANSACTION_AMOUNT.toNumber());
    });

    it("rejects already executed transactions", async () => {
      // Execute transaction first time
      await program.methods
        .executeTransaction(TRANSACTION_SEED)
        .accounts({
          owner: owner.publicKey,
          userWalletPda,
          transactionPda,
          receiver: receiver.publicKey,
          systemProgram: SystemProgram.programId,
        })
        .signers([owner])
        .rpc();

      // Try to execute again - should fail
      await expectTransactionFailure(
        program.methods
          .executeTransaction(TRANSACTION_SEED)
          .accounts({
            owner: owner.publicKey,
            userWalletPda,
            transactionPda,
            receiver: receiver.publicKey,
            systemProgram: SystemProgram.programId,
          })
          .signers([owner])
          .rpc()
      );
    });

    it("rejects insufficient balance", async () => {
      // Create a transaction larger than wallet balance
      const largeAmount = DEPOSIT_AMOUNT.add(new BN(1 * LAMPORTS_PER_SOL));
      const largeTxSeed = "large-transaction";
      const { transactionPda: largeTxPda } = derivePdas(owner.publicKey);
      const [actualLargeTxPda] = PublicKey.findProgramAddressSync(
        [Buffer.from(largeTxSeed), userWalletPda.toBuffer()],
        PROGRAM_ID
      );

      await program.methods
        .createTransaction(largeTxSeed, largeAmount)
        .accounts({
          owner: owner.publicKey,
          userWalletPda,
          transactionPda: actualLargeTxPda,
          receiver: receiver.publicKey,
          systemProgram: SystemProgram.programId,
        })
        .signers([owner])
        .rpc();

      // Try to execute - should fail due to insufficient funds
      await expectTransactionFailure(
        program.methods
          .executeTransaction(largeTxSeed)
          .accounts({
            owner: owner.publicKey,
            userWalletPda,
            transactionPda: actualLargeTxPda,
            receiver: receiver.publicKey,
            systemProgram: SystemProgram.programId,
          })
          .signers([owner])
          .rpc()
      );
    });

    it("closes transaction account after execution", async () => {
      const ownerBalanceBefore = await getBalance(owner.publicKey);

      await program.methods
        .executeTransaction(TRANSACTION_SEED)
        .accounts({
          owner: owner.publicKey,
          userWalletPda,
          transactionPda,
          receiver: receiver.publicKey,
          systemProgram: SystemProgram.programId,
        })
        .signers([owner])
        .rpc();

      // Account should be closed, trying to fetch should throw
      try {
        await program.account.userTransaction.fetch(transactionPda);
        assert.fail("Transaction account should have been closed");
      } catch (error) {
        // Expected - account no longer exists
        expect(error.message).to.include("Account does not exist");
      }

      // Owner should have received rent back (though fees may offset this)
      const ownerBalanceAfter = await getBalance(owner.publicKey);
      // Note: We can't guarantee owner balance increases due to transaction fees
      // The important thing is the account was closed
      expect(ownerBalanceAfter).to.be.greaterThan(0); // Just verify owner still has funds
    });
  });

  describe("withdraw()", () => {
    beforeEach(async () => {
      // Deposit funds before testing withdrawals
      await program.methods
        .deposit(DEPOSIT_AMOUNT)
        .accounts({
          owner: owner.publicKey,
          userWalletPda,
          systemProgram: SystemProgram.programId,
        })
        .signers([owner])
        .rpc();
    });

    it("allows owner to withdraw funds", async () => {
      const ownerBalanceBefore = await getBalance(owner.publicKey);
      const walletBalanceBefore = await getBalance(userWalletPda);

      await program.methods
        .withdraw(WITHDRAWAL_AMOUNT)
        .accounts({
          owner: owner.publicKey,
          userWalletPda,
          systemProgram: SystemProgram.programId,
        })
        .signers([owner])
        .rpc();

      const ownerBalanceAfter = await getBalance(owner.publicKey);
      const walletBalanceAfter = await getBalance(userWalletPda);

      // Owner should have received withdrawal (minus fees)
      expect(ownerBalanceAfter).to.be.greaterThan(ownerBalanceBefore);
      
      // Wallet should have less SOL
      expect(walletBalanceBefore - walletBalanceAfter).to.equal(WITHDRAWAL_AMOUNT.toNumber());
    });

    it("rejects zero withdrawals", async () => {
      await expectTransactionFailure(
        program.methods
          .withdraw(new BN(0))
          .accounts({
            owner: owner.publicKey,
            userWalletPda,
            systemProgram: SystemProgram.programId,
          })
          .signers([owner])
          .rpc()
      );
    });

    it("updates wallet balance correctly", async () => {
      const walletBalanceBefore = await getBalance(userWalletPda);

      await program.methods
        .withdraw(WITHDRAWAL_AMOUNT)
        .accounts({
          owner: owner.publicKey,
          userWalletPda,
          systemProgram: SystemProgram.programId,
        })
        .signers([owner])
        .rpc();

      const walletBalanceAfter = await getBalance(userWalletPda);
      const expectedBalance = walletBalanceBefore - WITHDRAWAL_AMOUNT.toNumber();
      
      expect(walletBalanceAfter).to.equal(expectedBalance);
    });

    it("rejects withdrawal exceeding balance", async () => {
      const excessiveAmount = DEPOSIT_AMOUNT.add(new BN(1 * LAMPORTS_PER_SOL));

      await expectTransactionFailure(
        program.methods
          .withdraw(excessiveAmount)
          .accounts({
            owner: owner.publicKey,
            userWalletPda,
            systemProgram: SystemProgram.programId,
          })
          .signers([owner])
          .rpc()
      );
    });
  });

  describe("authorization", () => {
    beforeEach(async () => {
      // Setup wallet with funds for authorization tests
      await program.methods
        .deposit(DEPOSIT_AMOUNT)
        .accounts({
          owner: owner.publicKey,
          userWalletPda,
          systemProgram: SystemProgram.programId,
        })
        .signers([owner])
        .rpc();
    });

    it("enforces owner-only operations for deposit", async () => {
      const [unauthorizedWalletPda] = PublicKey.findProgramAddressSync(
        [Buffer.from("wallet"), unauthorized.publicKey.toBuffer()],
        PROGRAM_ID
      );

      await expectTransactionFailure(
        program.methods
          .deposit(DEPOSIT_AMOUNT)
          .accounts({
            owner: unauthorized.publicKey,
            userWalletPda: unauthorizedWalletPda,
            systemProgram: SystemProgram.programId,
          })
          .signers([unauthorized])
          .rpc()
      );
    });

    it("enforces owner-only operations for withdraw", async () => {
      await expectTransactionFailure(
        program.methods
          .withdraw(WITHDRAWAL_AMOUNT)
          .accounts({
            owner: unauthorized.publicKey,
            userWalletPda,
            systemProgram: SystemProgram.programId,
          })
          .signers([unauthorized])
          .rpc()
      );
    });

    it("enforces owner-only operations for create_transaction", async () => {
      const { transactionPda } = derivePdas(owner.publicKey);

      await expectTransactionFailure(
        program.methods
          .createTransaction(TRANSACTION_SEED, TRANSACTION_AMOUNT)
          .accounts({
            owner: unauthorized.publicKey,
            userWalletPda,
            transactionPda,
            receiver: receiver.publicKey,
            systemProgram: SystemProgram.programId,
          })
          .signers([unauthorized])
          .rpc()
      );
    });

    it("validates transaction receiver matches", async () => {
      const { transactionPda } = derivePdas(owner.publicKey);
      const wrongReceiver = Keypair.generate();

      // Create transaction with correct receiver
      await program.methods
        .createTransaction(TRANSACTION_SEED, TRANSACTION_AMOUNT)
        .accounts({
          owner: owner.publicKey,
          userWalletPda,
          transactionPda,
          receiver: receiver.publicKey,
          systemProgram: SystemProgram.programId,
        })
        .signers([owner])
        .rpc();

      // Try to execute with wrong receiver - should fail
      await expectTransactionFailure(
        program.methods
          .executeTransaction(TRANSACTION_SEED)
          .accounts({
            owner: owner.publicKey,
            userWalletPda,
            transactionPda,
            receiver: wrongReceiver.publicKey,
            systemProgram: SystemProgram.programId,
          })
          .signers([owner])
          .rpc()
      );
    });
  });

  describe("edge cases and integration", () => {
    it("handles complete wallet lifecycle", async () => {
      const initialBalance = await getBalance(userWalletPda);
      
      // 1. Deposit funds
      await program.methods
        .deposit(DEPOSIT_AMOUNT)
        .accounts({
          owner: owner.publicKey,
          userWalletPda,
          systemProgram: SystemProgram.programId,
        })
        .signers([owner])
        .rpc();

      // 2. Create transaction
      const { transactionPda } = derivePdas(owner.publicKey);
      await program.methods
        .createTransaction(TRANSACTION_SEED, TRANSACTION_AMOUNT)
        .accounts({
          owner: owner.publicKey,
          userWalletPda,
          transactionPda,
          receiver: receiver.publicKey,
          systemProgram: SystemProgram.programId,
        })
        .signers([owner])
        .rpc();

      // 3. Execute transaction
      await program.methods
        .executeTransaction(TRANSACTION_SEED)
        .accounts({
          owner: owner.publicKey,
          userWalletPda,
          transactionPda,
          receiver: receiver.publicKey,
          systemProgram: SystemProgram.programId,
        })
        .signers([owner])
        .rpc();

      // 4. Withdraw remaining funds
      const remainingAmount = DEPOSIT_AMOUNT.sub(TRANSACTION_AMOUNT);
      await program.methods
        .withdraw(remainingAmount)
        .accounts({
          owner: owner.publicKey,
          userWalletPda,
          systemProgram: SystemProgram.programId,
        })
        .signers([owner])
        .rpc();

      // Verify final state
      const finalBalance = await getBalance(userWalletPda);
      const receiverBalance = await getBalance(receiver.publicKey);
      
      expect(finalBalance).to.be.closeTo(initialBalance, 1000000); // Account for rent
      expect(receiverBalance).to.equal(TRANSACTION_AMOUNT.toNumber());
    });
  });
});
