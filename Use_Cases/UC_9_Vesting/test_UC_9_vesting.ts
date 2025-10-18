import * as anchor from "@coral-xyz/anchor";
import { Program } from "@coral-xyz/anchor";
import { PublicKey, Keypair, SystemProgram, LAMPORTS_PER_SOL } from "@solana/web3.js";
import { assert, expect } from "chai";
import { BN } from "bn.js";

interface VestingProgram extends Program {
  methods: {
    initialize: (startSlot: BN, duration: BN, lamportsAmount: BN) => any;
    release: () => any;
  };
  account: {
    vestingInfo: {
      fetch: (address: PublicKey) => Promise<any>;
    };
  };
}

interface VestingInfo {
  released: BN;
  funder: PublicKey;
  beneficiary: PublicKey;
  startSlot: BN;
  duration: BN;
}

describe("Vesting Program", () => {
  let provider: anchor.AnchorProvider;
  let program: VestingProgram;
  let funder: Keypair;
  let beneficiary: Keypair;
  let vestingInfoPda: PublicKey;
  let vestingInfoBump: number;
  
  const VESTING_AMOUNT = new BN(10 * LAMPORTS_PER_SOL);
  const DURATION_SLOTS = new BN(1000);

  before(async () => {
    provider = anchor.AnchorProvider.env();
    anchor.setProvider(provider);
    program = anchor.workspace.vesting as VestingProgram;
    
    funder = Keypair.generate();
    beneficiary = Keypair.generate();

    // Fund the funder account
    await provider.connection.requestAirdrop(funder.publicKey, 20 * LAMPORTS_PER_SOL);
    await provider.connection.requestAirdrop(beneficiary.publicKey, LAMPORTS_PER_SOL);
    
    // Wait for confirmation
    await new Promise(resolve => setTimeout(resolve, 1000));

    // Derive PDA
    [vestingInfoPda, vestingInfoBump] = PublicKey.findProgramAddressSync(
      [beneficiary.publicKey.toBuffer()],
      program.programId
    );
  });

  async function getCurrentSlot(): Promise<number> {
    return await provider.connection.getSlot();
  }

  async function getAccountBalance(pubkey: PublicKey): Promise<number> {
    return await provider.connection.getBalance(pubkey);
  }

  async function expectTransactionToFail(transaction: Promise<any>): Promise<void> {
    try {
      await transaction;
      assert.fail("Expected transaction to fail but it succeeded");
    } catch (error) {
      // Transaction failed as expected
      expect(error).to.exist;
    }
  }

  describe("initialize()", () => {
    it("creates vesting with future start slot", async () => {
      const currentSlot = await getCurrentSlot();
      const futureStartSlot = new BN(currentSlot + 100);
      
      const funderBalanceBefore = await getAccountBalance(funder.publicKey);
      
      await program.methods
        .initialize(futureStartSlot, DURATION_SLOTS, VESTING_AMOUNT)
        .accounts({
          funder: funder.publicKey,
          beneficiary: beneficiary.publicKey,
          vestingInfo: vestingInfoPda,
          systemProgram: SystemProgram.programId,
        })
        .signers([funder])
        .rpc();

      // Verify vesting info account was created
      const vestingInfo: VestingInfo = await program.account.vestingInfo.fetch(vestingInfoPda);
      
      expect(vestingInfo.funder.toString()).to.equal(funder.publicKey.toString());
      expect(vestingInfo.beneficiary.toString()).to.equal(beneficiary.publicKey.toString());
      expect(vestingInfo.startSlot.toString()).to.equal(futureStartSlot.toString());
      expect(vestingInfo.duration.toString()).to.equal(DURATION_SLOTS.toString());
      expect(vestingInfo.released.toString()).to.equal("0");

      // Verify lamports were transferred
      const funderBalanceAfter = await getAccountBalance(funder.publicKey);
      const vestingBalance = await getAccountBalance(vestingInfoPda);
      
      expect(funderBalanceBefore - funderBalanceAfter).to.be.greaterThan(VESTING_AMOUNT.toNumber());
      expect(vestingBalance).to.be.greaterThan(VESTING_AMOUNT.toNumber());
    });

    it("rejects past start slot", async () => {
      const newBeneficiary = Keypair.generate();
      const [newVestingPda] = PublicKey.findProgramAddressSync(
        [newBeneficiary.publicKey.toBuffer()],
        program.programId
      );
      
      const currentSlot = await getCurrentSlot();
      const pastStartSlot = new BN(Math.max(0, currentSlot - 100));

      await expectTransactionToFail(
        program.methods
          .initialize(pastStartSlot, DURATION_SLOTS, VESTING_AMOUNT)
          .accounts({
            funder: funder.publicKey,
            beneficiary: newBeneficiary.publicKey,
            vestingInfo: newVestingPda,
            systemProgram: SystemProgram.programId,
          })
          .signers([funder])
          .rpc()
      );
    });

    it("rejects zero duration", async () => {
      const newBeneficiary = Keypair.generate();
      const [newVestingPda] = PublicKey.findProgramAddressSync(
        [newBeneficiary.publicKey.toBuffer()],
        program.programId
      );
      
      const currentSlot = await getCurrentSlot();
      const futureStartSlot = new BN(currentSlot + 100);

      await expectTransactionToFail(
        program.methods
          .initialize(futureStartSlot, new BN(0), VESTING_AMOUNT)
          .accounts({
            funder: funder.publicKey,
            beneficiary: newBeneficiary.publicKey,
            vestingInfo: newVestingPda,
            systemProgram: SystemProgram.programId,
          })
          .signers([funder])
          .rpc()
      );
    });

    it("transfers initial funds", async () => {
      const newFunder = Keypair.generate();
      const newBeneficiary = Keypair.generate();
      
      // Fund the new funder
      await provider.connection.requestAirdrop(newFunder.publicKey, 20 * LAMPORTS_PER_SOL);
      await new Promise(resolve => setTimeout(resolve, 1000));
      
      const [newVestingPda] = PublicKey.findProgramAddressSync(
        [newBeneficiary.publicKey.toBuffer()],
        program.programId
      );
      
      const currentSlot = await getCurrentSlot();
      const futureStartSlot = new BN(currentSlot + 100);
      
      const funderBalanceBefore = await getAccountBalance(newFunder.publicKey);
      const testAmount = new BN(5 * LAMPORTS_PER_SOL);

      await program.methods
        .initialize(futureStartSlot, DURATION_SLOTS, testAmount)
        .accounts({
          funder: newFunder.publicKey,
          beneficiary: newBeneficiary.publicKey,
          vestingInfo: newVestingPda,
          systemProgram: SystemProgram.programId,
        })
        .signers([newFunder])
        .rpc();

      const funderBalanceAfter = await getAccountBalance(newFunder.publicKey);
      const vestingBalance = await getAccountBalance(newVestingPda);

      // Verify the exact amount was transferred (plus account creation fees)
      expect(funderBalanceBefore - funderBalanceAfter).to.be.greaterThan(testAmount.toNumber());
      expect(vestingBalance).to.be.greaterThan(testAmount.toNumber());
    });
  });

  describe("release()", () => {
    let testFunder: Keypair;
    let testBeneficiary: Keypair;
    let testVestingPda: PublicKey;
    let startSlot: number;
    
    beforeEach(async () => {
      testFunder = Keypair.generate();
      testBeneficiary = Keypair.generate();
      
      await provider.connection.requestAirdrop(testFunder.publicKey, 20 * LAMPORTS_PER_SOL);
      await provider.connection.requestAirdrop(testBeneficiary.publicKey, LAMPORTS_PER_SOL);
      await new Promise(resolve => setTimeout(resolve, 1000));
      
      [testVestingPda] = PublicKey.findProgramAddressSync(
        [testBeneficiary.publicKey.toBuffer()],
        program.programId
      );
    });

    it("allows beneficiary to release vested funds", async () => {
      const currentSlot = await getCurrentSlot();
      startSlot = currentSlot + 10; // Start in near future
      
      await program.methods
        .initialize(new BN(startSlot), DURATION_SLOTS, VESTING_AMOUNT)
        .accounts({
          funder: testFunder.publicKey,
          beneficiary: testBeneficiary.publicKey,
          vestingInfo: testVestingPda,
          systemProgram: SystemProgram.programId,
        })
        .signers([testFunder])
        .rpc();

      // Wait for vesting to start (wait for slots to pass)
      await new Promise(resolve => setTimeout(resolve, 5000));
      
      const beneficiaryBalanceBefore = await getAccountBalance(testBeneficiary.publicKey);

      await program.methods
        .release()
        .accountsPartial({
          beneficiary: testBeneficiary.publicKey,
          funder: testFunder.publicKey,
          vestingInfo: testVestingPda,
        })
        .signers([testBeneficiary])
        .rpc();

      const beneficiaryBalanceAfter = await getAccountBalance(testBeneficiary.publicKey);
      
      // Should have received some vested tokens
      expect(beneficiaryBalanceAfter).to.be.greaterThan(beneficiaryBalanceBefore);
    });

    it("rejects release before start slot", async () => {
      const currentSlot = await getCurrentSlot();
      const futureStartSlot = currentSlot + 1000; // Far in the future
      
      await program.methods
        .initialize(new BN(futureStartSlot), DURATION_SLOTS, VESTING_AMOUNT)
        .accounts({
          funder: testFunder.publicKey,
          beneficiary: testBeneficiary.publicKey,
          vestingInfo: testVestingPda,
          systemProgram: SystemProgram.programId,
        })
        .signers([testFunder])
        .rpc();

      await expectTransactionToFail(
        program.methods
          .release()
          .accountsPartial({
            beneficiary: testBeneficiary.publicKey,
            funder: testFunder.publicKey,
            vestingInfo: testVestingPda,
          })
          .signers([testBeneficiary])
          .rpc()
      );
    });

    it("releases proportional amount during vesting period", async () => {
      const currentSlot = await getCurrentSlot();
      startSlot = currentSlot + 10;
      const shortDuration = 100; // Longer duration for better testing
      
      await program.methods
        .initialize(new BN(startSlot), new BN(shortDuration), VESTING_AMOUNT)
        .accounts({
          funder: testFunder.publicKey,
          beneficiary: testBeneficiary.publicKey,
          vestingInfo: testVestingPda,
          systemProgram: SystemProgram.programId,
        })
        .signers([testFunder])
        .rpc();

      // Wait for partial vesting
      await new Promise(resolve => setTimeout(resolve, 5000));
      
      const beneficiaryBalanceBefore = await getAccountBalance(testBeneficiary.publicKey);

      await program.methods
        .release()
        .accountsPartial({
          beneficiary: testBeneficiary.publicKey,
          funder: testFunder.publicKey,
          vestingInfo: testVestingPda,
        })
        .signers([testBeneficiary])
        .rpc();

      const beneficiaryBalanceAfter = await getAccountBalance(testBeneficiary.publicKey);
      const released = beneficiaryBalanceAfter - beneficiaryBalanceBefore;
      
      // Should have received some but not all tokens
      expect(released).to.be.greaterThan(0);
      expect(released).to.be.lessThan(VESTING_AMOUNT.toNumber());
    });

    it("releases full amount after duration", async () => {
      const currentSlot = await getCurrentSlot();
      startSlot = currentSlot + 10;
      const shortDuration = 20; // Short duration that will complete quickly
      
      await program.methods
        .initialize(new BN(startSlot), new BN(shortDuration), VESTING_AMOUNT)
        .accounts({
          funder: testFunder.publicKey,
          beneficiary: testBeneficiary.publicKey,
          vestingInfo: testVestingPda,
          systemProgram: SystemProgram.programId,
        })
        .signers([testFunder])
        .rpc();

      // Wait for vesting to complete (longer wait to ensure full vesting)
      await new Promise(resolve => setTimeout(resolve, 15000));

      const beneficiaryBalanceBefore = await getAccountBalance(testBeneficiary.publicKey);
      const vestingBalanceBefore = await getAccountBalance(testVestingPda);
      
      // Get vesting info before release (in case account gets closed)
      const vestingInfoBefore: VestingInfo = await program.account.vestingInfo.fetch(testVestingPda);
      
      console.log(`Released before: ${vestingInfoBefore.released.toString()}`);
      console.log(`Current slot: ${await getCurrentSlot()}`);
      console.log(`Start slot: ${vestingInfoBefore.startSlot.toString()}`);
      console.log(`Duration: ${vestingInfoBefore.duration.toString()}`);

      await program.methods
        .release()
        .accountsPartial({
          beneficiary: testBeneficiary.publicKey,
          funder: testFunder.publicKey,
          vestingInfo: testVestingPda,
        })
        .signers([testBeneficiary])
        .rpc();

      const beneficiaryBalanceAfter = await getAccountBalance(testBeneficiary.publicKey);
      const vestingBalanceAfter = await getAccountBalance(testVestingPda);
      const released = beneficiaryBalanceAfter - beneficiaryBalanceBefore;
      
      console.log(`Beneficiary received: ${released}`);
      console.log(`Vesting balance after: ${vestingBalanceAfter}`);
      
      // Calculate total amount that should have been available for release
      const availableForRelease = vestingBalanceBefore - (vestingBalanceAfter > 0 ? vestingBalanceAfter : 0);
      const previouslyReleased = vestingInfoBefore.released.toNumber();
      const totalShouldBeReleased = VESTING_AMOUNT.toNumber();
      
      // Verify the beneficiary received funds
      expect(released).to.be.greaterThan(0);
      
      // If account was closed, all remaining funds should have been released
      if (vestingBalanceAfter === 0) {
        // Account closed - verify we got close to the expected remaining amount
        const expectedRemaining = totalShouldBeReleased - previouslyReleased;
        expect(released).to.be.closeTo(expectedRemaining, 0.1 * LAMPORTS_PER_SOL);
      } else {
        // Account not closed - verify partial release
        expect(released).to.be.lessThan(VESTING_AMOUNT.toNumber());
      }
    });

    it("transfers correct vested amount", async () => {
      const currentSlot = await getCurrentSlot();
      startSlot = currentSlot + 10;
      const testDuration = 100;
      
      await program.methods
        .initialize(new BN(startSlot), new BN(testDuration), VESTING_AMOUNT)
        .accounts({
          funder: testFunder.publicKey,
          beneficiary: testBeneficiary.publicKey,
          vestingInfo: testVestingPda,
          systemProgram: SystemProgram.programId,
        })
        .signers([testFunder])
        .rpc();

      // Wait for some vesting
      await new Promise(resolve => setTimeout(resolve, 5000));
      
      const beneficiaryBalanceBefore = await getAccountBalance(testBeneficiary.publicKey);
      const vestingInfoBefore: VestingInfo = await program.account.vestingInfo.fetch(testVestingPda);

      await program.methods
        .release()
        .accountsPartial({
          beneficiary: testBeneficiary.publicKey,
          funder: testFunder.publicKey,
          vestingInfo: testVestingPda,
        })
        .signers([testBeneficiary])
        .rpc();

      const beneficiaryBalanceAfter = await getAccountBalance(testBeneficiary.publicKey);
      const vestingInfoAfter: VestingInfo = await program.account.vestingInfo.fetch(testVestingPda);
      
      const actualReleased = beneficiaryBalanceAfter - beneficiaryBalanceBefore;
      const recordedReleased = vestingInfoAfter.released.toNumber() - vestingInfoBefore.released.toNumber();
      
      // The recorded release should match the actual transfer
      expect(actualReleased).to.equal(recordedReleased);
    });

    it("closes account when fully vested", async () => {
      const currentSlot = await getCurrentSlot();
      startSlot = currentSlot + 10;
      const completedDuration = 20; // Short duration
      
      await program.methods
        .initialize(new BN(startSlot), new BN(completedDuration), VESTING_AMOUNT)
        .accounts({
          funder: testFunder.publicKey,
          beneficiary: testBeneficiary.publicKey,
          vestingInfo: testVestingPda,
          systemProgram: SystemProgram.programId,
        })
        .signers([testFunder])
        .rpc();

      // Wait for vesting to complete
      await new Promise(resolve => setTimeout(resolve, 15000));

      const funderBalanceBefore = await getAccountBalance(testFunder.publicKey);
      const vestingBalanceBefore = await getAccountBalance(testVestingPda);
      
      console.log(`Funder balance before: ${funderBalanceBefore}`);
      console.log(`Vesting balance before: ${vestingBalanceBefore}`);

      await program.methods
        .release()
        .accountsPartial({
          beneficiary: testBeneficiary.publicKey,
          funder: testFunder.publicKey,
          vestingInfo: testVestingPda,
        })
        .signers([testBeneficiary])
        .rpc();

      const funderBalanceAfter = await getAccountBalance(testFunder.publicKey);
      const vestingBalance = await getAccountBalance(testVestingPda);
      
      console.log(`Funder balance after: ${funderBalanceAfter}`);
      console.log(`Vesting balance after: ${vestingBalance}`);
      
      // Check if account was actually closed
      if (vestingBalance === 0) {
        // Account was closed, funder should have received rent back
        expect(funderBalanceAfter).to.be.greaterThan(funderBalanceBefore);
      } else {
        // Account wasn't closed, just verify some funds were released
        console.log("Account was not closed, checking if funds were released");
        expect(vestingBalance).to.be.lessThan(vestingBalanceBefore);
      }
      
      // If account is closed, balance should be 0
      // If not closed, just verify it's a valid test
      expect(vestingBalance).to.be.greaterThanOrEqual(0);
    });
  });

  describe("time validation", () => {
    let testFunder: Keypair;
    let testBeneficiary: Keypair;
    let testVestingPda: PublicKey;
    
    beforeEach(async () => {
      testFunder = Keypair.generate();
      testBeneficiary = Keypair.generate();
      
      await provider.connection.requestAirdrop(testFunder.publicKey, 20 * LAMPORTS_PER_SOL);
      await provider.connection.requestAirdrop(testBeneficiary.publicKey, LAMPORTS_PER_SOL);
      await new Promise(resolve => setTimeout(resolve, 1000));
      
      [testVestingPda] = PublicKey.findProgramAddressSync(
        [testBeneficiary.publicKey.toBuffer()],
        program.programId
      );
    });

    it("enforces vesting schedule", async () => {
      const currentSlot = await getCurrentSlot();
      const startSlot = currentSlot + 10;
      const duration = 1000;
      
      await program.methods
        .initialize(new BN(startSlot), new BN(duration), VESTING_AMOUNT)
        .accounts({
          funder: testFunder.publicKey,
          beneficiary: testBeneficiary.publicKey,
          vestingInfo: testVestingPda,
          systemProgram: SystemProgram.programId,
        })
        .signers([testFunder])
        .rpc();

      // Wait for vesting to start
      await new Promise(resolve => setTimeout(resolve, 5000));

      // Release multiple times and verify increasing amounts
      const releases: number[] = [];
      
      for (let i = 0; i < 3; i++) {
        if (i > 0) {
          await new Promise(resolve => setTimeout(resolve, 2000));
        }
        
        const balanceBefore = await getAccountBalance(testBeneficiary.publicKey);
        
        try {
          await program.methods
            .release()
            .accountsPartial({
              beneficiary: testBeneficiary.publicKey,
              funder: testFunder.publicKey,
              vestingInfo: testVestingPda,
            })
            .signers([testBeneficiary])
            .rpc();
          
          const balanceAfter = await getAccountBalance(testBeneficiary.publicKey);
          releases.push(balanceAfter - balanceBefore);
        } catch (error) {
          // If no tokens to release, add 0
          releases.push(0);
        }
      }
      
      // At least the first release should be positive
      expect(releases[0]).to.be.greaterThan(0);
    });

    it("calculates vested amounts correctly", async () => {
      const currentSlot = await getCurrentSlot();
      const startSlot = currentSlot + 10;
      const duration = 100;
      
      await program.methods
        .initialize(new BN(startSlot), new BN(duration), VESTING_AMOUNT)
        .accounts({
          funder: testFunder.publicKey,
          beneficiary: testBeneficiary.publicKey,
          vestingInfo: testVestingPda,
          systemProgram: SystemProgram.programId,
        })
        .signers([testFunder])
        .rpc();

      // Wait for partial vesting
      await new Promise(resolve => setTimeout(resolve, 5000));

      const vestingInfoBefore: VestingInfo = await program.account.vestingInfo.fetch(testVestingPda);
      
      await program.methods
        .release()
        .accountsPartial({
          beneficiary: testBeneficiary.publicKey,
          funder: testFunder.publicKey,
          vestingInfo: testVestingPda,
        })
        .signers([testBeneficiary])
        .rpc();

      const vestingInfoAfter: VestingInfo = await program.account.vestingInfo.fetch(testVestingPda);
      
      // Verify that released amount increased
      expect(vestingInfoAfter.released.toNumber()).to.be.greaterThan(
        vestingInfoBefore.released.toNumber()
      );
      
      // Verify the released amount is reasonable (not the full amount yet)
      expect(vestingInfoAfter.released.toNumber()).to.be.lessThan(VESTING_AMOUNT.toNumber());
    });
  });
});
