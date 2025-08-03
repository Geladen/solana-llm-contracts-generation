import * as anchor from '@coral-xyz/anchor';
import { Program } from '@coral-xyz/anchor';
import { BettingContract } from '../target/types/bet_deepseek';
import { assert, expect } from 'chai';
import { Keypair, PublicKey, SystemProgram, LAMPORTS_PER_SOL } from '@solana/web3.js';

describe('bet_deepseek', () => {
  const provider = anchor.AnchorProvider.env();
  anchor.setProvider(provider);

  const program = anchor.workspace.BettingContract as Program<BettingContract>;
  const admin = provider.wallet;
  
  let betStatePDA: PublicKey;
  let betStateBump: number;
  let oracle = Keypair.generate();
  let player1 = Keypair.generate();
  let player2 = Keypair.generate();
  const betAmount = LAMPORTS_PER_SOL; // 1 SOL

  async function airdrop(to: PublicKey, amount = betAmount * 2) {
    const sig = await provider.connection.requestAirdrop(to, amount);
    await provider.connection.confirmTransaction(sig);
  }

  async function resetBetState(deadlineOffset = 60) {
    // Close existing account if it exists
    try {
      await program.methods.close()
        .accounts({
          admin: admin.publicKey,
          betState: betStatePDA
        })
        .rpc();
    } catch (err) {
      console.log("No account to close");
    }

    // Initialize new state
    const deadline = new Date().getTime() / 1000 + deadlineOffset;
    [betStatePDA, betStateBump] = PublicKey.findProgramAddressSync(
      [Buffer.from("bet_state")],
      program.programId
    );

    await program.methods.initialize(oracle.publicKey, new anchor.BN(deadline))
      .accounts({
        admin: admin.publicKey,
        betState: betStatePDA,
        systemProgram: SystemProgram.programId
      })
      .rpc();
  }

  before(async () => {
    await airdrop(oracle.publicKey);
    await airdrop(player1.publicKey);
    await airdrop(player2.publicKey);
    await resetBetState();
  });

  it('should allow players to join with equal deposits', async () => {
    await program.methods.join(new anchor.BN(betAmount))
      .accounts({
        player: player1.publicKey,
        betState: betStatePDA,
        systemProgram: SystemProgram.programId
      })
      .signers([player1])
      .rpc();

    await program.methods.join(new anchor.BN(betAmount))
      .accounts({
        player: player2.publicKey,
        betState: betStatePDA,
        systemProgram: SystemProgram.programId
      })
      .signers([player2])
      .rpc();

    const betState = await program.account.betState.fetch(betStatePDA);
    assert.equal(betState.player1.toString(), player1.publicKey.toString());
    assert.equal(betState.player2.toString(), player2.publicKey.toString());
  });

  it('should prevent non-oracle from resolving winner', async () => {
    await resetBetState();

    // Join players
    await program.methods.join(new anchor.BN(betAmount))
      .accounts({
        player: player1.publicKey,
        betState: betStatePDA,
        systemProgram: SystemProgram.programId
      })
      .signers([player1])
      .rpc();

    await program.methods.join(new anchor.BN(betAmount))
      .accounts({
        player: player2.publicKey,
        betState: betStatePDA,
        systemProgram: SystemProgram.programId
      })
      .signers([player2])
      .rpc();

    // Try to resolve with wrong oracle
    const fakeOracle = Keypair.generate();
    await airdrop(fakeOracle.publicKey);

    try {
      await program.methods.win(player1.publicKey)
        .accounts({
          oracle: fakeOracle.publicKey,
          betState: betStatePDA,
          winner: player1.publicKey,
          systemProgram: SystemProgram.programId
        })
        .signers([fakeOracle])
        .rpc();
      assert.fail("Should have failed with invalid oracle");
    } catch (err) {
      expect(err.message).to.contain("InvalidOracle");
    }
  });

  // Add this to your program (lib.rs) to enable account cleanup
  /*
  pub fn close(ctx: Context<Close>) -> Result<()> {
    Ok(())
  }

  #[derive(Accounts)]
  pub struct Close<'info> {
    #[account(mut, close = admin)]
    pub bet_state: Account<'info, BetState>,
    #[account(mut)]
    pub admin: Signer<'info>,
  }
  */
});
