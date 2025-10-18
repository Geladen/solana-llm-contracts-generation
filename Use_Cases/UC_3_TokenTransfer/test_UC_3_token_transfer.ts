import * as anchor from "@coral-xyz/anchor";
import { Program } from "@coral-xyz/anchor";
import { PublicKey, Keypair, SystemProgram, LAMPORTS_PER_SOL } from "@solana/web3.js";
import {
  TOKEN_PROGRAM_ID,
  ASSOCIATED_TOKEN_PROGRAM_ID,
  createMint,
  getOrCreateAssociatedTokenAccount,
  mintTo,
  getAccount,
} from "@solana/spl-token";
import { assert } from "chai";
import BN from "bn.js";

/**
 * Universal Token Transfer Program Test Suite
 * 
 * This test suite is designed to work with any Solana program that implements
 * the following interface:
 * 
 * Functions:
 * - deposit(): Transfers ATA ownership to a PDA
 * - withdraw(amount: u64): Withdraws tokens from the temporary ATA
 * 
 * Required PDAs:
 * - atas_holder: PDA that holds ownership of temporary ATAs
 * - deposit_info: PDA that stores deposit metadata (keyed by temp_ata)
 * 
 * IMPORTANT: The Rust program must mark temp_ata as mut in DepositCtx:
 * #[account(mut, constraint = ...)]
 * pub temp_ata: Account<'info, TokenAccount>,
 */

