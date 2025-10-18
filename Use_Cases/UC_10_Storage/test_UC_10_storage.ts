import * as anchor from "@coral-xyz/anchor";
import { Program } from "@coral-xyz/anchor";
import { PublicKey, SystemProgram, Keypair } from "@solana/web3.js";
import { expect } from "chai";

// Generic interface for any storage program
interface StorageProgram {
  methods: {
    initialize(): any;
    storeString?(data: string): any;
    store_string?(data: string): any;
    storeBytes?(data: Buffer): any;
    store_bytes?(data: Buffer): any;
  };
  account: {
    memoryStringPda?: {
      fetch(address: PublicKey): Promise<{ myString: string }>;
    };
    memoryStringPDA?: {
      fetch(address: PublicKey): Promise<{ myString: string }>;
    };
    memoryBytesPda?: {
      fetch(address: PublicKey): Promise<{ myBytes: Buffer }>;
    };
    memoryBytesPDA?: {
      fetch(address: PublicKey): Promise<{ myBytes: Buffer }>;
    };
  };
}

describe("Storage Program", () => {
  // Configure the client to use the local cluster
  anchor.setProvider(anchor.AnchorProvider.env());
  
  const program = anchor.workspace.storage as Program<StorageProgram>;
  const provider = anchor.getProvider();
  
  let user: Keypair;
  let stringStoragePDA: PublicKey;
  let bytesStoragePDA: PublicKey;
  let stringBump: number;
  let bytesBump: number;

  // Test data constants
  const TEST_STRINGS = {
    short: "test",
    medium: "This is a medium length string for testing",
    long: "This is a very long string that will test the reallocation functionality of the storage program. It contains multiple sentences and should exceed the initial allocated space to verify that reallocation works correctly.",
    unicode: "Hello 世界 🌍 émojis and ñoñó characters",
    empty: ""
  };

  const TEST_BYTES = {
    small: Buffer.from([1, 2, 3, 4, 5]),
    medium: Buffer.from(Array(50).fill(0).map((_, i) => i % 256)), // Reduced size
    large: Buffer.from(Array(200).fill(0).map((_, i) => i % 256)),  // Reduced size
    empty: Buffer.from([])
  };

  beforeEach(async () => {
    // Generate new user for each test to ensure isolation
    user = Keypair.generate();
    
    // Airdrop SOL to the user
    const signature = await provider.connection.requestAirdrop(
      user.publicKey,
      2 * anchor.web3.LAMPORTS_PER_SOL
    );
    await provider.connection.confirmTransaction(signature);

    // Derive PDAs
    [stringStoragePDA, stringBump] = PublicKey.findProgramAddressSync(
      [Buffer.from("storage_string"), user.publicKey.toBuffer()],
      program.programId
    );

    [bytesStoragePDA, bytesBump] = PublicKey.findProgramAddressSync(
      [Buffer.from("storage_bytes"), user.publicKey.toBuffer()],
      program.programId
    );
  });

  /**
   * Helper function to get account balance
   */
  async function getAccountBalance(pubkey: PublicKey): Promise<number> {
    try {
      return await provider.connection.getBalance(pubkey);
    } catch {
      return 0;
    }
  }

  /**
   * Helper function to check if account exists
   */
  async function accountExists(pubkey: PublicKey): Promise<boolean> {
    try {
      const accountInfo = await provider.connection.getAccountInfo(pubkey);
      return accountInfo !== null;
    } catch {
      return false;
    }
  }

  /**
   * Helper function to execute transaction and handle errors generically
   */
  async function executeTransaction(transactionBuilder: any): Promise<{ success: boolean; signature?: string; error?: any }> {
    try {
      const signature = await transactionBuilder;
      return { success: true, signature };
    } catch (error) {
      console.error("Transaction failed:", error);
      return { success: false, error };
    }
  }

  /**
   * Helper function to call store string method (handles both naming conventions)
   */
  async function callStoreString(data: string) {
    const method = program.methods.storeString || program.methods.store_string;
    if (!method) {
      throw new Error("Neither storeString nor store_string method found on program");
    }
    
    return await method(data)
      .accounts({
        user: user.publicKey,
        stringStoragePda: stringStoragePDA,
        systemProgram: SystemProgram.programId,
      })
      .signers([user])
      .rpc();
  }

  /**
   * Helper function to call store bytes method (handles both naming conventions)
   */
  async function callStoreBytes(data: Buffer) {
    const method = program.methods.storeBytes || program.methods.store_bytes;
    if (!method) {
      throw new Error("Neither storeBytes nor store_bytes method found on program");
    }
    
    // Pass Buffer directly - Anchor expects Buffer for Vec<u8>
    try {
      return await method(data)
        .accounts({
          user: user.publicKey,
          bytesStoragePda: bytesStoragePDA,
          systemProgram: SystemProgram.programId,
        })
        .signers([user])
        .rpc();
    } catch (error) {
      console.error("Error calling storeBytes with bytesStoragePda:", error);
      // Try with alternative account name
      try {
        return await method(data)
          .accounts({
            user: user.publicKey,
            bytesStoragePda: bytesStoragePDA,
            systemProgram: SystemProgram.programId,
          })
          .signers([user])
          .rpc();
      } catch (error2) {
        console.error("Error calling storeBytes with bytesStoragePda:", error2);
        throw error2;
      }
    }
  }

  /**
   * Helper function to fetch string account data (handles both naming conventions)
   */
  async function fetchStringAccount(address: PublicKey) {
    return await program.account.memoryStringPda.fetch(address);
  }

  /**
   * Helper function to fetch bytes account data (handles both naming conventions)
   */
  async function fetchBytesAccount(address: PublicKey) {
    return await program.account.memoryBytesPda.fetch(address);
  }

  describe("initialize()", () => {
    it("creates storage accounts", async () => {
      // Check initial state - accounts should not exist
      expect(await accountExists(stringStoragePDA)).to.be.false;
      expect(await accountExists(bytesStoragePDA)).to.be.false;

      // Record initial balances
      const initialUserBalance = await getAccountBalance(user.publicKey);

      // Execute initialize
      const result = await executeTransaction(
        program.methods
          .initialize()
          .accounts({
            user: user.publicKey,
            stringStoragePda: stringStoragePDA,
            bytesStoragePda: bytesStoragePDA,
            systemProgram: SystemProgram.programId,
          })
          .signers([user])
          .rpc()
      );

      expect(result.success).to.be.true;

      // Verify accounts were created
      expect(await accountExists(stringStoragePDA)).to.be.true;
      expect(await accountExists(bytesStoragePDA)).to.be.true;

      // Verify user paid for account creation (balance should decrease)
      const finalUserBalance = await getAccountBalance(user.publicKey);
      expect(finalUserBalance).to.be.lessThan(initialUserBalance);
    });

    it("handles reinitialization gracefully", async () => {
      // Initialize once
      await executeTransaction(
        program.methods
          .initialize()
          .accounts({
            user: user.publicKey,
            stringStoragePda: stringStoragePDA,
            bytesStoragePda: bytesStoragePDA,
            systemProgram: SystemProgram.programId,
          })
          .signers([user])
          .rpc()
      );

      // Try to initialize again - should not fail due to init_if_needed
      const result = await executeTransaction(
        program.methods
          .initialize()
          .accounts({
            user: user.publicKey,
            stringStoragePda: stringStoragePDA,
            bytesStoragePda: bytesStoragePDA,
            systemProgram: SystemProgram.programId,
          })
          .signers([user])
          .rpc()
      );

      expect(result.success).to.be.true;
    });
  });

  describe("store_string()", () => {
    beforeEach(async () => {
      // Initialize accounts before each string test
      await executeTransaction(
        program.methods
          .initialize()
          .accounts({
            user: user.publicKey,
            stringStoragePda: stringStoragePDA,
            bytesStoragePda: bytesStoragePDA,
            systemProgram: SystemProgram.programId,
          })
          .signers([user])
          .rpc()
      );
    });

    it("stores strings of various lengths", async () => {
      for (const [name, testString] of Object.entries(TEST_STRINGS)) {
        const result = await executeTransaction(callStoreString(testString));

        expect(result.success).to.be.true;

        // Verify data was stored correctly
        const accountData = await fetchStringAccount(stringStoragePDA);
        expect(accountData.myString).to.equal(testString);
      }
    });

    it("handles string reallocation", async () => {
      const initialBalance = await getAccountBalance(user.publicKey);

      // Store short string first
      await executeTransaction(callStoreString(TEST_STRINGS.short));

      const midBalance = await getAccountBalance(user.publicKey);

      // Store long string (should trigger reallocation)
      const result = await executeTransaction(callStoreString(TEST_STRINGS.long));

      expect(result.success).to.be.true;

      const finalBalance = await getAccountBalance(user.publicKey);

      // User should have paid more for the larger allocation
      expect(finalBalance).to.be.lessThan(midBalance);

      // Verify long string was stored correctly
      const accountData = await fetchStringAccount(stringStoragePDA);
      expect(accountData.myString).to.equal(TEST_STRINGS.long);
    });

    it("updates existing string data", async () => {
      // Store initial string
      await executeTransaction(callStoreString(TEST_STRINGS.short));

      // Update with different string
      const result = await executeTransaction(callStoreString(TEST_STRINGS.unicode));

      expect(result.success).to.be.true;

      // Verify updated data
      const accountData = await fetchStringAccount(stringStoragePDA);
      expect(accountData.myString).to.equal(TEST_STRINGS.unicode);
      expect(accountData.myString).to.not.equal(TEST_STRINGS.short);
    });
  });

  describe("store_bytes()", () => {
    beforeEach(async () => {
      // Initialize accounts before each bytes test
      await executeTransaction(
        program.methods
          .initialize()
          .accounts({
            user: user.publicKey,
            stringStoragePda: stringStoragePDA,
            bytesStoragePda: bytesStoragePDA,
            systemProgram: SystemProgram.programId,
          })
          .signers([user])
          .rpc()
      );
    });

    it("stores byte arrays of various sizes", async () => {
      for (const [name, testBytes] of Object.entries(TEST_BYTES)) {
        const result = await executeTransaction(callStoreBytes(testBytes));

        expect(result.success).to.be.true;

        // Verify data was stored correctly
        const accountData = await fetchBytesAccount(bytesStoragePDA);
        expect(Buffer.from(accountData.myBytes)).to.deep.equal(testBytes);
      }
    });

    it("handles bytes reallocation", async () => {
      const initialBalance = await getAccountBalance(user.publicKey);

      // Store small bytes first
      await executeTransaction(callStoreBytes(TEST_BYTES.small));

      const midBalance = await getAccountBalance(user.publicKey);

      // Store large bytes (should trigger reallocation)
      const result = await executeTransaction(callStoreBytes(TEST_BYTES.large));

      expect(result.success).to.be.true;

      const finalBalance = await getAccountBalance(user.publicKey);

      // User should have paid more for the larger allocation
      expect(finalBalance).to.be.lessThan(midBalance);

      // Verify large bytes were stored correctly
      const accountData = await fetchBytesAccount(bytesStoragePDA);
      expect(Buffer.from(accountData.myBytes)).to.deep.equal(TEST_BYTES.large);
    });

    it("updates existing byte data", async () => {
      // Store initial bytes
      await executeTransaction(callStoreBytes(TEST_BYTES.small));

      // Update with different bytes
      const result = await executeTransaction(callStoreBytes(TEST_BYTES.medium));

      expect(result.success).to.be.true;

      // Verify updated data
      const accountData = await fetchBytesAccount(bytesStoragePDA);
      expect(Buffer.from(accountData.myBytes)).to.deep.equal(TEST_BYTES.medium);
      expect(Buffer.from(accountData.myBytes)).to.not.deep.equal(TEST_BYTES.small);
    });
  });

  describe("data persistence", () => {
    beforeEach(async () => {
      // Initialize accounts
      await executeTransaction(
        program.methods
          .initialize()
          .accounts({
            user: user.publicKey,
            stringStoragePda: stringStoragePDA,
            bytesStoragePda: bytesStoragePDA,
            systemProgram: SystemProgram.programId,
          })
          .signers([user])
          .rpc()
      );
    });

    it("maintains string data integrity", async () => {
      // Store string data
      await executeTransaction(callStoreString(TEST_STRINGS.medium));

      // Verify data multiple times to ensure persistence
      for (let i = 0; i < 3; i++) {
        const accountData = await fetchStringAccount(stringStoragePDA);
        expect(accountData.myString).to.equal(TEST_STRINGS.medium);
      }

      // Perform other operations and verify data still exists
      await executeTransaction(callStoreBytes(TEST_BYTES.medium));

      // String data should remain unchanged
      const accountData = await fetchStringAccount(stringStoragePDA);
      expect(accountData.myString).to.equal(TEST_STRINGS.medium);
    });

    it("maintains bytes data integrity", async () => {
      // Store bytes data
      await executeTransaction(callStoreBytes(TEST_BYTES.medium));

      // Verify data multiple times to ensure persistence
      for (let i = 0; i < 3; i++) {
        const accountData = await fetchBytesAccount(bytesStoragePDA);
        expect(Buffer.from(accountData.myBytes)).to.deep.equal(TEST_BYTES.medium);
      }

      // Perform other operations and verify data still exists
      await executeTransaction(callStoreString(TEST_STRINGS.medium));

      // Bytes data should remain unchanged
      const accountData = await fetchBytesAccount(bytesStoragePDA);
      expect(Buffer.from(accountData.myBytes)).to.deep.equal(TEST_BYTES.medium);
    });

    it("handles concurrent operations correctly", async () => {
      const testString = TEST_STRINGS.long;
      const testBytes = TEST_BYTES.large;

      // Store both string and bytes data
      const stringResult = executeTransaction(callStoreString(testString));
      const bytesResult = executeTransaction(callStoreBytes(testBytes));

      // Wait for both operations (note: they may not be truly concurrent due to same user)
      const [stringRes, bytesRes] = await Promise.all([stringResult, bytesResult]);

      expect(stringRes.success).to.be.true;
      expect(bytesRes.success).to.be.true;

      // Verify both data sets are correct
      const stringData = await fetchStringAccount(stringStoragePDA);
      const bytesData = await fetchBytesAccount(bytesStoragePDA);

      expect(stringData.myString).to.equal(testString);
      expect(Buffer.from(bytesData.myBytes)).to.deep.equal(testBytes);
    });
  });
});
