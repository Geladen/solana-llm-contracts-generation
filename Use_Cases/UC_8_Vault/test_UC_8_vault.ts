import * as anchor from "@coral-xyz/anchor";
import { Program, AnchorProvider, Wallet } from "@coral-xyz/anchor";
import { 
  PublicKey, 
  Keypair, 
  SystemProgram, 
  LAMPORTS_PER_SOL,
  Connection,
  Transaction
} from "@solana/web3.js";
import { BN } from "bn.js";
import { assert, expect } from "chai";

// Generic vault program interface - adapt to your specific program
interface VaultProgram {
  methods: {
    initialize(waitTime: BN, initialAmount: BN): any;
    withdraw(amount: BN): any;
    finalize(): any;
    cancel(): any;
  };
  account: {
    vaultInfo: {
      fetch(address: PublicKey): Promise<any>;
    };
  };
}

describe("Vault Program", () => {
  // Test configuration
  const provider = AnchorProvider.env();
  anchor.setProvider(provider);
  
  // Load your program - replace with actual program loading
  const program = anchor.workspace.vault as Program<VaultProgram>;
  
  // Test accounts
  let owner: Keypair;
  let recovery: Keypair;
  let receiver: Keypair;
  let vaultInfo: PublicKey;
  let vaultBump: number;
  
  // Test constants
  const INITIAL_AMOUNT = new BN(LAMPORTS_PER_SOL);
  const WAIT_TIME = new BN(10); // 10 slots
  const WITHDRAWAL_AMOUNT = new BN(0.5 * LAMPORTS_PER_SOL);

  beforeEach(async () => {
    // Generate fresh keypairs for each test
    owner = Keypair.generate();
    recovery = Keypair.generate();
    receiver = Keypair.generate();

    // Derive PDA for vault info
    [vaultInfo, vaultBump] = PublicKey.findProgramAddressSync(
      [owner.publicKey.toBuffer()],
      program.programId
    );

    // Airdrop SOL to test accounts
    await Promise.all([
      provider.connection.requestAirdrop(owner.publicKey, 2 * LAMPORTS_PER_SOL),
      provider.connection.requestAirdrop(recovery.publicKey, LAMPORTS_PER_SOL),
      provider.connection.requestAirdrop(receiver.publicKey, LAMPORTS_PER_SOL),
    ]);

    // Wait for airdrops to confirm
    await new Promise(resolve => setTimeout(resolve, 1000));
  });

  // Helper functions
  const getAccountBalance = async (publicKey: PublicKey): Promise<number> => {
    return await provider.connection.getBalance(publicKey);
  };

  const expectTransactionToFail = async (transactionPromise: Promise<any>) => {
    try {
      await transactionPromise;
      assert.fail("Expected transaction to fail but it succeeded");
    } catch (error) {
      // Transaction failed as expected
      expect(error).to.exist;
    }
  };

  const initializeVault = async (
    waitTime: BN = WAIT_TIME,
    initialAmount: BN = INITIAL_AMOUNT
  ) => {
    await program.methods
      .initialize(waitTime, initialAmount)
      .accounts({
        owner: owner.publicKey,
        recovery: recovery.publicKey,
        vaultInfo,
        systemProgram: SystemProgram.programId,
      })
      .signers([owner])
      .rpc();
  };

  describe("initialize()", () => {
    it("creates vault with valid parameters", async () => {
      const initialBalance = await getAccountBalance(owner.publicKey);
      
      await initializeVault();
      
      // Verify vault account was created
      const vaultAccount = await program.account.vaultInfo.fetch(vaultInfo);
      expect(vaultAccount.owner.toString()).to.equal(owner.publicKey.toString());
      expect(vaultAccount.recovery.toString()).to.equal(recovery.publicKey.toString());
      expect(vaultAccount.waitTime.toString()).to.equal(WAIT_TIME.toString());
      
      // Verify lamports were transferred
      const finalBalance = await getAccountBalance(owner.publicKey);
      const vaultBalance = await getAccountBalance(vaultInfo);
      
      expect(vaultBalance).to.be.greaterThan(INITIAL_AMOUNT.toNumber());
      expect(initialBalance - finalBalance).to.be.greaterThan(INITIAL_AMOUNT.toNumber());
    });

    it("rejects zero wait time", async () => {
      await expectTransactionToFail(
        initializeVault(new BN(0), INITIAL_AMOUNT)
      );
    });

    it("transfers initial funds", async () => {
      const ownerBalanceBefore = await getAccountBalance(owner.publicKey);
      
      await initializeVault();
      
      const ownerBalanceAfter = await getAccountBalance(owner.publicKey);
      const vaultBalance = await getAccountBalance(vaultInfo);
      
      // Owner balance should decrease by more than initial amount (including rent)
      expect(ownerBalanceBefore - ownerBalanceAfter).to.be.greaterThan(INITIAL_AMOUNT.toNumber());
      
      // Vault should have received the funds
      expect(vaultBalance).to.be.greaterThan(INITIAL_AMOUNT.toNumber());
    });

    it("prevents double initialization", async () => {
      await initializeVault();
      
      await expectTransactionToFail(
        initializeVault()
      );
    });
  });

  describe("withdraw()", () => {
    beforeEach(async () => {
      await initializeVault();
    });

    it("allows owner to request withdrawal", async () => {
      await program.methods
        .withdraw(WITHDRAWAL_AMOUNT)
        .accounts({
          owner: owner.publicKey,
          receiver: receiver.publicKey,
          vaultInfo,
        })
        .signers([owner])
        .rpc();

      const vaultAccount = await program.account.vaultInfo.fetch(vaultInfo);
      expect(vaultAccount.amount.toString()).to.equal(WITHDRAWAL_AMOUNT.toString());
      expect(vaultAccount.receiver.toString()).to.equal(receiver.publicKey.toString());
    });

    it("sets correct wait period", async () => {
      const slot = await provider.connection.getSlot();
      
      await program.methods
        .withdraw(WITHDRAWAL_AMOUNT)
        .accounts({
          owner: owner.publicKey,
          receiver: receiver.publicKey,
          vaultInfo,
        })
        .signers([owner])
        .rpc();

      const vaultAccount = await program.account.vaultInfo.fetch(vaultInfo);
      expect(vaultAccount.requestTime.toNumber()).to.be.greaterThanOrEqual(slot);
    });

    it("rejects zero amount", async () => {
      await expectTransactionToFail(
        program.methods
          .withdraw(new BN(0))
          .accounts({
            owner: owner.publicKey,
            receiver: receiver.publicKey,
            vaultInfo,
          })
          .signers([owner])
          .rpc()
      );
    });

    it("preserves rent exemption", async () => {
      const vaultBalance = await getAccountBalance(vaultInfo);
      const excessiveAmount = new BN(vaultBalance - 1000); // Leave very little for rent

      await expectTransactionToFail(
        program.methods
          .withdraw(excessiveAmount)
          .accounts({
            owner: owner.publicKey,
            receiver: receiver.publicKey,
            vaultInfo,
          })
          .signers([owner])
          .rpc()
      );
    });

    it("rejects non-owner withdrawal request", async () => {
      const nonOwner = Keypair.generate();
      await provider.connection.requestAirdrop(nonOwner.publicKey, LAMPORTS_PER_SOL);
      await new Promise(resolve => setTimeout(resolve, 1000));

      await expectTransactionToFail(
        program.methods
          .withdraw(WITHDRAWAL_AMOUNT)
          .accounts({
            owner: nonOwner.publicKey,
            receiver: receiver.publicKey,
            vaultInfo,
          })
          .signers([nonOwner])
          .rpc()
      );
    });

    it("prevents multiple concurrent withdrawal requests", async () => {
      // First withdrawal request
      await program.methods
        .withdraw(WITHDRAWAL_AMOUNT)
        .accounts({
          owner: owner.publicKey,
          receiver: receiver.publicKey,
          vaultInfo,
        })
        .signers([owner])
        .rpc();

      // Second withdrawal request should fail
      await expectTransactionToFail(
        program.methods
          .withdraw(WITHDRAWAL_AMOUNT)
          .accounts({
            owner: owner.publicKey,
            receiver: receiver.publicKey,
            vaultInfo,
          })
          .signers([owner])
          .rpc()
      );
    });
  });

  describe("finalize()", () => {
    beforeEach(async () => {
      await initializeVault();
      await program.methods
        .withdraw(WITHDRAWAL_AMOUNT)
        .accounts({
          owner: owner.publicKey,
          receiver: receiver.publicKey,
          vaultInfo,
        })
        .signers([owner])
        .rpc();
    });

    it("rejects premature finalization", async () => {
      await expectTransactionToFail(
        program.methods
          .finalize()
          .accounts({
            owner: owner.publicKey,
            receiver: receiver.publicKey,
            vaultInfo,
          })
          .signers([owner])
          .rpc()
      );
    });

    it("completes withdrawal after wait time", async () => {
      // Wait for the wait period to pass
      const vaultAccount = await program.account.vaultInfo.fetch(vaultInfo);
      const targetSlot = vaultAccount.requestTime.toNumber() + vaultAccount.waitTime.toNumber();
      
      // Wait until target slot is reached
      let currentSlot = await provider.connection.getSlot();
      while (currentSlot < targetSlot) {
        await new Promise(resolve => setTimeout(resolve, 400)); // Wait ~400ms per slot
        currentSlot = await provider.connection.getSlot();
      }

      const receiverBalanceBefore = await getAccountBalance(receiver.publicKey);
      const vaultBalanceBefore = await getAccountBalance(vaultInfo);

      await program.methods
        .finalize()
        .accounts({
          owner: owner.publicKey,
          receiver: receiver.publicKey,
          vaultInfo,
        })
        .signers([owner])
        .rpc();

      const receiverBalanceAfter = await getAccountBalance(receiver.publicKey);
      const vaultBalanceAfter = await getAccountBalance(vaultInfo);

      // Verify funds transfer
      expect(receiverBalanceAfter - receiverBalanceBefore).to.equal(WITHDRAWAL_AMOUNT.toNumber());
      expect(vaultBalanceBefore - vaultBalanceAfter).to.equal(WITHDRAWAL_AMOUNT.toNumber());
    });

    it("resets vault state", async () => {
      // Wait for the wait period to pass
      const vaultAccount = await program.account.vaultInfo.fetch(vaultInfo);
      const targetSlot = vaultAccount.requestTime.toNumber() + vaultAccount.waitTime.toNumber();
      
      let currentSlot = await provider.connection.getSlot();
      while (currentSlot < targetSlot) {
        await new Promise(resolve => setTimeout(resolve, 400));
        currentSlot = await provider.connection.getSlot();
      }

      await program.methods
        .finalize()
        .accounts({
          owner: owner.publicKey,
          receiver: receiver.publicKey,
          vaultInfo,
        })
        .signers([owner])
        .rpc();

      const finalVaultAccount = await program.account.vaultInfo.fetch(vaultInfo);
      
      // State should be reset (assuming Idle = 0)
      expect(finalVaultAccount.state).to.deep.equal({ idle: {} });
    });

    it("rejects finalization with wrong receiver", async () => {
      const wrongReceiver = Keypair.generate();
      await provider.connection.requestAirdrop(wrongReceiver.publicKey, LAMPORTS_PER_SOL);
      await new Promise(resolve => setTimeout(resolve, 1000));

      // Wait for the wait period to pass
      const vaultAccount = await program.account.vaultInfo.fetch(vaultInfo);
      const targetSlot = vaultAccount.requestTime.toNumber() + vaultAccount.waitTime.toNumber();
      
      let currentSlot = await provider.connection.getSlot();
      while (currentSlot < targetSlot) {
        await new Promise(resolve => setTimeout(resolve, 400));
        currentSlot = await provider.connection.getSlot();
      }

      await expectTransactionToFail(
        program.methods
          .finalize()
          .accounts({
            owner: owner.publicKey,
            receiver: wrongReceiver.publicKey,
            vaultInfo,
          })
          .signers([owner])
          .rpc()
      );
    });
  });

  describe("cancel()", () => {
    beforeEach(async () => {
      await initializeVault();
      await program.methods
        .withdraw(WITHDRAWAL_AMOUNT)
        .accounts({
          owner: owner.publicKey,
          receiver: receiver.publicKey,
          vaultInfo,
        })
        .signers([owner])
        .rpc();
    });

    it("allows recovery to cancel request", async () => {
      await program.methods
        .cancel()
        .accounts({
          recovery: recovery.publicKey,
          owner: owner.publicKey,
          vaultInfo,
        })
        .signers([recovery])
        .rpc();

      const vaultAccount = await program.account.vaultInfo.fetch(vaultInfo);
      expect(vaultAccount.state).to.deep.equal({ idle: {} });
    });

    it("rejects non-recovery cancellation", async () => {
      const nonRecovery = Keypair.generate();
      await provider.connection.requestAirdrop(nonRecovery.publicKey, LAMPORTS_PER_SOL);
      await new Promise(resolve => setTimeout(resolve, 1000));

      await expectTransactionToFail(
        program.methods
          .cancel()
          .accounts({
            recovery: nonRecovery.publicKey,
            owner: owner.publicKey,
            vaultInfo,
          })
          .signers([nonRecovery])
          .rpc()
      );
    });

    it("rejects owner cancellation", async () => {
      await expectTransactionToFail(
        program.methods
          .cancel()
          .accounts({
            recovery: owner.publicKey, // Owner trying to cancel
            owner: owner.publicKey,
            vaultInfo,
          })
          .signers([owner])
          .rpc()
      );
    });

    it("resets withdrawal state", async () => {
      await program.methods
        .cancel()
        .accounts({
          recovery: recovery.publicKey,
          owner: owner.publicKey,
          vaultInfo,
        })
        .signers([recovery])
        .rpc();

      const vaultAccount = await program.account.vaultInfo.fetch(vaultInfo);
      expect(vaultAccount.state).to.deep.equal({ idle: {} });

      // Should be able to make a new withdrawal request
      await program.methods
        .withdraw(WITHDRAWAL_AMOUNT)
        .accounts({
          owner: owner.publicKey,
          receiver: receiver.publicKey,
          vaultInfo,
        })
        .signers([owner])
        .rpc();
    });

    it("rejects cancellation when no request pending", async () => {
      // Cancel first request
      await program.methods
        .cancel()
        .accounts({
          recovery: recovery.publicKey,
          owner: owner.publicKey,
          vaultInfo,
        })
        .signers([recovery])
        .rpc();

      // Try to cancel again when no request is pending
      await expectTransactionToFail(
        program.methods
          .cancel()
          .accounts({
            recovery: recovery.publicKey,
            owner: owner.publicKey,
            vaultInfo,
          })
          .signers([recovery])
          .rpc()
      );
    });
  });

  describe("time validation", () => {
    beforeEach(async () => {
      await initializeVault();
    });

    it("enforces wait period", async () => {
      await program.methods
        .withdraw(WITHDRAWAL_AMOUNT)
        .accounts({
          owner: owner.publicKey,
          receiver: receiver.publicKey,
          vaultInfo,
        })
        .signers([owner])
        .rpc();

      // Try to finalize immediately - should fail
      await expectTransactionToFail(
        program.methods
          .finalize()
          .accounts({
            owner: owner.publicKey,
            receiver: receiver.publicKey,
            vaultInfo,
          })
          .signers([owner])
          .rpc()
      );
    });

    it("prevents early finalization", async () => {
      await program.methods
        .withdraw(WITHDRAWAL_AMOUNT)
        .accounts({
          owner: owner.publicKey,
          receiver: receiver.publicKey,
          vaultInfo,
        })
        .signers([owner])
        .rpc();

      const vaultAccount = await program.account.vaultInfo.fetch(vaultInfo);
      const currentSlot = await provider.connection.getSlot();
      const requiredSlot = vaultAccount.requestTime.toNumber() + vaultAccount.waitTime.toNumber();

      expect(currentSlot).to.be.lessThan(requiredSlot);

      // Should fail until wait time passes
      await expectTransactionToFail(
        program.methods
          .finalize()
          .accounts({
            owner: owner.publicKey,
            receiver: receiver.publicKey,
            vaultInfo,
          })
          .signers([owner])
          .rpc()
      );
    });
  });

  describe("integration scenarios", () => {
    it("handles full vault lifecycle", async () => {
      // Initialize
      await initializeVault();
      
      // Request withdrawal
      await program.methods
        .withdraw(WITHDRAWAL_AMOUNT)
        .accounts({
          owner: owner.publicKey,
          receiver: receiver.publicKey,
          vaultInfo,
        })
        .signers([owner])
        .rpc();

      // Cancel and request again
      await program.methods
        .cancel()
        .accounts({
          recovery: recovery.publicKey,
          owner: owner.publicKey,
          vaultInfo,
        })
        .signers([recovery])
        .rpc();

      // New withdrawal request
      await program.methods
        .withdraw(WITHDRAWAL_AMOUNT)
        .accounts({
          owner: owner.publicKey,
          receiver: receiver.publicKey,
          vaultInfo,
        })
        .signers([owner])
        .rpc();

      // Wait and finalize
      const vaultAccount = await program.account.vaultInfo.fetch(vaultInfo);
      const targetSlot = vaultAccount.requestTime.toNumber() + vaultAccount.waitTime.toNumber();
      
      let currentSlot = await provider.connection.getSlot();
      while (currentSlot < targetSlot) {
        await new Promise(resolve => setTimeout(resolve, 400));
        currentSlot = await provider.connection.getSlot();
      }

      const receiverBalanceBefore = await getAccountBalance(receiver.publicKey);

      await program.methods
        .finalize()
        .accounts({
          owner: owner.publicKey,
          receiver: receiver.publicKey,
          vaultInfo,
        })
        .signers([owner])
        .rpc();

      const receiverBalanceAfter = await getAccountBalance(receiver.publicKey);
      expect(receiverBalanceAfter - receiverBalanceBefore).to.equal(WITHDRAWAL_AMOUNT.toNumber());
    });

    it("handles multiple withdrawal cycles", async () => {
      await initializeVault();

      // First withdrawal cycle
      await program.methods
        .withdraw(WITHDRAWAL_AMOUNT)
        .accounts({
          owner: owner.publicKey,
          receiver: receiver.publicKey,
          vaultInfo,
        })
        .signers([owner])
        .rpc();

      // Wait and finalize first
      let vaultAccount = await program.account.vaultInfo.fetch(vaultInfo);
      let targetSlot = vaultAccount.requestTime.toNumber() + vaultAccount.waitTime.toNumber();
      
      let currentSlot = await provider.connection.getSlot();
      while (currentSlot < targetSlot) {
        await new Promise(resolve => setTimeout(resolve, 400));
        currentSlot = await provider.connection.getSlot();
      }

      await program.methods
        .finalize()
        .accounts({
          owner: owner.publicKey,
          receiver: receiver.publicKey,
          vaultInfo,
        })
        .signers([owner])
        .rpc();

      // Second withdrawal cycle
      const secondWithdrawal = new BN(0.1 * LAMPORTS_PER_SOL);
      await program.methods
        .withdraw(secondWithdrawal)
        .accounts({
          owner: owner.publicKey,
          receiver: receiver.publicKey,
          vaultInfo,
        })
        .signers([owner])
        .rpc();

      // Verify second withdrawal can be made
      vaultAccount = await program.account.vaultInfo.fetch(vaultInfo);
      expect(vaultAccount.amount.toString()).to.equal(secondWithdrawal.toString());
    });
  });
});