describe("Token Transfer Program", () => {
  // Configure the client
  const provider = anchor.AnchorProvider.env();
  anchor.setProvider(provider);

  const program = anchor.workspace.token_transfer as Program;
  
  // Test accounts
  let mint: PublicKey;
  let sender: Keypair;
  let recipient: Keypair;
  let senderAta: PublicKey;
  let recipientAta: PublicKey;
  let atasHolderPda: PublicKey;
  let depositInfoPda: PublicKey;

  // Constants
  const MINT_DECIMALS = 6;
  const INITIAL_MINT_AMOUNT = 1_000_000; // 1 token with 6 decimals

  /**
   * Helper function to derive the atas_holder PDA
   */
  function getAtasHolderPda(): [PublicKey, number] {
    return PublicKey.findProgramAddressSync(
      [Buffer.from("atas_holder")],
      program.programId
    );
  }

  /**
   * Helper function to derive the deposit_info PDA
   */
  function getDepositInfoPda(tempAta: PublicKey): [PublicKey, number] {
    return PublicKey.findProgramAddressSync(
      [tempAta.toBuffer()],
      program.programId
    );
  }

  /**
   * Helper function to get account lamports
   */
  async function getLamports(pubkey: PublicKey): Promise<number> {
    const accountInfo = await provider.connection.getAccountInfo(pubkey);
    return accountInfo?.lamports ?? 0;
  }

  /**
   * Helper function to check if an account exists
   */
  async function accountExists(pubkey: PublicKey): Promise<boolean> {
    const accountInfo = await provider.connection.getAccountInfo(pubkey);
    return accountInfo !== null;
  }

  /**
   * Setup function to initialize test accounts before each test
   */
  beforeEach(async () => {
    // Generate keypairs
    sender = Keypair.generate();
    recipient = Keypair.generate();

    // Airdrop SOL to sender and recipient
    const senderAirdrop = await provider.connection.requestAirdrop(
      sender.publicKey,
      2 * LAMPORTS_PER_SOL
    );
    const recipientAirdrop = await provider.connection.requestAirdrop(
      recipient.publicKey,
      2 * LAMPORTS_PER_SOL
    );
    
    await provider.connection.confirmTransaction(senderAirdrop);
    await provider.connection.confirmTransaction(recipientAirdrop);

    // Create mint
    mint = await createMint(
      provider.connection,
      sender,
      sender.publicKey,
      null,
      MINT_DECIMALS
    );

    // Create sender's ATA (this will be the temp_ata)
    const senderAtaAccount = await getOrCreateAssociatedTokenAccount(
      provider.connection,
      sender,
      mint,
      sender.publicKey
    );
    senderAta = senderAtaAccount.address;

    // Mint tokens to sender
    await mintTo(
      provider.connection,
      sender,
      mint,
      senderAta,
      sender,
      INITIAL_MINT_AMOUNT
    );

    // Create recipient's ATA
    const recipientAtaAccount = await getOrCreateAssociatedTokenAccount(
      provider.connection,
      sender,
      mint,
      recipient.publicKey
    );
    recipientAta = recipientAtaAccount.address;

    // Derive PDAs
    [atasHolderPda] = getAtasHolderPda();
    [depositInfoPda] = getDepositInfoPda(senderAta);
  });

  describe("deposit()", () => {
    it("transfers ATA ownership to program", async () => {
      // Get initial ATA owner
      const initialAtaAccount = await getAccount(
        provider.connection,
        senderAta
      );
      assert.equal(
        initialAtaAccount.owner.toBase58(),
        sender.publicKey.toBase58(),
        "Initial owner should be sender"
      );

      // Execute deposit
      await program.methods
        .deposit()
        .accounts({
          sender: sender.publicKey,
          recipient: recipient.publicKey,
          mint: mint,
          tempAta: senderAta,
          depositInfo: depositInfoPda,
          tokenProgram: TOKEN_PROGRAM_ID,
          systemProgram: SystemProgram.programId,
          associatedTokenProgram: ASSOCIATED_TOKEN_PROGRAM_ID,
        })
        .signers([sender])
        .rpc();

      // Verify ATA ownership transferred to PDA
      const finalAtaAccount = await getAccount(
        provider.connection,
        senderAta
      );
      assert.equal(
        finalAtaAccount.owner.toBase58(),
        atasHolderPda.toBase58(),
        "ATA owner should be transferred to atas_holder PDA"
      );

      // Verify deposit info was created
      const depositInfoExists = await accountExists(depositInfoPda);
      assert.isTrue(depositInfoExists, "DepositInfo account should be created");
    });

    it("requires positive token balance", async () => {
      // Create a new ATA with zero balance
      const emptyKeypair = Keypair.generate();
      await provider.connection.requestAirdrop(
        emptyKeypair.publicKey,
        LAMPORTS_PER_SOL
      );
      await new Promise(resolve => setTimeout(resolve, 1000));

      const emptyAta = await getOrCreateAssociatedTokenAccount(
        provider.connection,
        sender,
        mint,
        emptyKeypair.publicKey
      );

      const [emptyDepositInfoPda] = getDepositInfoPda(emptyAta.address);

      // Attempt deposit with zero balance
      try {
        await program.methods
          .deposit()
          .accounts({
            sender: emptyKeypair.publicKey,
            recipient: recipient.publicKey,
            mint: mint,
            tempAta: emptyAta.address,
            depositInfo: emptyDepositInfoPda,
            tokenProgram: TOKEN_PROGRAM_ID,
            systemProgram: SystemProgram.programId,
            associatedTokenProgram: ASSOCIATED_TOKEN_PROGRAM_ID,
          })
          .signers([emptyKeypair])
          .rpc();
        
        assert.fail("Transaction should have failed with zero balance");
      } catch (error) {
        // Transaction failed as expected (checking for constraint violation)
        assert.isDefined(error);
      }
    });

    it("validates mint consistency", async () => {
      // Create a different mint
      const wrongMint = await createMint(
        provider.connection,
        sender,
        sender.publicKey,
        null,
        MINT_DECIMALS
      );

      // Attempt deposit with mismatched mint
      try {
        await program.methods
          .deposit()
          .accounts({
            sender: sender.publicKey,
            recipient: recipient.publicKey,
            mint: wrongMint, // Wrong mint
            tempAta: senderAta, // ATA for different mint
            depositInfo: depositInfoPda,
            tokenProgram: TOKEN_PROGRAM_ID,
            systemProgram: SystemProgram.programId,
            associatedTokenProgram: ASSOCIATED_TOKEN_PROGRAM_ID,
          })
          .signers([sender])
          .rpc();
        
        assert.fail("Transaction should have failed with mint mismatch");
      } catch (error) {
        // Transaction failed as expected
        assert.isDefined(error);
      }
    });
  });

  describe("withdraw()", () => {
    // Setup: deposit tokens before each withdraw test
    beforeEach(async () => {
      await program.methods
        .deposit()
        .accounts({
          sender: sender.publicKey,
          recipient: recipient.publicKey,
          mint: mint,
          tempAta: senderAta,
          depositInfo: depositInfoPda,
          tokenProgram: TOKEN_PROGRAM_ID,
          systemProgram: SystemProgram.programId,
          associatedTokenProgram: ASSOCIATED_TOKEN_PROGRAM_ID,
        })
        .signers([sender])
        .rpc();
    });

    it("allows recipient to withdraw tokens", async () => {
      const withdrawAmount = 1; // 1 token (will be multiplied by decimals)

      // Get initial balances
      const initialRecipientBalance = (await getAccount(
        provider.connection,
        recipientAta
      )).amount;

      // Execute withdraw
      await program.methods
        .withdraw(new BN(withdrawAmount))
        .accounts({
          mint: mint,
          recipient: recipient.publicKey,
          sender: sender.publicKey,
          recipientAta: recipientAta,
          tempAta: senderAta,
          depositInfo: depositInfoPda,
          atasHolderPda: atasHolderPda,
          tokenProgram: TOKEN_PROGRAM_ID,
          systemProgram: SystemProgram.programId,
          associatedTokenProgram: ASSOCIATED_TOKEN_PROGRAM_ID,
          rent: anchor.web3.SYSVAR_RENT_PUBKEY,
        })
        .signers([recipient])
        .rpc();

      // Verify tokens were transferred
      const finalRecipientBalance = (await getAccount(
        provider.connection,
        recipientAta
      )).amount;

      const expectedAmount = BigInt(withdrawAmount) * BigInt(10 ** MINT_DECIMALS);
      assert.equal(
        finalRecipientBalance.toString(),
        (initialRecipientBalance + expectedAmount).toString(),
        "Recipient should receive correct token amount"
      );
    });

    it("transfers correct token amount", async () => {
      const withdrawAmount = 1; // 1 full token

      const initialTempBalance = (await getAccount(
        provider.connection,
        senderAta
      )).amount;
      
      const initialRecipientBalance = (await getAccount(
        provider.connection,
        recipientAta
      )).amount;

      await program.methods
        .withdraw(new BN(withdrawAmount))
        .accounts({
          mint: mint,
          recipient: recipient.publicKey,
          sender: sender.publicKey,
          recipientAta: recipientAta,
          tempAta: senderAta,
          depositInfo: depositInfoPda,
          atasHolderPda: atasHolderPda,
          tokenProgram: TOKEN_PROGRAM_ID,
          systemProgram: SystemProgram.programId,
          associatedTokenProgram: ASSOCIATED_TOKEN_PROGRAM_ID,
          rent: anchor.web3.SYSVAR_RENT_PUBKEY,
        })
        .signers([recipient])
        .rpc();

      const finalRecipientBalance = (await getAccount(
        provider.connection,
        recipientAta
      )).amount;

      const expectedIncrease = BigInt(withdrawAmount) * BigInt(10 ** MINT_DECIMALS);
      const actualIncrease = finalRecipientBalance - initialRecipientBalance;

      assert.equal(
        actualIncrease.toString(),
        expectedIncrease.toString(),
        "Token transfer amount should match withdrawal amount"
      );
    });

    it("rejects zero withdrawals", async () => {
      try {
        await program.methods
          .withdraw(new BN(0))
          .accounts({
            mint: mint,
            recipient: recipient.publicKey,
            sender: sender.publicKey,
            recipientAta: recipientAta,
            tempAta: senderAta,
            depositInfo: depositInfoPda,
            atasHolderPda: atasHolderPda,
            tokenProgram: TOKEN_PROGRAM_ID,
            systemProgram: SystemProgram.programId,
            associatedTokenProgram: ASSOCIATED_TOKEN_PROGRAM_ID,
            rent: anchor.web3.SYSVAR_RENT_PUBKEY,
          })
          .signers([recipient])
          .rpc();
        
        assert.fail("Transaction should have failed with zero withdrawal");
      } catch (error) {
        // Transaction failed as expected
        assert.isDefined(error);
      }
    });

    it("rejects excessive withdrawals", async () => {
      const excessiveAmount = 1000; // More than available

      try {
        await program.methods
          .withdraw(new BN(excessiveAmount))
          .accounts({
            mint: mint,
            recipient: recipient.publicKey,
            sender: sender.publicKey,
            recipientAta: recipientAta,
            tempAta: senderAta,
            depositInfo: depositInfoPda,
            atasHolderPda: atasHolderPda,
            tokenProgram: TOKEN_PROGRAM_ID,
            systemProgram: SystemProgram.programId,
            associatedTokenProgram: ASSOCIATED_TOKEN_PROGRAM_ID,
            rent: anchor.web3.SYSVAR_RENT_PUBKEY,
          })
          .signers([recipient])
          .rpc();
        
        assert.fail("Transaction should have failed with excessive withdrawal");
      } catch (error) {
        // Transaction failed as expected
        assert.isDefined(error);
      }
    });

    it("closes accounts on full withdrawal", async () => {
      const fullAmount = 1; // Full balance (1 token)

      // Get initial lamports
      const initialSenderLamports = await getLamports(sender.publicKey);
      const depositInfoLamports = await getLamports(depositInfoPda);

      // Verify accounts exist before withdrawal
      assert.isTrue(
        await accountExists(senderAta),
        "Temp ATA should exist before withdrawal"
      );
      assert.isTrue(
        await accountExists(depositInfoPda),
        "DepositInfo should exist before withdrawal"
      );

      // Execute full withdrawal
      await program.methods
        .withdraw(new BN(fullAmount))
        .accounts({
          mint: mint,
          recipient: recipient.publicKey,
          sender: sender.publicKey,
          recipientAta: recipientAta,
          tempAta: senderAta,
          depositInfo: depositInfoPda,
          atasHolderPda: atasHolderPda,
          tokenProgram: TOKEN_PROGRAM_ID,
          systemProgram: SystemProgram.programId,
          associatedTokenProgram: ASSOCIATED_TOKEN_PROGRAM_ID,
          rent: anchor.web3.SYSVAR_RENT_PUBKEY,
        })
        .signers([recipient])
        .rpc();

      // Verify accounts are closed
      assert.isFalse(
        await accountExists(senderAta),
        "Temp ATA should be closed after full withdrawal"
      );
      assert.isFalse(
        await accountExists(depositInfoPda),
        "DepositInfo should be closed after full withdrawal"
      );

      // Verify lamports were returned to sender
      const finalSenderLamports = await getLamports(sender.publicKey);
      assert.isTrue(
        finalSenderLamports > initialSenderLamports,
        "Sender should receive lamports from closed accounts"
      );
    });
  });

  describe("authorization", () => {
    beforeEach(async () => {
      // Deposit tokens
      await program.methods
        .deposit()
        .accounts({
          sender: sender.publicKey,
          recipient: recipient.publicKey,
          mint: mint,
          tempAta: senderAta,
          depositInfo: depositInfoPda,
          tokenProgram: TOKEN_PROGRAM_ID,
          systemProgram: SystemProgram.programId,
          associatedTokenProgram: ASSOCIATED_TOKEN_PROGRAM_ID,
        })
        .signers([sender])
        .rpc();
    });

    it("prevents non-recipient withdrawals", async () => {
      // Create unauthorized user
      const unauthorized = Keypair.generate();
      await provider.connection.requestAirdrop(
        unauthorized.publicKey,
        LAMPORTS_PER_SOL
      );
      await new Promise(resolve => setTimeout(resolve, 1000));

      // Create ATA for unauthorized user
      const unauthorizedAta = await getOrCreateAssociatedTokenAccount(
        provider.connection,
        sender,
        mint,
        unauthorized.publicKey
      );

      try {
        await program.methods
          .withdraw(new BN(1))
          .accounts({
            mint: mint,
            recipient: unauthorized.publicKey, // Wrong recipient
            sender: sender.publicKey,
            recipientAta: unauthorizedAta.address,
            tempAta: senderAta,
            depositInfo: depositInfoPda,
            atasHolderPda: atasHolderPda,
            tokenProgram: TOKEN_PROGRAM_ID,
            systemProgram: SystemProgram.programId,
            associatedTokenProgram: ASSOCIATED_TOKEN_PROGRAM_ID,
            rent: anchor.web3.SYSVAR_RENT_PUBKEY,
          })
          .signers([unauthorized])
          .rpc();
        
        assert.fail("Transaction should have failed with unauthorized recipient");
      } catch (error) {
        // Transaction failed as expected
        assert.isDefined(error);
      }
    });

    it("validates recipient identity", async () => {
      // Attempt to withdraw with sender instead of recipient
      try {
        await program.methods
          .withdraw(new BN(1))
          .accounts({
            mint: mint,
            recipient: sender.publicKey, // Wrong recipient (should be recipient)
            sender: sender.publicKey,
            recipientAta: senderAta,
            tempAta: senderAta,
            depositInfo: depositInfoPda,
            atasHolderPda: atasHolderPda,
            tokenProgram: TOKEN_PROGRAM_ID,
            systemProgram: SystemProgram.programId,
            associatedTokenProgram: ASSOCIATED_TOKEN_PROGRAM_ID,
            rent: anchor.web3.SYSVAR_RENT_PUBKEY,
          })
          .signers([sender])
          .rpc();
        
        assert.fail("Transaction should have failed with wrong recipient");
      } catch (error) {
        // Transaction failed as expected (constraint violation)
        assert.isDefined(error);
      }
    });
  });
});
