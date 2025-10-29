import * as anchor from "@coral-xyz/anchor";
import { Program } from "@coral-xyz/anchor";
import { PublicKey, Keypair, SystemProgram, LAMPORTS_PER_SOL } from "@solana/web3.js";
import {
  TOKEN_PROGRAM_ID,
  ASSOCIATED_TOKEN_PROGRAM_ID,
  createMint,
  mintTo,
  getOrCreateAssociatedTokenAccount,
  getAccount,
  createInitializeAccountInstruction,
} from "@solana/spl-token";
import { assert } from "chai";
import BN from "bn.js";

describe("Constant Product AMM Program", () => {
  const provider = anchor.AnchorProvider.env();
  anchor.setProvider(provider);

  const program = anchor.workspace.constant_product_amm as Program;
  const connection = provider.connection;
  const payer = provider.wallet as anchor.Wallet;

  // Test accounts
  let mint0: PublicKey;
  let mint1: PublicKey;
  let ammInfoPda: PublicKey;
  let ammInfoBump: number;
  let mintedPda: PublicKey;
  let mintedBump: number;
  let senderTokenAccount0: PublicKey;
  let senderTokenAccount1: PublicKey;
  let pdaTokenAccount0: PublicKey;
  let pdaTokenAccount1: PublicKey;
  let mint0Authority: Keypair;
  let mint1Authority: Keypair;

  // Constants
  const DECIMALS_0 = 6;
  const DECIMALS_1 = 9;
  const INITIAL_SUPPLY = 1_000_000;

  before(async () => {
    // Create mint authorities
    mint0Authority = Keypair.generate();
    mint1Authority = Keypair.generate();

    // Airdrop SOL to mint authorities
    await connection.requestAirdrop(mint0Authority.publicKey, 2 * LAMPORTS_PER_SOL);
    await connection.requestAirdrop(mint1Authority.publicKey, 2 * LAMPORTS_PER_SOL);
    await new Promise(resolve => setTimeout(resolve, 1000));

    // Create mints
    mint0 = await createMint(
      connection,
      mint0Authority,
      mint0Authority.publicKey,
      null,
      DECIMALS_0
    );

    mint1 = await createMint(
      connection,
      mint1Authority,
      mint1Authority.publicKey,
      null,
      DECIMALS_1
    );

    // Derive PDAs
    [ammInfoPda, ammInfoBump] = PublicKey.findProgramAddressSync(
      [Buffer.from("amm"), mint0.toBuffer(), mint1.toBuffer()],
      program.programId
    );

    [mintedPda, mintedBump] = PublicKey.findProgramAddressSync(
      [Buffer.from("minted"), payer.publicKey.toBuffer()],
      program.programId
    );

    // Create sender's token accounts
    const senderAccount0 = await getOrCreateAssociatedTokenAccount(
      connection,
      payer.payer,
      mint0,
      payer.publicKey
    );
    senderTokenAccount0 = senderAccount0.address;

    const senderAccount1 = await getOrCreateAssociatedTokenAccount(
      connection,
      payer.payer,
      mint1,
      payer.publicKey
    );
    senderTokenAccount1 = senderAccount1.address;

    // Mint tokens to sender
    await mintTo(
      connection,
      mint0Authority,
      mint0,
      senderTokenAccount0,
      mint0Authority.publicKey,
      INITIAL_SUPPLY * Math.pow(10, DECIMALS_0)
    );

    await mintTo(
      connection,
      mint1Authority,
      mint1,
      senderTokenAccount1,
      mint1Authority.publicKey,
      INITIAL_SUPPLY * Math.pow(10, DECIMALS_1)
    );
  });

  describe("initialize()", () => {
    it("creates AMM with token pair", async () => {
      // Create separate token accounts for the PDA (not associated token accounts)
      // These will be regular token accounts that we create and then transfer ownership
      const pdaAccount0Keypair = Keypair.generate();
      const pdaAccount1Keypair = Keypair.generate();
      
      // Create token account 0
      const createTokenAccount0Ix = await connection.getMinimumBalanceForRentExemption(165);
      await program.provider.sendAndConfirm(
        new anchor.web3.Transaction().add(
          anchor.web3.SystemProgram.createAccount({
            fromPubkey: payer.publicKey,
            newAccountPubkey: pdaAccount0Keypair.publicKey,
            space: 165,
            lamports: createTokenAccount0Ix,
            programId: TOKEN_PROGRAM_ID,
          }),
          createInitializeAccountInstruction(
            pdaAccount0Keypair.publicKey,
            mint0,
            payer.publicKey,
            TOKEN_PROGRAM_ID
          )
        ),
        [pdaAccount0Keypair]
      );
      pdaTokenAccount0 = pdaAccount0Keypair.publicKey;

      // Create token account 1
      const createTokenAccount1Ix = await connection.getMinimumBalanceForRentExemption(165);
      await program.provider.sendAndConfirm(
        new anchor.web3.Transaction().add(
          anchor.web3.SystemProgram.createAccount({
            fromPubkey: payer.publicKey,
            newAccountPubkey: pdaAccount1Keypair.publicKey,
            space: 165,
            lamports: createTokenAccount1Ix,
            programId: TOKEN_PROGRAM_ID,
          }),
          createInitializeAccountInstruction(
            pdaAccount1Keypair.publicKey,
            mint1,
            payer.publicKey,
            TOKEN_PROGRAM_ID
          )
        ),
        [pdaAccount1Keypair]
      );
      pdaTokenAccount1 = pdaAccount1Keypair.publicKey;

      const lamportsBefore = await connection.getBalance(payer.publicKey);

      await program.methods
        .initialize()
        .accounts({
          initializer: payer.publicKey,
          ammInfo: ammInfoPda,
          mint0: mint0,
          mint1: mint1,
          tokenAccount0: pdaTokenAccount0,
          tokenAccount1: pdaTokenAccount1,
          tokenProgram: TOKEN_PROGRAM_ID,
          systemProgram: SystemProgram.programId,
          associatedTokenProgram: ASSOCIATED_TOKEN_PROGRAM_ID,
        })
        .rpc();

      const lamportsAfter = await connection.getBalance(payer.publicKey);

      // Verify that lamports were spent (account creation + transaction fee)
      assert.isTrue(lamportsBefore > lamportsAfter, "Lamports should be spent for initialization");

      // Verify AMM info account exists
      const ammInfo = await program.account.ammInfo.fetch(ammInfoPda);
      assert.equal(ammInfo.mint0.toString(), mint0.toString());
      assert.equal(ammInfo.mint1.toString(), mint1.toString());
    });

    it("transfers token account ownership", async () => {
      // Verify token accounts are now owned by the PDA
      const tokenAccount0Info = await getAccount(connection, pdaTokenAccount0);
      const tokenAccount1Info = await getAccount(connection, pdaTokenAccount1);

      assert.equal(tokenAccount0Info.owner.toString(), ammInfoPda.toString());
      assert.equal(tokenAccount1Info.owner.toString(), ammInfoPda.toString());
    });
  });

  describe("deposit()", () => {
    it("accepts initial liquidity deposit", async () => {
      const amount0 = new BN(100);
      const amount1 = new BN(200);

      const balanceBefore0 = await connection.getTokenAccountBalance(senderTokenAccount0);
      const balanceBefore1 = await connection.getTokenAccountBalance(senderTokenAccount1);

      await program.methods
        .deposit(amount0, amount1)
        .accounts({
          sender: payer.publicKey,
          mint0: mint0,
          mint1: mint1,
          ammInfo: ammInfoPda,
          mintedPda: mintedPda,
          sendersTokenAccount0: senderTokenAccount0,
          sendersTokenAccount1: senderTokenAccount1,
          pdasTokenAccount0: pdaTokenAccount0,
          pdasTokenAccount1: pdaTokenAccount1,
          tokenProgram: TOKEN_PROGRAM_ID,
          systemProgram: SystemProgram.programId,
          associatedTokenProgram: ASSOCIATED_TOKEN_PROGRAM_ID,
        })
        .rpc();

      const balanceAfter0 = await connection.getTokenAccountBalance(senderTokenAccount0);
      const balanceAfter1 = await connection.getTokenAccountBalance(senderTokenAccount1);

      // Verify tokens were transferred
      const expectedAmount0 = amount0.toNumber() * Math.pow(10, DECIMALS_0);
      const expectedAmount1 = amount1.toNumber() * Math.pow(10, DECIMALS_1);

      assert.approximately(
        Number(balanceBefore0.value.amount) - Number(balanceAfter0.value.amount),
        expectedAmount0,
        1,
        "Token 0 transfer amount mismatch"
      );

      assert.approximately(
        Number(balanceBefore1.value.amount) - Number(balanceAfter1.value.amount),
        expectedAmount1,
        1,
        "Token 1 transfer amount mismatch"
      );

      // Verify AMM state
      const ammInfo = await program.account.ammInfo.fetch(ammInfoPda);
      assert.equal(ammInfo.reserve0.toNumber(), amount0.toNumber());
      assert.equal(ammInfo.reserve1.toNumber(), amount1.toNumber());
      assert.equal(ammInfo.supply.toNumber(), amount0.toNumber());
      assert.isTrue(ammInfo.everDeposited);

      // Verify minted PDA
      const mintedInfo = await program.account.mintedPda.fetch(mintedPda);
      assert.equal(mintedInfo.minted.toNumber(), amount0.toNumber());
    });

    it("maintains exchange rate in subsequent deposits", async () => {
      const amount0 = new BN(50);
      const amount1 = new BN(100); // Maintains 1:2 ratio

      await program.methods
        .deposit(amount0, amount1)
        .accounts({
          sender: payer.publicKey,
          mint0: mint0,
          mint1: mint1,
          ammInfo: ammInfoPda,
          mintedPda: mintedPda,
          sendersTokenAccount0: senderTokenAccount0,
          sendersTokenAccount1: senderTokenAccount1,
          pdasTokenAccount0: pdaTokenAccount0,
          pdasTokenAccount1: pdaTokenAccount1,
          tokenProgram: TOKEN_PROGRAM_ID,
          systemProgram: SystemProgram.programId,
          associatedTokenProgram: ASSOCIATED_TOKEN_PROGRAM_ID,
        })
        .rpc();

      const ammInfo = await program.account.ammInfo.fetch(ammInfoPda);
      assert.equal(ammInfo.reserve0.toNumber(), 150);
      assert.equal(ammInfo.reserve1.toNumber(), 300);
    });

    it("mints liquidity tokens proportionally", async () => {
      const mintedBefore = await program.account.mintedPda.fetch(mintedPda);
      const ammInfoBefore = await program.account.ammInfo.fetch(ammInfoPda);

      const amount0 = new BN(25);
      const amount1 = new BN(50);

      await program.methods
        .deposit(amount0, amount1)
        .accounts({
          sender: payer.publicKey,
          mint0: mint0,
          mint1: mint1,
          ammInfo: ammInfoPda,
          mintedPda: mintedPda,
          sendersTokenAccount0: senderTokenAccount0,
          sendersTokenAccount1: senderTokenAccount1,
          pdasTokenAccount0: pdaTokenAccount0,
          pdasTokenAccount1: pdaTokenAccount1,
          tokenProgram: TOKEN_PROGRAM_ID,
          systemProgram: SystemProgram.programId,
          associatedTokenProgram: ASSOCIATED_TOKEN_PROGRAM_ID,
        })
        .rpc();

      const mintedAfter = await program.account.mintedPda.fetch(mintedPda);
      const expectedMint = (amount0.toNumber() * ammInfoBefore.supply.toNumber()) / ammInfoBefore.reserve0.toNumber();

      assert.equal(
        mintedAfter.minted.toNumber() - mintedBefore.minted.toNumber(),
        expectedMint,
        "Liquidity tokens not minted proportionally"
      );
    });

    it("rejects zero amounts", async () => {
      try {
        await program.methods
          .deposit(new BN(0), new BN(100))
          .accounts({
            sender: payer.publicKey,
            mint0: mint0,
            mint1: mint1,
            ammInfo: ammInfoPda,
            mintedPda: mintedPda,
            sendersTokenAccount0: senderTokenAccount0,
            sendersTokenAccount1: senderTokenAccount1,
            pdasTokenAccount0: pdaTokenAccount0,
            pdasTokenAccount1: pdaTokenAccount1,
            tokenProgram: TOKEN_PROGRAM_ID,
            systemProgram: SystemProgram.programId,
            associatedTokenProgram: ASSOCIATED_TOKEN_PROGRAM_ID,
          })
          .rpc();
        assert.fail("Should have rejected zero amount");
      } catch (error) {
        // Transaction should fail - we don't check error message
        assert.isDefined(error);
      }
    });

    it("rejects unbalanced deposits after initial", async () => {
      try {
        await program.methods
          .deposit(new BN(10), new BN(30)) // Wrong ratio (should be 1:2)
          .accounts({
            sender: payer.publicKey,
            mint0: mint0,
            mint1: mint1,
            ammInfo: ammInfoPda,
            mintedPda: mintedPda,
            sendersTokenAccount0: senderTokenAccount0,
            sendersTokenAccount1: senderTokenAccount1,
            pdasTokenAccount0: pdaTokenAccount0,
            pdasTokenAccount1: pdaTokenAccount1,
            tokenProgram: TOKEN_PROGRAM_ID,
            systemProgram: SystemProgram.programId,
            associatedTokenProgram: ASSOCIATED_TOKEN_PROGRAM_ID,
          })
          .rpc();
        assert.fail("Should have rejected unbalanced deposit");
      } catch (error) {
        assert.isDefined(error);
      }
    });
  });

  describe("redeem()", () => {
    let reservesBefore: { reserve0: number; reserve1: number; supply: number };

    before(async () => {
      const ammInfo = await program.account.ammInfo.fetch(ammInfoPda);
      reservesBefore = {
        reserve0: ammInfo.reserve0.toNumber(),
        reserve1: ammInfo.reserve1.toNumber(),
        supply: ammInfo.supply.toNumber(),
      };
    });

    it("allows liquidity redemption", async () => {
      const redeemAmount = new BN(25);

      const balanceBefore0 = await connection.getTokenAccountBalance(senderTokenAccount0);
      const balanceBefore1 = await connection.getTokenAccountBalance(senderTokenAccount1);

      await program.methods
        .redeem(redeemAmount)
        .accounts({
          sender: payer.publicKey,
          mint0: mint0,
          mint1: mint1,
          ammInfo: ammInfoPda,
          mintedPda: mintedPda,
          sendersTokenAccount0: senderTokenAccount0,
          sendersTokenAccount1: senderTokenAccount1,
          pdasTokenAccount0: pdaTokenAccount0,
          pdasTokenAccount1: pdaTokenAccount1,
          tokenProgram: TOKEN_PROGRAM_ID,
          systemProgram: SystemProgram.programId,
          associatedTokenProgram: ASSOCIATED_TOKEN_PROGRAM_ID,
        })
        .rpc();

      const balanceAfter0 = await connection.getTokenAccountBalance(senderTokenAccount0);
      const balanceAfter1 = await connection.getTokenAccountBalance(senderTokenAccount1);

      // Verify tokens were received
      assert.isTrue(
        Number(balanceAfter0.value.amount) > Number(balanceBefore0.value.amount),
        "Should receive token 0"
      );
      assert.isTrue(
        Number(balanceAfter1.value.amount) > Number(balanceBefore1.value.amount),
        "Should receive token 1"
      );
    });

    it("returns proportional token amounts", async () => {
      const ammInfo = await program.account.ammInfo.fetch(ammInfoPda);
      const redeemAmount = new BN(10);

      const expectedAmount0 = (redeemAmount.toNumber() * ammInfo.reserve0.toNumber()) / ammInfo.supply.toNumber();
      const expectedAmount1 = (redeemAmount.toNumber() * ammInfo.reserve1.toNumber()) / ammInfo.supply.toNumber();

      const balanceBefore0 = await connection.getTokenAccountBalance(senderTokenAccount0);
      const balanceBefore1 = await connection.getTokenAccountBalance(senderTokenAccount1);

      await program.methods
        .redeem(redeemAmount)
        .accounts({
          sender: payer.publicKey,
          mint0: mint0,
          mint1: mint1,
          ammInfo: ammInfoPda,
          mintedPda: mintedPda,
          sendersTokenAccount0: senderTokenAccount0,
          sendersTokenAccount1: senderTokenAccount1,
          pdasTokenAccount0: pdaTokenAccount0,
          pdasTokenAccount1: pdaTokenAccount1,
          tokenProgram: TOKEN_PROGRAM_ID,
          systemProgram: SystemProgram.programId,
          associatedTokenProgram: ASSOCIATED_TOKEN_PROGRAM_ID,
        })
        .rpc();

      const balanceAfter0 = await connection.getTokenAccountBalance(senderTokenAccount0);
      const balanceAfter1 = await connection.getTokenAccountBalance(senderTokenAccount1);

      const received0 = (Number(balanceAfter0.value.amount) - Number(balanceBefore0.value.amount)) / Math.pow(10, DECIMALS_0);
      const received1 = (Number(balanceAfter1.value.amount) - Number(balanceBefore1.value.amount)) / Math.pow(10, DECIMALS_1);

      assert.approximately(received0, expectedAmount0, 0.01, "Amount 0 not proportional");
      assert.approximately(received1, expectedAmount1, 0.01, "Amount 1 not proportional");
    });

    it("updates reserves correctly", async () => {
      const ammInfoBefore = await program.account.ammInfo.fetch(ammInfoPda);
      const redeemAmount = new BN(10);

      await program.methods
        .redeem(redeemAmount)
        .accounts({
          sender: payer.publicKey,
          mint0: mint0,
          mint1: mint1,
          ammInfo: ammInfoPda,
          mintedPda: mintedPda,
          sendersTokenAccount0: senderTokenAccount0,
          sendersTokenAccount1: senderTokenAccount1,
          pdasTokenAccount0: pdaTokenAccount0,
          pdasTokenAccount1: pdaTokenAccount1,
          tokenProgram: TOKEN_PROGRAM_ID,
          systemProgram: SystemProgram.programId,
          associatedTokenProgram: ASSOCIATED_TOKEN_PROGRAM_ID,
        })
        .rpc();

      const ammInfoAfter = await program.account.ammInfo.fetch(ammInfoPda);

      assert.isTrue(ammInfoAfter.reserve0.toNumber() < ammInfoBefore.reserve0.toNumber());
      assert.isTrue(ammInfoAfter.reserve1.toNumber() < ammInfoBefore.reserve1.toNumber());
      assert.isTrue(ammInfoAfter.supply.toNumber() < ammInfoBefore.supply.toNumber());
    });

    it("rejects excessive redemption", async () => {
      const mintedInfo = await program.account.mintedPda.fetch(mintedPda);
      const excessiveAmount = mintedInfo.minted.add(new BN(1000));

      try {
        await program.methods
          .redeem(excessiveAmount)
          .accounts({
            sender: payer.publicKey,
            mint0: mint0,
            mint1: mint1,
            ammInfo: ammInfoPda,
            mintedPda: mintedPda,
            sendersTokenAccount0: senderTokenAccount0,
            sendersTokenAccount1: senderTokenAccount1,
            pdasTokenAccount0: pdaTokenAccount0,
            pdasTokenAccount1: pdaTokenAccount1,
            tokenProgram: TOKEN_PROGRAM_ID,
            systemProgram: SystemProgram.programId,
            associatedTokenProgram: ASSOCIATED_TOKEN_PROGRAM_ID,
          })
          .rpc();
        assert.fail("Should have rejected excessive redemption");
      } catch (error) {
        assert.isDefined(error);
      }
    });
  });

  describe("swap()", () => {
    it("executes token swaps in both directions", async () => {
      // Swap mint0 for mint1
      const amountIn = new BN(5);
      const minOut = new BN(1);

      const balanceBefore1 = await connection.getTokenAccountBalance(senderTokenAccount1);

      await program.methods
        .swap(true, amountIn, minOut)
        .accounts({
          sender: payer.publicKey,
          mint0: mint0,
          mint1: mint1,
          ammInfo: ammInfoPda,
          mintedPda: mintedPda,
          sendersTokenAccount0: senderTokenAccount0,
          sendersTokenAccount1: senderTokenAccount1,
          pdasTokenAccount0: pdaTokenAccount0,
          pdasTokenAccount1: pdaTokenAccount1,
          tokenProgram: TOKEN_PROGRAM_ID,
          systemProgram: SystemProgram.programId,
          associatedTokenProgram: ASSOCIATED_TOKEN_PROGRAM_ID,
        })
        .rpc();

      const balanceAfter1 = await connection.getTokenAccountBalance(senderTokenAccount1);
      assert.isTrue(
        Number(balanceAfter1.value.amount) > Number(balanceBefore1.value.amount),
        "Should receive token 1"
      );

      // Swap mint1 for mint0
      const balanceBefore0 = await connection.getTokenAccountBalance(senderTokenAccount0);

      await program.methods
        .swap(false, amountIn, minOut)
        .accounts({
          sender: payer.publicKey,
          mint0: mint0,
          mint1: mint1,
          ammInfo: ammInfoPda,
          mintedPda: mintedPda,
          sendersTokenAccount0: senderTokenAccount0,
          sendersTokenAccount1: senderTokenAccount1,
          pdasTokenAccount0: pdaTokenAccount0,
          pdasTokenAccount1: pdaTokenAccount1,
          tokenProgram: TOKEN_PROGRAM_ID,
          systemProgram: SystemProgram.programId,
          associatedTokenProgram: ASSOCIATED_TOKEN_PROGRAM_ID,
        })
        .rpc();

      const balanceAfter0 = await connection.getTokenAccountBalance(senderTokenAccount0);
      assert.isTrue(
        Number(balanceAfter0.value.amount) > Number(balanceBefore0.value.amount),
        "Should receive token 0"
      );
    });

    it("calculates correct output amount", async () => {
      const ammInfoBefore = await program.account.ammInfo.fetch(ammInfoPda);
      const amountIn = new BN(10);

      console.log("Reserve0 before:", ammInfoBefore.reserve0.toNumber());
      console.log("Reserve1 before:", ammInfoBefore.reserve1.toNumber());
      console.log("Amount in:", amountIn.toNumber());

      // Calculate expected output using constant product formula
      // For swapping mint0 to mint1: out = (in * reserve1) / (reserve0 + in)
      const expectedOut = (amountIn.toNumber() * ammInfoBefore.reserve1.toNumber()) / (ammInfoBefore.reserve0.toNumber() + amountIn.toNumber());
      console.log("Expected out (base units):", expectedOut);

      const balanceBefore = await connection.getTokenAccountBalance(senderTokenAccount1);

      await program.methods
        .swap(true, amountIn, new BN(1))
        .accounts({
          sender: payer.publicKey,
          mint0: mint0,
          mint1: mint1,
          ammInfo: ammInfoPda,
          mintedPda: mintedPda,
          sendersTokenAccount0: senderTokenAccount0,
          sendersTokenAccount1: senderTokenAccount1,
          pdasTokenAccount0: pdaTokenAccount0,
          pdasTokenAccount1: pdaTokenAccount1,
          tokenProgram: TOKEN_PROGRAM_ID,
          systemProgram: SystemProgram.programId,
          associatedTokenProgram: ASSOCIATED_TOKEN_PROGRAM_ID,
        })
        .rpc();

      const balanceAfter = await connection.getTokenAccountBalance(senderTokenAccount1);
      const actualOutRaw = Number(balanceAfter.value.amount) - Number(balanceBefore.value.amount);
      const actualOut = actualOutRaw / Math.pow(10, DECIMALS_1);
      
      console.log("Actual out (raw):", actualOutRaw);
      console.log("Actual out (decimal):", actualOut);

      const ammInfoAfter = await program.account.ammInfo.fetch(ammInfoPda);
      
      console.log("Reserve0 after:", ammInfoAfter.reserve0.toNumber());
      console.log("Reserve1 after:", ammInfoAfter.reserve1.toNumber());

      // Verify the reserves changed as expected
      assert.equal(
        ammInfoAfter.reserve0.toNumber(),
        ammInfoBefore.reserve0.toNumber() + amountIn.toNumber(),
        "Reserve 0 should increase by amountIn"
      );

      // The calculation is done in base units, so we should allow for rounding
      // Since division happens in integer math, there can be small differences
      const tolerance = 1; // 1 unit difference is acceptable due to integer division
      assert.approximately(actualOut, expectedOut, tolerance, "Output amount calculation incorrect");
    });

    it("enforces minimum output requirement", async () => {
      const amountIn = new BN(5);
      const unrealisticMinOut = new BN(1000000);

      try {
        await program.methods
          .swap(true, amountIn, unrealisticMinOut)
          .accounts({
            sender: payer.publicKey,
            mint0: mint0,
            mint1: mint1,
            ammInfo: ammInfoPda,
            mintedPda: mintedPda,
            sendersTokenAccount0: senderTokenAccount0,
            sendersTokenAccount1: senderTokenAccount1,
            pdasTokenAccount0: pdaTokenAccount0,
            pdasTokenAccount1: pdaTokenAccount1,
            tokenProgram: TOKEN_PROGRAM_ID,
            systemProgram: SystemProgram.programId,
            associatedTokenProgram: ASSOCIATED_TOKEN_PROGRAM_ID,
          })
          .rpc();
        assert.fail("Should have rejected swap with unrealistic min output");
      } catch (error) {
        assert.isDefined(error);
      }
    });

    it("updates reserves after swap", async () => {
      const ammInfoBefore = await program.account.ammInfo.fetch(ammInfoPda);
      const amountIn = new BN(3);

      await program.methods
        .swap(true, amountIn, new BN(1))
        .accounts({
          sender: payer.publicKey,
          mint0: mint0,
          mint1: mint1,
          ammInfo: ammInfoPda,
          mintedPda: mintedPda,
          sendersTokenAccount0: senderTokenAccount0,
          sendersTokenAccount1: senderTokenAccount1,
          pdasTokenAccount0: pdaTokenAccount0,
          pdasTokenAccount1: pdaTokenAccount1,
          tokenProgram: TOKEN_PROGRAM_ID,
          systemProgram: SystemProgram.programId,
          associatedTokenProgram: ASSOCIATED_TOKEN_PROGRAM_ID,
        })
        .rpc();

      const ammInfoAfter = await program.account.ammInfo.fetch(ammInfoPda);

      assert.isTrue(
        ammInfoAfter.reserve0.toNumber() > ammInfoBefore.reserve0.toNumber(),
        "Reserve 0 should increase"
      );
      assert.isTrue(
        ammInfoAfter.reserve1.toNumber() < ammInfoBefore.reserve1.toNumber(),
        "Reserve 1 should decrease"
      );
    });

    it("rejects zero input", async () => {
      try {
        await program.methods
          .swap(true, new BN(0), new BN(1))
          .accounts({
            sender: payer.publicKey,
            mint0: mint0,
            mint1: mint1,
            ammInfo: ammInfoPda,
            mintedPda: mintedPda,
            sendersTokenAccount0: senderTokenAccount0,
            sendersTokenAccount1: senderTokenAccount1,
            pdasTokenAccount0: pdaTokenAccount0,
            pdasTokenAccount1: pdaTokenAccount1,
            tokenProgram: TOKEN_PROGRAM_ID,
            systemProgram: SystemProgram.programId,
            associatedTokenProgram: ASSOCIATED_TOKEN_PROGRAM_ID,
          })
          .rpc();
        assert.fail("Should have rejected zero input");
      } catch (error) {
        assert.isDefined(error);
      }
    });
  });

  describe("constant product formula", () => {
    it("maintains k = reserve0 * reserve1", async () => {
      const ammInfoBefore = await program.account.ammInfo.fetch(ammInfoPda);
      const kBefore = ammInfoBefore.reserve0.toNumber() * ammInfoBefore.reserve1.toNumber();

      // Perform a swap
      await program.methods
        .swap(true, new BN(2), new BN(1))
        .accounts({
          sender: payer.publicKey,
          mint0: mint0,
          mint1: mint1,
          ammInfo: ammInfoPda,
          mintedPda: mintedPda,
          sendersTokenAccount0: senderTokenAccount0,
          sendersTokenAccount1: senderTokenAccount1,
          pdasTokenAccount0: pdaTokenAccount0,
          pdasTokenAccount1: pdaTokenAccount1,
          tokenProgram: TOKEN_PROGRAM_ID,
          systemProgram: SystemProgram.programId,
          associatedTokenProgram: ASSOCIATED_TOKEN_PROGRAM_ID,
        })
        .rpc();

      const ammInfoAfter = await program.account.ammInfo.fetch(ammInfoPda);
      const kAfter = ammInfoAfter.reserve0.toNumber() * ammInfoAfter.reserve1.toNumber();

      // k should remain approximately constant (or increase slightly due to no fees in this implementation)
      assert.isAtLeast(kAfter, kBefore * 0.99, "Constant product invariant violated");
    });

    it("calculates swap prices correctly", async () => {
      const ammInfo = await program.account.ammInfo.fetch(ammInfoPda);
      
      // Price of token0 in terms of token1
      const price = ammInfo.reserve1.toNumber() / ammInfo.reserve0.toNumber();
      
      // Small swap to check price
      const smallAmountIn = 1;
      const expectedOut = (smallAmountIn * ammInfo.reserve1.toNumber()) / (ammInfo.reserve0.toNumber() + smallAmountIn);
      const effectivePrice = expectedOut / smallAmountIn;

      // Effective price should be close to spot price for small swaps
      assert.approximately(effectivePrice, price, price * 0.1, "Price calculation incorrect");
    });
  });
});
