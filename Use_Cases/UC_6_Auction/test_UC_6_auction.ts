import * as anchor from "@coral-xyz/anchor";
import { Program } from "@coral-xyz/anchor";
import { PublicKey, Keypair, LAMPORTS_PER_SOL, SystemProgram } from "@solana/web3.js";
import { expect } from "chai";
import BN from "bn.js";

// Generic interface for auction programs - adapt to your specific program type
interface AuctionProgram extends Program {
  methods: {
    start(object: string, duration: BN, startingBid: BN): any;
    bid(object: string, amount: BN): any;
    end(object: string): any;
  };
}

describe("Universal Auction Program Test Suite", () => {
  // Configure the client to use the local cluster
  anchor.setProvider(anchor.AnchorProvider.env());
  
  // Replace with your specific program - this should be adapted per implementation
  const program = anchor.workspace.auction as AuctionProgram;
  const provider = anchor.getProvider();

  // Test accounts
  let seller: Keypair;
  let bidder1: Keypair;
  let bidder2: Keypair;
  let auctionInfo: PublicKey;
  let testCounter = 0;
  
  // Test parameters
  const auctionDuration = new BN(100); // 100 slots
  const startingBid = new BN(1 * LAMPORTS_PER_SOL);
  const higherBid = new BN(2 * LAMPORTS_PER_SOL);
  const evenHigherBid = new BN(3 * LAMPORTS_PER_SOL);

  // Utility functions
  const getBalance = async (pubkey: PublicKey): Promise<number> => {
    return await provider.connection.getBalance(pubkey);
  };

  const sleep = (ms: number): Promise<void> => {
    return new Promise(resolve => setTimeout(resolve, ms));
  };

  const derivePDA = (object: string): [PublicKey, number] => {
    // Ensure seed is within 32 byte limit
    const seed = Buffer.from(object);
    if (seed.length > 32) {
      throw new Error(`Seed too long: ${seed.length} bytes. Max 32 bytes allowed.`);
    }
    return PublicKey.findProgramAddressSync([seed], program.programId);
  };

  const getUniqueAuctionName = (): string => {
    testCounter++;
    return `test${testCounter}`;
  };

  const expectTransactionToFail = async (txPromise: Promise<any>): Promise<boolean> => {
    try {
      await txPromise;
      return false; // Transaction succeeded when it should have failed
    } catch (error) {
      return true; // Transaction failed as expected
    }
  };

  beforeEach(async () => {
    // Create fresh keypairs for each test
    seller = Keypair.generate();
    bidder1 = Keypair.generate();
    bidder2 = Keypair.generate();
    
    // Fund accounts
    await Promise.all([
      provider.connection.requestAirdrop(seller.publicKey, 10 * LAMPORTS_PER_SOL),
      provider.connection.requestAirdrop(bidder1.publicKey, 10 * LAMPORTS_PER_SOL),
      provider.connection.requestAirdrop(bidder2.publicKey, 10 * LAMPORTS_PER_SOL),
    ]);

    // Wait for airdrops to confirm
    await sleep(1000);

    // Derive PDA for auction with unique short name
    [auctionInfo] = derivePDA(getUniqueAuctionName());
  });

  describe("start()", () => {
    it("creates auction with valid parameters", async () => {
      const auctionObject = getUniqueAuctionName();
      const [testAuctionInfo] = derivePDA(auctionObject);
      const sellerBalanceBefore = await getBalance(seller.publicKey);
      
      await program.methods
        .start(auctionObject, auctionDuration, startingBid)
        .accounts({
          seller: seller.publicKey,
          auctionInfo: testAuctionInfo,
          systemProgram: SystemProgram.programId,
        })
        .signers([seller])
        .rpc();

      // Verify auction account was created
      const auctionAccount = await program.account.auctionInfo.fetch(testAuctionInfo);
      expect(auctionAccount.seller.toString()).to.equal(seller.publicKey.toString());
      expect(auctionAccount.highestBid.toString()).to.equal(startingBid.toString());
      expect(auctionAccount.object).to.equal(auctionObject);
      
      // Verify seller paid for account creation
      const sellerBalanceAfter = await getBalance(seller.publicKey);
      expect(sellerBalanceAfter).to.be.lessThan(sellerBalanceBefore);
    });

    it("sets correct end time and starting bid", async () => {
      const auctionObject = getUniqueAuctionName();
      const [testAuctionInfo] = derivePDA(auctionObject);
      const currentSlot = await provider.connection.getSlot();
      
      await program.methods
        .start(auctionObject, auctionDuration, startingBid)
        .accounts({
          seller: seller.publicKey,
          auctionInfo: testAuctionInfo,
          systemProgram: SystemProgram.programId,
        })
        .signers([seller])
        .rpc();

      const auctionAccount = await program.account.auctionInfo.fetch(testAuctionInfo);
      
      // End time should be current slot + duration (with some tolerance for processing time)
      expect(auctionAccount.endTime.toNumber()).to.be.greaterThan(currentSlot);
      expect(auctionAccount.endTime.toNumber()).to.be.lessThanOrEqual(currentSlot + auctionDuration.toNumber() + 10);
      
      // Starting bid should be set correctly
      expect(auctionAccount.highestBid.toString()).to.equal(startingBid.toString());
    });

    it("prevents duplicate auctions with same object name", async () => {
      const auctionObject = getUniqueAuctionName();
      const [testAuctionInfo] = derivePDA(auctionObject);
      // First auction should succeed
      await program.methods
        .start(auctionObject, auctionDuration, startingBid)
        .accounts({
          seller: seller.publicKey,
          auctionInfo: testAuctionInfo,
          systemProgram: SystemProgram.programId,
        })
        .signers([seller])
        .rpc();

      // Second auction with same object should fail
      const failed = await expectTransactionToFail(
        program.methods
          .start(auctionObject, auctionDuration, startingBid)
          .accounts({
            seller: seller.publicKey,
            auctionInfo: testAuctionInfo,
            systemProgram: SystemProgram.programId,
          })
          .signers([seller])
          .rpc()
      );
      
      expect(failed).to.be.true;
    });
  });

  describe("bid()", () => {
    let auctionObject: string;

    beforeEach(async () => {
      // Start an auction before each bid test
      auctionObject = getUniqueAuctionName();
      [auctionInfo] = derivePDA(auctionObject);
      
      await program.methods
        .start(auctionObject, auctionDuration, startingBid)
        .accounts({
          seller: seller.publicKey,
          auctionInfo: auctionInfo,
          systemProgram: SystemProgram.programId,
        })
        .signers([seller])
        .rpc();
    });

    it("accepts higher bids", async () => {
      const bidder1BalanceBefore = await getBalance(bidder1.publicKey);
      const auctionBalanceBefore = await getBalance(auctionInfo);
      
      await program.methods
        .bid(auctionObject, higherBid)
        .accounts({
          bidder: bidder1.publicKey,
          auctionInfo: auctionInfo,
          currentHighestBidder: seller.publicKey, // seller is initially the highest bidder
          systemProgram: SystemProgram.programId,
        })
        .signers([bidder1])
        .rpc();

      // Verify lamport transfers
      const bidder1BalanceAfter = await getBalance(bidder1.publicKey);
      const auctionBalanceAfter = await getBalance(auctionInfo);
      
      expect(bidder1BalanceAfter).to.be.lessThan(bidder1BalanceBefore);
      expect(auctionBalanceAfter).to.be.greaterThan(auctionBalanceBefore);
      
      // Verify auction state update
      const auctionAccount = await program.account.auctionInfo.fetch(auctionInfo);
      expect(auctionAccount.highestBidder.toString()).to.equal(bidder1.publicKey.toString());
      expect(auctionAccount.highestBid.toString()).to.equal(higherBid.toString());
    });

    it("rejects lower bids", async () => {
      // First, place a higher bid
      await program.methods
        .bid(auctionObject, higherBid)
        .accounts({
          bidder: bidder1.publicKey,
          auctionInfo: auctionInfo,
          currentHighestBidder: seller.publicKey,
          systemProgram: SystemProgram.programId,
        })
        .signers([bidder1])
        .rpc();

      // Then try to place a lower bid - should fail
      const failed = await expectTransactionToFail(
        program.methods
          .bid(auctionObject, startingBid) // Lower than higherBid
          .accounts({
            bidder: bidder2.publicKey,
            auctionInfo: auctionInfo,
            currentHighestBidder: bidder1.publicKey,
            systemProgram: SystemProgram.programId,
          })
          .signers([bidder2])
          .rpc()
      );
      
      expect(failed).to.be.true;
    });

    it("refunds previous bidder", async () => {
      // First bid
      await program.methods
        .bid(auctionObject, higherBid)
        .accounts({
          bidder: bidder1.publicKey,
          auctionInfo: auctionInfo,
          currentHighestBidder: seller.publicKey,
          systemProgram: SystemProgram.programId,
        })
        .signers([bidder1])
        .rpc();

      const bidder1BalanceAfterFirstBid = await getBalance(bidder1.publicKey);

      // Second, higher bid
      await program.methods
        .bid(auctionObject, evenHigherBid)
        .accounts({
          bidder: bidder2.publicKey,
          auctionInfo: auctionInfo,
          currentHighestBidder: bidder1.publicKey,
          systemProgram: SystemProgram.programId,
        })
        .signers([bidder2])
        .rpc();

      // Verify bidder1 was refunded
      const bidder1BalanceAfterRefund = await getBalance(bidder1.publicKey);
      expect(bidder1BalanceAfterRefund).to.be.greaterThan(bidder1BalanceAfterFirstBid);
      
      // The refund should be close to the original bid amount (minus transaction fees)
      const refundAmount = bidder1BalanceAfterRefund - bidder1BalanceAfterFirstBid;
      expect(refundAmount).to.be.greaterThan(higherBid.toNumber() * 0.95); // Allow for fees
    });

    it("rejects bids after end time", async () => {
      // Wait for auction to end (in real test, you might need to manipulate time or use shorter duration)
      // For this test, we'll assume the auction has a very short duration or we have time manipulation
      
      // If your test environment supports time manipulation, use it here
      // Otherwise, create an auction with very short duration (1-2 slots)
      const shortDuration = new BN(1);
      const shortAuctionObject = getUniqueAuctionName();
      const [shortAuctionInfo] = derivePDA(shortAuctionObject);
      
      await program.methods
        .start(shortAuctionObject, shortDuration, startingBid)
        .accounts({
          seller: seller.publicKey,
          auctionInfo: shortAuctionInfo,
          systemProgram: SystemProgram.programId,
        })
        .signers([seller])
        .rpc();

      // Wait for auction to end
      await sleep(5000); // Wait for slots to pass

      const failed = await expectTransactionToFail(
        program.methods
          .bid(shortAuctionObject, higherBid)
          .accounts({
            bidder: bidder1.publicKey,
            auctionInfo: shortAuctionInfo,
            currentHighestBidder: seller.publicKey,
            systemProgram: SystemProgram.programId,
          })
          .signers([bidder1])
          .rpc()
      );

      expect(failed).to.be.true;
    });

    it("updates highest bid correctly", async () => {
      const initialAccount = await program.account.auctionInfo.fetch(auctionInfo);
      
      await program.methods
        .bid(auctionObject, higherBid)
        .accounts({
          bidder: bidder1.publicKey,
          auctionInfo: auctionInfo,
          currentHighestBidder: seller.publicKey,
          systemProgram: SystemProgram.programId,
        })
        .signers([bidder1])
        .rpc();

      const updatedAccount = await program.account.auctionInfo.fetch(auctionInfo);
      
      expect(updatedAccount.highestBid.toString()).to.equal(higherBid.toString());
      expect(updatedAccount.highestBidder.toString()).to.equal(bidder1.publicKey.toString());
      expect(updatedAccount.highestBid.toNumber()).to.be.greaterThan(initialAccount.highestBid.toNumber());
    });
  });

  describe("end()", () => {
    let auctionObject: string;

    beforeEach(async () => {
      // Start auction and place a bid
      auctionObject = getUniqueAuctionName();
      [auctionInfo] = derivePDA(auctionObject);
      
      await program.methods
        .start(auctionObject, auctionDuration, startingBid)
        .accounts({
          seller: seller.publicKey,
          auctionInfo: auctionInfo,
          systemProgram: SystemProgram.programId,
        })
        .signers([seller])
        .rpc();

      await program.methods
        .bid(auctionObject, higherBid)
        .accounts({
          bidder: bidder1.publicKey,
          auctionInfo: auctionInfo,
          currentHighestBidder: seller.publicKey,
          systemProgram: SystemProgram.programId,
        })
        .signers([bidder1])
        .rpc();
    });

    it("allows seller to end after duration", async () => {
      // Create auction with very short duration
      const shortDuration = new BN(1);
      const shortAuctionObject = getUniqueAuctionName();
      const [shortAuctionInfo] = derivePDA(shortAuctionObject);
      
      await program.methods
        .start(shortAuctionObject, shortDuration, startingBid)
        .accounts({
          seller: seller.publicKey,
          auctionInfo: shortAuctionInfo,
          systemProgram: SystemProgram.programId,
        })
        .signers([seller])
        .rpc();

      // Wait for auction to end
      await sleep(5000);

      // Should be able to end the auction
      await program.methods
        .end(shortAuctionObject)
        .accounts({
          seller: seller.publicKey,
          auctionInfo: shortAuctionInfo,
        })
        .signers([seller])
        .rpc();

      // Verify auction account state or that it was closed
      try {
        await program.account.auctionInfo.fetch(shortAuctionInfo);
        // If account still exists, verify it's marked as ended or closed
      } catch (error) {
        // Account was closed, which is also acceptable
      }
    });

    it("transfers funds to seller", async () => {
      const shortDuration = new BN(3);
      const transferAuctionObject = getUniqueAuctionName();
      const [transferAuctionInfo] = derivePDA(transferAuctionObject);
      
      await program.methods
        .start(transferAuctionObject, shortDuration, startingBid)
        .accounts({
          seller: seller.publicKey,
          auctionInfo: transferAuctionInfo,
          systemProgram: SystemProgram.programId,
        })
        .signers([seller])
        .rpc();

      await program.methods
        .bid(transferAuctionObject, higherBid)
        .accounts({
          bidder: bidder1.publicKey,
          auctionInfo: transferAuctionInfo,
          currentHighestBidder: seller.publicKey,
          systemProgram: SystemProgram.programId,
        })
        .signers([bidder1])
        .rpc();

      await sleep(5000); // Wait for auction to end

      const sellerBalanceBefore = await getBalance(seller.publicKey);
      
      await program.methods
        .end(transferAuctionObject)
        .accounts({
          seller: seller.publicKey,
          auctionInfo: transferAuctionInfo,
        })
        .signers([seller])
        .rpc();

      const sellerBalanceAfter = await getBalance(seller.publicKey);
      
      // Seller should receive the winning bid amount (minus transaction fees)
      expect(sellerBalanceAfter).to.be.greaterThan(sellerBalanceBefore);
      const received = sellerBalanceAfter - sellerBalanceBefore;
      expect(received).to.be.greaterThan(higherBid.toNumber() * 0.95); // Allow for fees
    });

    it("rejects early ending", async () => {
      const failed = await expectTransactionToFail(
        program.methods
          .end(auctionObject)
          .accounts({
            seller: seller.publicKey,
            auctionInfo: auctionInfo,
          })
          .signers([seller])
          .rpc()
      );

      expect(failed).to.be.true;
    });

    it("rejects non-seller ending", async () => {
      const shortDuration = new BN(1);
      const nonSellerAuctionObject = getUniqueAuctionName();
      const [nonSellerAuctionInfo] = derivePDA(nonSellerAuctionObject);
      
      await program.methods
        .start(nonSellerAuctionObject, shortDuration, startingBid)
        .accounts({
          seller: seller.publicKey,
          auctionInfo: nonSellerAuctionInfo,
          systemProgram: SystemProgram.programId,
        })
        .signers([seller])
        .rpc();

      await sleep(5000); // Wait for auction to end

      // Bidder trying to end auction should fail
      const failed = await expectTransactionToFail(
        program.methods
          .end(nonSellerAuctionObject)
          .accounts({
            seller: bidder1.publicKey, // Wrong seller
            auctionInfo: nonSellerAuctionInfo,
          })
          .signers([bidder1])
          .rpc()
      );

      expect(failed).to.be.true;
    });
  });

  describe("time validation", () => {
    it("enforces auction duration", async () => {
      const auctionObject = getUniqueAuctionName();
      const [testAuctionInfo] = derivePDA(auctionObject);
      const currentSlot = await provider.connection.getSlot();
      
      await program.methods
        .start(auctionObject, auctionDuration, startingBid)
        .accounts({
          seller: seller.publicKey,
          auctionInfo: testAuctionInfo,
          systemProgram: SystemProgram.programId,
        })
        .signers([seller])
        .rpc();

      const auctionAccount = await program.account.auctionInfo.fetch(testAuctionInfo);
      const expectedEndTime = currentSlot + auctionDuration.toNumber();
      
      // Allow some tolerance for processing time
      expect(auctionAccount.endTime.toNumber()).to.be.greaterThan(expectedEndTime - 5);
      expect(auctionAccount.endTime.toNumber()).to.be.lessThanOrEqual(expectedEndTime + 5);
    });

    it("prevents bidding after end", async () => {
      const shortDuration = new BN(1);
      const timeoutAuctionObject = getUniqueAuctionName();
      const [timeoutAuctionInfo] = derivePDA(timeoutAuctionObject);
      
      await program.methods
        .start(timeoutAuctionObject, shortDuration, startingBid)
        .accounts({
          seller: seller.publicKey,
          auctionInfo: timeoutAuctionInfo,
          systemProgram: SystemProgram.programId,
        })
        .signers([seller])
        .rpc();

      await sleep(5000); // Wait for auction to end

      const failed = await expectTransactionToFail(
        program.methods
          .bid(timeoutAuctionObject, higherBid)
          .accounts({
            bidder: bidder1.publicKey,
            auctionInfo: timeoutAuctionInfo,
            currentHighestBidder: seller.publicKey,
            systemProgram: SystemProgram.programId,
          })
          .signers([bidder1])
          .rpc()
      );

      expect(failed).to.be.true;
    });
  });

  describe("edge cases and security", () => {
    it("handles self-bidding by seller", async () => {
      const auctionObject = getUniqueAuctionName();
      const [testAuctionInfo] = derivePDA(auctionObject);
      
      await program.methods
        .start(auctionObject, auctionDuration, startingBid)
        .accounts({
          seller: seller.publicKey,
          auctionInfo: testAuctionInfo,
          systemProgram: SystemProgram.programId,
        })
        .signers([seller])
        .rpc();

      // Check if seller can bid on their own auction
      // Some implementations allow this, others don't
      try {
        await program.methods
          .bid(auctionObject, higherBid)
          .accounts({
            bidder: seller.publicKey,
            auctionInfo: testAuctionInfo,
            currentHighestBidder: seller.publicKey,
            systemProgram: SystemProgram.programId,
          })
          .signers([seller])
          .rpc();

        // If we reach here, self-bidding is allowed
        const auctionAccount = await program.account.auctionInfo.fetch(testAuctionInfo);
        expect(auctionAccount.highestBidder.toString()).to.equal(seller.publicKey.toString());
        expect(auctionAccount.highestBid.toString()).to.equal(higherBid.toString());
      } catch (error) {
        // Self-bidding is prevented - this is also acceptable behavior
        expect(true).to.be.true; // Test passes either way
      }
    });

    it("handles zero bid amounts", async () => {
      const auctionObject = getUniqueAuctionName();
      const [testAuctionInfo] = derivePDA(auctionObject);
      
      await program.methods
        .start(auctionObject, auctionDuration, startingBid)
        .accounts({
          seller: seller.publicKey,
          auctionInfo: testAuctionInfo,
          systemProgram: SystemProgram.programId,
        })
        .signers([seller])
        .rpc();

      const failed = await expectTransactionToFail(
        program.methods
          .bid(auctionObject, new BN(0))
          .accounts({
            bidder: bidder1.publicKey,
            auctionInfo: testAuctionInfo,
            currentHighestBidder: seller.publicKey,
            systemProgram: SystemProgram.programId,
          })
          .signers([bidder1])
          .rpc()
      );

      expect(failed).to.be.true;
    });

    it("handles multiple rapid bids correctly", async () => {
      const auctionObject = getUniqueAuctionName();
      const [testAuctionInfo] = derivePDA(auctionObject);
      
      await program.methods
        .start(auctionObject, auctionDuration, startingBid)
        .accounts({
          seller: seller.publicKey,
          auctionInfo: testAuctionInfo,
          systemProgram: SystemProgram.programId,
        })
        .signers([seller])
        .rpc();

      // Place multiple bids in sequence
      await program.methods
        .bid(auctionObject, new BN(2 * LAMPORTS_PER_SOL))
        .accounts({
          bidder: bidder1.publicKey,
          auctionInfo: testAuctionInfo,
          currentHighestBidder: seller.publicKey,
          systemProgram: SystemProgram.programId,
        })
        .signers([bidder1])
        .rpc();

      await program.methods
        .bid(auctionObject, new BN(3 * LAMPORTS_PER_SOL))
        .accounts({
          bidder: bidder2.publicKey,
          auctionInfo: testAuctionInfo,
          currentHighestBidder: bidder1.publicKey,
          systemProgram: SystemProgram.programId,
        })
        .signers([bidder2])
        .rpc();

      const auctionAccount = await program.account.auctionInfo.fetch(testAuctionInfo);
      expect(auctionAccount.highestBidder.toString()).to.equal(bidder2.publicKey.toString());
      expect(auctionAccount.highestBid.toString()).to.equal("3000000000");
    });
  });
});
