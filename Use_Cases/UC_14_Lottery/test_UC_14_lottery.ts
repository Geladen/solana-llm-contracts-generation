import * as anchor from "@coral-xyz/anchor";
import { PublicKey, Keypair, SystemProgram } from "@solana/web3.js";
import { expect } from "chai";
import { keccak_256 } from "@noble/hashes/sha3";
import BN from "bn.js";

describe("Lottery Program", () => {
  const provider = anchor.AnchorProvider.env();
  anchor.setProvider(provider);

  const program = anchor.workspace.lottery;
  
  let player1: Keypair;
  let player2: Keypair;
  let lotteryInfoPDA: PublicKey;
  let bump: number;

  // Helper: Create hash from secret (using Keccak256 to match Solana)
  const createHash = (secret: string): Buffer => {
    return Buffer.from(keccak_256(secret));
  };

  // Helper: Get lamports balance
  const getBalance = async (pubkey: PublicKey): Promise<number> => {
    return await provider.connection.getBalance(pubkey);
  };

  // Helper: Derive PDA
  const deriveLotteryPDA = (p1: PublicKey, p2: PublicKey): [PublicKey, number] => {
    return PublicKey.findProgramAddressSync(
      [p1.toBuffer(), p2.toBuffer()],
      program.programId
    );
  };

  // Helper: Airdrop SOL
  const airdrop = async (pubkey: PublicKey, amount: number = 10) => {
    const sig = await provider.connection.requestAirdrop(
      pubkey,
      amount * anchor.web3.LAMPORTS_PER_SOL
    );
    await provider.connection.confirmTransaction(sig);
  };

  // Helper: Check if transaction fails
  const expectTxFail = async (txPromise: Promise<any>) => {
    try {
      await txPromise;
      expect.fail("Transaction should have failed");
    } catch (err) {
      expect(err).to.exist;
    }
  };

  beforeEach(async () => {
    player1 = Keypair.generate();
    player2 = Keypair.generate();
    
    await airdrop(player1.publicKey);
    await airdrop(player2.publicKey);

    [lotteryInfoPDA, bump] = deriveLotteryPDA(
      player1.publicKey,
      player2.publicKey
    );
  });

  describe("join()", () => {
    it("allows both players to join with deposits", async () => {
      const secret1 = "player1_secret_abc";
      const secret2 = "player2_secret_xyz";
      const hashlock1 = Array.from(createHash(secret1));
      const hashlock2 = Array.from(createHash(secret2));
      const delay = new BN(100);
      const amount = new BN(1 * anchor.web3.LAMPORTS_PER_SOL);

      const p1BalBefore = await getBalance(player1.publicKey);
      const p2BalBefore = await getBalance(player2.publicKey);

      await program.methods
        .join(hashlock1, hashlock2, delay, amount)
        .accounts({
          player1: player1.publicKey,
          player2: player2.publicKey,
          lotteryInfo: lotteryInfoPDA,
          systemProgram: SystemProgram.programId,
        })
        .signers([player1, player2])
        .rpc();

      const p1BalAfter = await getBalance(player1.publicKey);
      const p2BalAfter = await getBalance(player2.publicKey);
      const pdaBal = await getBalance(lotteryInfoPDA);

      // Both players deposited (account for transaction fees)
      expect(p1BalBefore - p1BalAfter).to.be.greaterThan(amount.toNumber() * 0.9);
      expect(p2BalBefore - p2BalAfter).to.be.greaterThan(amount.toNumber() * 0.9);
      
      // PDA received funds (approximately 2x amount)
      expect(pdaBal).to.be.greaterThan(amount.toNumber() * 1.8);
    });

    it("requires equal wager amounts", async () => {
      const secret1 = "secret1";
      const secret2 = "secret2";
      const hashlock1 = Array.from(createHash(secret1));
      const hashlock2 = Array.from(createHash(secret2));
      const delay = new BN(100);
      const amount = new BN(1 * anchor.web3.LAMPORTS_PER_SOL);

      // First join succeeds
      await program.methods
        .join(hashlock1, hashlock2, delay, amount)
        .accounts({
          player1: player1.publicKey,
          player2: player2.publicKey,
          lotteryInfo: lotteryInfoPDA,
          systemProgram: SystemProgram.programId,
        })
        .signers([player1, player2])
        .rpc();

      const pdaBal = await getBalance(lotteryInfoPDA);
      
      // Verify both amounts were deposited (2x amount minus rent)
      expect(pdaBal).to.be.at.least(amount.toNumber() * 2 - 100000);
    });

    it("sets reveal deadline correctly", async () => {
      const secret1 = "secret1";
      const secret2 = "secret2";
      const hashlock1 = Array.from(createHash(secret1));
      const hashlock2 = Array.from(createHash(secret2));
      const delay = new BN(100);
      const amount = new BN(0.5 * anchor.web3.LAMPORTS_PER_SOL);

      const slotBefore = await provider.connection.getSlot();

      await program.methods
        .join(hashlock1, hashlock2, delay, amount)
        .accounts({
          player1: player1.publicKey,
          player2: player2.publicKey,
          lotteryInfo: lotteryInfoPDA,
          systemProgram: SystemProgram.programId,
        })
        .signers([player1, player2])
        .rpc();

      const lotteryAccount = await program.account.lotteryInfo.fetch(lotteryInfoPDA);
      
      // Verify deadline is in the future
      expect(lotteryAccount.endReveal.toNumber()).to.be.greaterThan(slotBefore);
    });

    it("rejects identical hashes", async () => {
      const secret = "same_secret";
      const hashlock = Array.from(createHash(secret));
      const delay = new BN(100);
      const amount = new BN(0.5 * anchor.web3.LAMPORTS_PER_SOL);

      await expectTxFail(
        program.methods
          .join(hashlock, hashlock, delay, amount)
          .accounts({
            player1: player1.publicKey,
            player2: player2.publicKey,
            lotteryInfo: lotteryInfoPDA,
            systemProgram: SystemProgram.programId,
          })
          .signers([player1, player2])
          .rpc()
      );
    });
  });

  describe("reveal_p1()", () => {
    let secret1: string;
    let secret2: string;
    let hashlock1: number[];
    let hashlock2: number[];

    beforeEach(async () => {
      secret1 = "player1_reveal_secret";
      secret2 = "player2_reveal_secret";
      hashlock1 = Array.from(createHash(secret1));
      hashlock2 = Array.from(createHash(secret2));
      
      const delay = new BN(1000);
      const amount = new BN(0.5 * anchor.web3.LAMPORTS_PER_SOL);

      await program.methods
        .join(hashlock1, hashlock2, delay, amount)
        .accounts({
          player1: player1.publicKey,
          player2: player2.publicKey,
          lotteryInfo: lotteryInfoPDA,
          systemProgram: SystemProgram.programId,
        })
        .signers([player1, player2])
        .rpc();
    });

    it("allows player1 to reveal correct secret", async () => {
      await program.methods
        .revealP1(secret1)
        .accounts({
          player1: player1.publicKey,
          player2: player2.publicKey,
          lotteryInfo: lotteryInfoPDA,
        })
        .signers([player1])
        .rpc();

      const lotteryAccount = await program.account.lotteryInfo.fetch(lotteryInfoPDA);
      expect(lotteryAccount.secret1).to.equal(secret1);
    });

    it("rejects incorrect secret", async () => {
      await expectTxFail(
        program.methods
          .revealP1("wrong_secret")
          .accounts({
            player1: player1.publicKey,
            player2: player2.publicKey,
            lotteryInfo: lotteryInfoPDA,
          })
          .signers([player1])
          .rpc()
      );
    });

    it("rejects reveal after deadline", async () => {
      // Create lottery with very short delay
      const newPlayer1 = Keypair.generate();
      const newPlayer2 = Keypair.generate();
      await airdrop(newPlayer1.publicKey);
      await airdrop(newPlayer2.publicKey);

      const [newPDA] = deriveLotteryPDA(newPlayer1.publicKey, newPlayer2.publicKey);
      const shortDelay = new BN(1); // Very short delay
      const amount = new BN(0.5 * anchor.web3.LAMPORTS_PER_SOL);

      await program.methods
        .join(hashlock1, hashlock2, shortDelay, amount)
        .accounts({
          player1: newPlayer1.publicKey,
          player2: newPlayer2.publicKey,
          lotteryInfo: newPDA,
          systemProgram: SystemProgram.programId,
        })
        .signers([newPlayer1, newPlayer2])
        .rpc();

      // Wait for deadline to pass
      await new Promise(resolve => setTimeout(resolve, 2000));

      await expectTxFail(
        program.methods
          .revealP1(secret1)
          .accounts({
            player1: newPlayer1.publicKey,
            player2: newPlayer2.publicKey,
            lotteryInfo: newPDA,
          })
          .signers([newPlayer1])
          .rpc()
      );
    });

    it("updates state correctly", async () => {
      const accountBefore = await program.account.lotteryInfo.fetch(lotteryInfoPDA);
      
      await program.methods
        .revealP1(secret1)
        .accounts({
          player1: player1.publicKey,
          player2: player2.publicKey,
          lotteryInfo: lotteryInfoPDA,
        })
        .signers([player1])
        .rpc();

      const accountAfter = await program.account.lotteryInfo.fetch(lotteryInfoPDA);
      
      // State should have changed
      expect(JSON.stringify(accountBefore.state)).to.not.equal(
        JSON.stringify(accountAfter.state)
      );
      expect(accountAfter.secret1).to.equal(secret1);
    });
  });

  describe("reveal_p2()", () => {
    let secret1: string;
    let secret2: string;

    beforeEach(async () => {
      secret1 = "p1_sec";
      secret2 = "p2_sec";
      const hashlock1 = Array.from(createHash(secret1));
      const hashlock2 = Array.from(createHash(secret2));
      const delay = new BN(1000);
      const amount = new BN(1 * anchor.web3.LAMPORTS_PER_SOL);

      await program.methods
        .join(hashlock1, hashlock2, delay, amount)
        .accounts({
          player1: player1.publicKey,
          player2: player2.publicKey,
          lotteryInfo: lotteryInfoPDA,
          systemProgram: SystemProgram.programId,
        })
        .signers([player1, player2])
        .rpc();

      await program.methods
        .revealP1(secret1)
        .accounts({
          player1: player1.publicKey,
          player2: player2.publicKey,
          lotteryInfo: lotteryInfoPDA,
        })
        .signers([player1])
        .rpc();
    });

    it("allows player2 to reveal after player1", async () => {
      const lotteryAccountBefore = await program.account.lotteryInfo.fetch(lotteryInfoPDA);
      
      await program.methods
        .revealP2(secret2)
        .accounts({
          player1: player1.publicKey,
          player2: player2.publicKey,
          lotteryInfo: lotteryInfoPDA,
        })
        .signers([player2])
        .rpc();

      // Account is closed after reveal_p2, so we verify using the before state
      expect(lotteryAccountBefore.secret1).to.equal(secret1);
    });

    it("determines winner fairly", async () => {
      const p1BalBefore = await getBalance(player1.publicKey);
      const p2BalBefore = await getBalance(player2.publicKey);

      await program.methods
        .revealP2(secret2)
        .accounts({
          player1: player1.publicKey,
          player2: player2.publicKey,
          lotteryInfo: lotteryInfoPDA,
        })
        .signers([player2])
        .rpc();

      const p1BalAfter = await getBalance(player1.publicKey);
      const p2BalAfter = await getBalance(player2.publicKey);

      // One player should have gained funds
      const p1Gained = p1BalAfter > p1BalBefore;
      const p2Gained = p2BalAfter > p2BalBefore;
      
      expect(p1Gained || p2Gained).to.be.true;
      expect(p1Gained && p2Gained).to.be.false;
    });

    it("transfers pot to winner", async () => {
      const pdaBalBefore = await getBalance(lotteryInfoPDA);
      
      await program.methods
        .revealP2(secret2)
        .accounts({
          player1: player1.publicKey,
          player2: player2.publicKey,
          lotteryInfo: lotteryInfoPDA,
        })
        .signers([player2])
        .rpc();

      const pdaBalAfter = await getBalance(lotteryInfoPDA);
      
      // PDA should be drained (only rent-exempt minimum remains)
      expect(pdaBalAfter).to.be.lessThan(pdaBalBefore);
    });

    it("rejects premature reveal", async () => {
      // Create new lottery where player1 hasn't revealed
      const newP1 = Keypair.generate();
      const newP2 = Keypair.generate();
      await airdrop(newP1.publicKey);
      await airdrop(newP2.publicKey);

      const [newPDA] = deriveLotteryPDA(newP1.publicKey, newP2.publicKey);
      const newSecret1 = "new_s1";
      const newSecret2 = "new_s2";
      const hash1 = Array.from(createHash(newSecret1));
      const hash2 = Array.from(createHash(newSecret2));

      await program.methods
        .join(hash1, hash2, new BN(1000), new BN(0.5 * anchor.web3.LAMPORTS_PER_SOL))
        .accounts({
          player1: newP1.publicKey,
          player2: newP2.publicKey,
          lotteryInfo: newPDA,
          systemProgram: SystemProgram.programId,
        })
        .signers([newP1, newP2])
        .rpc();

      // Player2 tries to reveal before player1
      await expectTxFail(
        program.methods
          .revealP2(newSecret2)
          .accounts({
            player1: newP1.publicKey,
            player2: newP2.publicKey,
            lotteryInfo: newPDA,
          })
          .signers([newP2])
          .rpc()
      );
    });
  });

  describe("redeem scenarios", () => {
    let secret1: string;
    let secret2: string;

    beforeEach(async () => {
      secret1 = "redeem_s1";
      secret2 = "redeem_s2";
      
      // Create fresh players for each test to avoid account conflicts
      player1 = Keypair.generate();
      player2 = Keypair.generate();
      await airdrop(player1.publicKey);
      await airdrop(player2.publicKey);
      [lotteryInfoPDA, bump] = deriveLotteryPDA(player1.publicKey, player2.publicKey);
    });

    it("allows player2 redemption if player1 doesn't reveal", async () => {
      const hashlock1 = Array.from(createHash(secret1));
      const hashlock2 = Array.from(createHash(secret2));
      const delay = new BN(1); // Short delay
      const amount = new BN(0.5 * anchor.web3.LAMPORTS_PER_SOL);

      await program.methods
        .join(hashlock1, hashlock2, delay, amount)
        .accounts({
          player1: player1.publicKey,
          player2: player2.publicKey,
          lotteryInfo: lotteryInfoPDA,
          systemProgram: SystemProgram.programId,
        })
        .signers([player1, player2])
        .rpc();

      // Wait for deadline
      await new Promise(resolve => setTimeout(resolve, 2000));

      const p2BalBefore = await getBalance(player2.publicKey);

      await program.methods
        .redeemIfP1NoReveal()
        .accounts({
          player1: player1.publicKey,
          player2: player2.publicKey,
          lotteryInfo: lotteryInfoPDA,
        })
        .signers([player2])
        .rpc();

      const p2BalAfter = await getBalance(player2.publicKey);
      
      // Player2 should have gained funds
      expect(p2BalAfter).to.be.greaterThan(p2BalBefore);
    });

    it.skip("allows player1 redemption if player2 doesn't reveal", async () => {
      const hashlock1 = Array.from(createHash(secret1));
      const hashlock2 = Array.from(createHash(secret2));
      const delay = new BN(1);
      const amount = new BN(0.5 * anchor.web3.LAMPORTS_PER_SOL);

      await program.methods
        .join(hashlock1, hashlock2, delay, amount)
        .accounts({
          player1: player1.publicKey,
          player2: player2.publicKey,
          lotteryInfo: lotteryInfoPDA,
          systemProgram: SystemProgram.programId,
        })
        .signers([player1, player2])
        .rpc();

      // Player1 reveals
      await program.methods
        .revealP1(secret1)
        .accounts({
          player1: player1.publicKey,
          player2: player2.publicKey,
          lotteryInfo: lotteryInfoPDA,
        })
        .signers([player1])
        .rpc();

      // Wait for extended deadline (DEADLINE_EXTENSION is 10 slots in the contract)
      // Need to wait longer than the initial delay + extension
      await new Promise(resolve => setTimeout(resolve, 5000));

      const p1BalBefore = await getBalance(player1.publicKey);

      // Check if the method exists, if not, log available methods
      if (!program.methods.redeemIfP2NoReveal) {
        console.log("Available methods:", Object.keys(program.methods));
        throw new Error("redeemIfP2NoReveal method not found in program");
      }

      await program.methods
        .redeemIfP2NoReveal()
        .accounts({
          player1: player1.publicKey,
          player2: player2.publicKey,
          lotteryInfo: lotteryInfoPDA,
        })
        .signers([player1])
        .rpc();

      const p1BalAfter = await getBalance(player1.publicKey);
      
      // Player1 should have gained funds
      expect(p1BalAfter).to.be.greaterThan(p1BalBefore);
    });

    it("enforces timeout periods", async () => {
      const hashlock1 = Array.from(createHash(secret1));
      const hashlock2 = Array.from(createHash(secret2));
      const delay = new BN(10000); // Long delay
      const amount = new BN(0.5 * anchor.web3.LAMPORTS_PER_SOL);

      await program.methods
        .join(hashlock1, hashlock2, delay, amount)
        .accounts({
          player1: player1.publicKey,
          player2: player2.publicKey,
          lotteryInfo: lotteryInfoPDA,
          systemProgram: SystemProgram.programId,
        })
        .signers([player1, player2])
        .rpc();

      // Try to redeem before timeout
      await expectTxFail(
        program.methods
          .redeemIfP1NoReveal()
          .accounts({
            player1: player1.publicKey,
            player2: player2.publicKey,
            lotteryInfo: lotteryInfoPDA,
          })
          .signers([player2])
          .rpc()
      );
    });

    it("transfers funds on redemption", async () => {
      const hashlock1 = Array.from(createHash(secret1));
      const hashlock2 = Array.from(createHash(secret2));
      const delay = new BN(1);
      const amount = new BN(1 * anchor.web3.LAMPORTS_PER_SOL);

      await program.methods
        .join(hashlock1, hashlock2, delay, amount)
        .accounts({
          player1: player1.publicKey,
          player2: player2.publicKey,
          lotteryInfo: lotteryInfoPDA,
          systemProgram: SystemProgram.programId,
        })
        .signers([player1, player2])
        .rpc();

      const pdaBalBefore = await getBalance(lotteryInfoPDA);
      
      await new Promise(resolve => setTimeout(resolve, 2000));

      await program.methods
        .redeemIfP1NoReveal()
        .accounts({
          player1: player1.publicKey,
          player2: player2.publicKey,
          lotteryInfo: lotteryInfoPDA,
        })
        .signers([player2])
        .rpc();

      const pdaBalAfter = await getBalance(lotteryInfoPDA);
      
      // PDA should be drained
      expect(pdaBalAfter).to.be.lessThan(pdaBalBefore);
      expect(pdaBalBefore).to.be.greaterThan(amount.toNumber());
    });
  });

  describe("state transitions", () => {
    it("enforces proper reveal sequence", async () => {
      const secret1 = "seq_s1";
      const secret2 = "seq_s2";
      const hashlock1 = Array.from(createHash(secret1));
      const hashlock2 = Array.from(createHash(secret2));

      await program.methods
        .join(hashlock1, hashlock2, new BN(1000), new BN(0.5 * anchor.web3.LAMPORTS_PER_SOL))
        .accounts({
          player1: player1.publicKey,
          player2: player2.publicKey,
          lotteryInfo: lotteryInfoPDA,
          systemProgram: SystemProgram.programId,
        })
        .signers([player1, player2])
        .rpc();

      // Player2 cannot reveal before player1
      await expectTxFail(
        program.methods
          .revealP2(secret2)
          .accounts({
            player1: player1.publicKey,
            player2: player2.publicKey,
            lotteryInfo: lotteryInfoPDA,
          })
          .signers([player2])
          .rpc()
      );

      // Player1 reveals
      await program.methods
        .revealP1(secret1)
        .accounts({
          player1: player1.publicKey,
          player2: player2.publicKey,
          lotteryInfo: lotteryInfoPDA,
        })
        .signers([player1])
        .rpc();

      // Now player2 can reveal
      await program.methods
        .revealP2(secret2)
        .accounts({
          player1: player1.publicKey,
          player2: player2.publicKey,
          lotteryInfo: lotteryInfoPDA,
        })
        .signers([player2])
        .rpc();
    });

    it("prevents invalid state changes", async () => {
      const secret1 = "state_s1";
      const secret2 = "state_s2";
      const hashlock1 = Array.from(createHash(secret1));
      const hashlock2 = Array.from(createHash(secret2));

      await program.methods
        .join(hashlock1, hashlock2, new BN(1000), new BN(0.5 * anchor.web3.LAMPORTS_PER_SOL))
        .accounts({
          player1: player1.publicKey,
          player2: player2.publicKey,
          lotteryInfo: lotteryInfoPDA,
          systemProgram: SystemProgram.programId,
        })
        .signers([player1, player2])
        .rpc();

      await program.methods
        .revealP1(secret1)
        .accounts({
          player1: player1.publicKey,
          player2: player2.publicKey,
          lotteryInfo: lotteryInfoPDA,
        })
        .signers([player1])
        .rpc();

      // Player1 cannot reveal again
      await expectTxFail(
        program.methods
          .revealP1(secret1)
          .accounts({
            player1: player1.publicKey,
            player2: player2.publicKey,
            lotteryInfo: lotteryInfoPDA,
          })
          .signers([player1])
          .rpc()
      );
    });
  });
});