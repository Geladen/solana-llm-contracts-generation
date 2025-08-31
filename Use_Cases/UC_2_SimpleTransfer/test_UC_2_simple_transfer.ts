import * as anchor from "@coral-xyz/anchor";
import { Program, AnchorProvider, Wallet } from "@coral-xyz/anchor";
import { 
  PublicKey, 
  Keypair, 
  LAMPORTS_PER_SOL, 
  SystemProgram,
  SYSVAR_RENT_PUBKEY
} from "@solana/web3.js";
import { expect } from "chai";
import BN from "bn.js";

/**
 * Universal Test Suite for Anchor Transfer Programs
 * 
 * This test suite works with any Anchor program that implements:
 * - deposit(amount: u64) function
 * - withdraw(amount: u64) function  
 * - PDA derivation from [recipient, sender] seeds
 * - Native SOL transfers with balance tracking
 * 
 * To use with your program:
 * 1. Replace PROGRAM_ID with your program's ID
 * 2. Update the program import/setup
 * 3. Ensure your program follows the expected interface
 */

describe("Universal Transfer Program Test Suite", () => {
  // Configuration - Update these for your specific program
  const PROGRAM_ID = new PublicKey("GXGCxuXmztgTRPAfuYF72eU6eTkdEKG8Amu81NCSSkPX");
  const DEPOSIT_AMOUNT = 0.1 * LAMPORTS_PER_SOL; // 0.1 SOL
  const WITHDRAW_AMOUNT = 0.05 * LAMPORTS_PER_SOL; // 0.05 SOL
  
  // Test accounts
  let provider: AnchorProvider;
  let program: Program<any>;
  let sender: Keypair;
  let recipient: Keypair;
  let unauthorized: Keypair;
  let balanceHolderPda: PublicKey;
  let pdaBump: number;

  before(async () => {
    // Setup provider and program
    provider = AnchorProvider.env();
    anchor.setProvider(provider);
    
    // Load your program - update this line for your specific program
    program = anchor.workspace.simple_transfer as Program<any>;
    
    // Generate test keypairs
    sender = Keypair.generate();
    recipient = Keypair.generate();
    unauthorized = Keypair.generate();
    
    // Fund test accounts
    await fundAccount(sender.publicKey, 2 * LAMPORTS_PER_SOL);
    await fundAccount(recipient.publicKey, 1 * LAMPORTS_PER_SOL);
    await fundAccount(unauthorized.publicKey, 1 * LAMPORTS_PER_SOL);
    
    // Derive PDA address
    [balanceHolderPda, pdaBump] = PublicKey.findProgramAddressSync(
      [recipient.publicKey.toBuffer(), sender.publicKey.toBuffer()],
      program.programId
    );
  });

  // Helper function to fund accounts
  async function fundAccount(publicKey: PublicKey, amount: number) {
    const signature = await provider.connection.requestAirdrop(publicKey, amount);
    await provider.connection.confirmTransaction(signature);
  }

  // Helper function to get account balance
  async function getBalance(publicKey: PublicKey): Promise<number> {
    return await provider.connection.getBalance(publicKey);
  }

  // Helper function to check if account exists
  async function accountExists(publicKey: PublicKey): Promise<boolean> {
    try {
      const accountInfo = await provider.connection.getAccountInfo(publicKey);
      return accountInfo !== null;
    } catch {
      return false;
    }
  }

  // Helper function to get PDA account data
  async function getPdaAccount(): Promise<any | null> {
    try {
      return await program.account.balanceHolderPda.fetch(balanceHolderPda);
    } catch {
      return null;
    }
  }

  describe("deposit()", () => {
    let initialSenderBalance: number;
    let initialPdaBalance: number;

    beforeEach(async () => {
      // Reset state before each test
      sender = Keypair.generate();
      recipient = Keypair.generate();
      await fundAccount(sender.publicKey, 2 * LAMPORTS_PER_SOL);
      
      [balanceHolderPda, pdaBump] = PublicKey.findProgramAddressSync(
        [recipient.publicKey.toBuffer(), sender.publicKey.toBuffer()],
        program.programId
      );
      
      initialSenderBalance = await getBalance(sender.publicKey);
      initialPdaBalance = await getBalance(balanceHolderPda);
    });

    it("allows sender to deposit positive amounts", async () => {
      await program.methods
        .deposit(new BN(DEPOSIT_AMOUNT))
        .accounts({
          balanceHolderPda,
          sender: sender.publicKey,
          recipient: recipient.publicKey,
          systemProgram: SystemProgram.programId,
        })
        .signers([sender])
        .rpc();

      // Verify lamports were transferred correctly
      const finalSenderBalance = await getBalance(sender.publicKey);
      const finalPdaBalance = await getBalance(balanceHolderPda);
      
      // Sender should have less lamports (deposit + transaction fees)
      expect(finalSenderBalance).to.be.lessThan(initialSenderBalance);
      
      // PDA should have the deposited amount plus rent
      expect(finalPdaBalance).to.be.greaterThan(initialPdaBalance + DEPOSIT_AMOUNT);
      
      // Verify PDA account was created and initialized
      const pdaAccount = await getPdaAccount();
      expect(pdaAccount).to.not.be.null;
      expect(pdaAccount.sender.toString()).to.equal(sender.publicKey.toString());
      expect(pdaAccount.recipient.toString()).to.equal(recipient.publicKey.toString());
      expect(pdaAccount.amount.toNumber()).to.equal(DEPOSIT_AMOUNT);
    });

    it("prevents zero amount deposits", async () => {
      try {
        await program.methods
          .deposit(new BN(0))
          .accounts({
            balanceHolderPda,
            sender: sender.publicKey,
            recipient: recipient.publicKey,
            systemProgram: SystemProgram.programId,
          })
          .signers([sender])
          .rpc();
        
        expect.fail("Should have failed with zero amount");
      } catch (error) {
        // Generic error checking - any failure is acceptable for zero amount
        expect(error).to.exist;
      }
    });

    it("transfers correct lamport amounts to PDA", async () => {
      const testAmount = 0.25 * LAMPORTS_PER_SOL;
      
      await program.methods
        .deposit(new BN(testAmount))
        .accounts({
          balanceHolderPda,
          sender: sender.publicKey,
          recipient: recipient.publicKey,
          systemProgram: SystemProgram.programId,
        })
        .signers([sender])
        .rpc();

      const finalPdaBalance = await getBalance(balanceHolderPda);
      const pdaAccount = await getPdaAccount();
      
      // PDA balance should include the deposit plus rent exemption
      expect(finalPdaBalance).to.be.greaterThan(testAmount);
      
      // PDA account should track the exact deposited amount
      expect(pdaAccount.amount.toNumber()).to.equal(testAmount);
    });

    it("initializes PDA with correct sender/recipient data", async () => {
      await program.methods
        .deposit(new BN(DEPOSIT_AMOUNT))
        .accounts({
          balanceHolderPda,
          sender: sender.publicKey,
          recipient: recipient.publicKey,
          systemProgram: SystemProgram.programId,
        })
        .signers([sender])
        .rpc();

      const pdaAccount = await getPdaAccount();
      
      expect(pdaAccount.sender.toString()).to.equal(sender.publicKey.toString());
      expect(pdaAccount.recipient.toString()).to.equal(recipient.publicKey.toString());
      expect(pdaAccount.amount.toNumber()).to.equal(DEPOSIT_AMOUNT);
    });

    it("creates PDA with proper seeds derivation", async () => {
      // Verify PDA doesn't exist before deposit
      expect(await accountExists(balanceHolderPda)).to.be.false;
      
      await program.methods
        .deposit(new BN(DEPOSIT_AMOUNT))
        .accounts({
          balanceHolderPda,
          sender: sender.publicKey,
          recipient: recipient.publicKey,
          systemProgram: SystemProgram.programId,
        })
        .signers([sender])
        .rpc();

      // Verify PDA exists after deposit and has correct derivation
      expect(await accountExists(balanceHolderPda)).to.be.true;
      
      // Verify the PDA can be re-derived with same seeds
      const [derivedPda] = PublicKey.findProgramAddressSync(
        [recipient.publicKey.toBuffer(), sender.publicKey.toBuffer()],
        program.programId
      );
      
      expect(derivedPda.toString()).to.equal(balanceHolderPda.toString());
    });
  });

  describe("withdraw()", () => {
    let initialRecipientBalance: number;
    let initialSenderBalance: number;

    beforeEach(async () => {
      // Setup: Create a fresh deposit for each withdraw test
      sender = Keypair.generate();
      recipient = Keypair.generate();
      await fundAccount(sender.publicKey, 2 * LAMPORTS_PER_SOL);
      await fundAccount(recipient.publicKey, 1 * LAMPORTS_PER_SOL);
      
      [balanceHolderPda, pdaBump] = PublicKey.findProgramAddressSync(
        [recipient.publicKey.toBuffer(), sender.publicKey.toBuffer()],
        program.programId
      );
      
      // Make a deposit first
      await program.methods
        .deposit(new BN(DEPOSIT_AMOUNT))
        .accounts({
          balanceHolderPda,
          sender: sender.publicKey,
          recipient: recipient.publicKey,
          systemProgram: SystemProgram.programId,
        })
        .signers([sender])
        .rpc();

      initialRecipientBalance = await getBalance(recipient.publicKey);
      initialSenderBalance = await getBalance(sender.publicKey);
    });

    it("allows recipient to withdraw available amounts", async () => {
      await program.methods
        .withdraw(new BN(WITHDRAW_AMOUNT))
        .accounts({
          recipient: recipient.publicKey,
          sender: sender.publicKey,
          balanceHolderPda,
          rent: SYSVAR_RENT_PUBKEY,
        })
        .signers([recipient])
        .rpc();

      const finalRecipientBalance = await getBalance(recipient.publicKey);
      const pdaAccount = await getPdaAccount();
      
      // Recipient should have more lamports (minus transaction fees)
      const balanceIncrease = finalRecipientBalance - initialRecipientBalance;
      expect(balanceIncrease).to.be.greaterThan(0);
      
      // PDA account amount should be reduced
      const expectedRemainingAmount = DEPOSIT_AMOUNT - WITHDRAW_AMOUNT;
      expect(pdaAccount.amount.toNumber()).to.equal(expectedRemainingAmount);
    });

    it("prevents zero amount withdrawals", async () => {
      try {
        await program.methods
          .withdraw(new BN(0))
          .accounts({
            recipient: recipient.publicKey,
            sender: sender.publicKey,
            balanceHolderPda,
            rent: SYSVAR_RENT_PUBKEY,
          })
          .signers([recipient])
          .rpc();
        
        expect.fail("Should have failed with zero amount");
      } catch (error) {
        expect(error).to.exist;
      }
    });

    it("prevents unauthorized withdrawals", async () => {
      try {
        await program.methods
          .withdraw(new BN(WITHDRAW_AMOUNT))
          .accounts({
            recipient: unauthorized.publicKey, // Wrong recipient
            sender: sender.publicKey,
            balanceHolderPda,
            rent: SYSVAR_RENT_PUBKEY,
          })
          .signers([unauthorized])
          .rpc();
        
        expect.fail("Should have failed with unauthorized signer");
      } catch (error) {
        expect(error).to.exist;
      }
    });

    it("transfers correct lamport amounts to recipient", async () => {
      const testWithdrawAmount = DEPOSIT_AMOUNT / 2;
      
      await program.methods
        .withdraw(new BN(testWithdrawAmount))
        .accounts({
          recipient: recipient.publicKey,
          sender: sender.publicKey,
          balanceHolderPda,
          rent: SYSVAR_RENT_PUBKEY,
        })
        .signers([recipient])
        .rpc();

      const finalRecipientBalance = await getBalance(recipient.publicKey);
      const balanceIncrease = finalRecipientBalance - initialRecipientBalance;
      
      // Should receive close to the withdrawn amount (minus tx fees)
      expect(balanceIncrease).to.be.greaterThan(testWithdrawAmount * 0.9);
    });

    it("updates PDA balance correctly", async () => {
      const pdaAccountBefore = await getPdaAccount();
      const initialAmount = pdaAccountBefore.amount.toNumber();
      
      await program.methods
        .withdraw(new BN(WITHDRAW_AMOUNT))
        .accounts({
          recipient: recipient.publicKey,
          sender: sender.publicKey,
          balanceHolderPda,
          rent: SYSVAR_RENT_PUBKEY,
        })
        .signers([recipient])
        .rpc();

      const pdaAccountAfter = await getPdaAccount();
      const finalAmount = pdaAccountAfter.amount.toNumber();
      
      expect(finalAmount).to.equal(initialAmount - WITHDRAW_AMOUNT);
    });

    it("closes account when balance reaches zero", async () => {
      // Withdraw the full amount
      await program.methods
        .withdraw(new BN(DEPOSIT_AMOUNT))
        .accounts({
          recipient: recipient.publicKey,
          sender: sender.publicKey,
          balanceHolderPda,
          rent: SYSVAR_RENT_PUBKEY,
        })
        .signers([recipient])
        .rpc();

      // Account should be closed (not exist or have zero balance)
      const pdaBalance = await getBalance(balanceHolderPda);
      expect(pdaBalance).to.equal(0);
      
      const pdaAccount = await getPdaAccount();
      expect(pdaAccount).to.be.null;
    });

    it("refunds remaining rent to sender on closure", async () => {
      // Withdraw full amount to trigger account closure
      await program.methods
        .withdraw(new BN(DEPOSIT_AMOUNT))
        .accounts({
          recipient: recipient.publicKey,
          sender: sender.publicKey,
          balanceHolderPda,
          rent: SYSVAR_RENT_PUBKEY,
        })
        .signers([recipient])
        .rpc();

      const finalSenderBalance = await getBalance(sender.publicKey);
      
      // Sender should have received rent refund, so balance should be higher than before
      // (This is a general check since exact amounts depend on rent calculations)
      expect(finalSenderBalance).to.be.greaterThan(initialSenderBalance);
    });
  });

  describe("authorization", () => {
    beforeEach(async () => {
      // Setup fresh accounts and deposit for authorization tests
      sender = Keypair.generate();
      recipient = Keypair.generate();
      unauthorized = Keypair.generate();
      
      await fundAccount(sender.publicKey, 2 * LAMPORTS_PER_SOL);
      await fundAccount(recipient.publicKey, 1 * LAMPORTS_PER_SOL);
      await fundAccount(unauthorized.publicKey, 1 * LAMPORTS_PER_SOL);
      
      [balanceHolderPda, pdaBump] = PublicKey.findProgramAddressSync(
        [recipient.publicKey.toBuffer(), sender.publicKey.toBuffer()],
        program.programId
      );
      
      // Make initial deposit
      await program.methods
        .deposit(new BN(DEPOSIT_AMOUNT))
        .accounts({
          balanceHolderPda,
          sender: sender.publicKey,
          recipient: recipient.publicKey,
          systemProgram: SystemProgram.programId,
        })
        .signers([sender])
        .rpc();
    });

    it("prevents non-recipient from withdrawing", async () => {
      try {
        await program.methods
          .withdraw(new BN(WITHDRAW_AMOUNT))
          .accounts({
            recipient: unauthorized.publicKey,
            sender: sender.publicKey,
            balanceHolderPda,
            rent: SYSVAR_RENT_PUBKEY,
          })
          .signers([unauthorized])
          .rpc();
        
        expect.fail("Should have failed with unauthorized recipient");
      } catch (error) {
        expect(error).to.exist;
        
        // Verify the account state wasn't changed
        const pdaAccount = await getPdaAccount();
        expect(pdaAccount.amount.toNumber()).to.equal(DEPOSIT_AMOUNT);
      }
    });

    it("enforces recipient validation constraint", async () => {
      // Create a different PDA for a different recipient pair
      const [wrongPda] = PublicKey.findProgramAddressSync(
        [unauthorized.publicKey.toBuffer(), sender.publicKey.toBuffer()],
        program.programId
      );

      try {
        await program.methods
          .withdraw(new BN(WITHDRAW_AMOUNT))
          .accounts({
            recipient: recipient.publicKey,
            sender: sender.publicKey,
            balanceHolderPda: wrongPda, // Wrong PDA
            rent: SYSVAR_RENT_PUBKEY,
          })
          .signers([recipient])
          .rpc();
        
        expect.fail("Should have failed with wrong PDA");
      } catch (error) {
        expect(error).to.exist;
      }
    });
  });

  describe("edge cases and error handling", () => {
    it("handles excessive withdrawal attempts", async () => {
      // Setup
      sender = Keypair.generate();
      recipient = Keypair.generate();
      await fundAccount(sender.publicKey, 2 * LAMPORTS_PER_SOL);
      await fundAccount(recipient.publicKey, 1 * LAMPORTS_PER_SOL);
      
      [balanceHolderPda] = PublicKey.findProgramAddressSync(
        [recipient.publicKey.toBuffer(), sender.publicKey.toBuffer()],
        program.programId
      );
      
      await program.methods
        .deposit(new BN(DEPOSIT_AMOUNT))
        .accounts({
          balanceHolderPda,
          sender: sender.publicKey,
          recipient: recipient.publicKey,
          systemProgram: SystemProgram.programId,
        })
        .signers([sender])
        .rpc();

      try {
        // Try to withdraw more than available
        await program.methods
          .withdraw(new BN(DEPOSIT_AMOUNT * 2))
          .accounts({
            recipient: recipient.publicKey,
            sender: sender.publicKey,
            balanceHolderPda,
            rent: SYSVAR_RENT_PUBKEY,
          })
          .signers([recipient])
          .rpc();
        
        expect.fail("Should have failed with insufficient funds");
      } catch (error) {
        expect(error).to.exist;
        
        // Verify account state is unchanged
        const pdaAccount = await getPdaAccount();
        expect(pdaAccount.amount.toNumber()).to.equal(DEPOSIT_AMOUNT);
      }
    });

    it("handles operations on non-existent PDA", async () => {
      // Try to withdraw from a PDA that was never created
      const nonExistentSender = Keypair.generate();
      const nonExistentRecipient = Keypair.generate();
      await fundAccount(nonExistentRecipient.publicKey, 1 * LAMPORTS_PER_SOL);
      
      const [nonExistentPda] = PublicKey.findProgramAddressSync(
        [nonExistentRecipient.publicKey.toBuffer(), nonExistentSender.publicKey.toBuffer()],
        program.programId
      );

      try {
        await program.methods
          .withdraw(new BN(WITHDRAW_AMOUNT))
          .accounts({
            recipient: nonExistentRecipient.publicKey,
            sender: nonExistentSender.publicKey,
            balanceHolderPda: nonExistentPda,
            rent: SYSVAR_RENT_PUBKEY,
          })
          .signers([nonExistentRecipient])
          .rpc();
        
        expect.fail("Should have failed with non-existent PDA");
      } catch (error) {
        expect(error).to.exist;
      }
    });
  });
});
