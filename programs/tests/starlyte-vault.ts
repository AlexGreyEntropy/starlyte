import * as anchor from "@coral-xyz/anchor";
import { Program } from "@coral-xyz/anchor";
import { StarlyteVault } from "../target/types/starlyte_vault";

describe("starlyte-vault", () => {
  anchor.setProvider(anchor.AnchorProvider.env());
  const program = anchor.workspace.StarlyteVault as Program<StarlyteVault>;

  it("Initializes vault", async () => {
    const [vaultPDA] = anchor.web3.PublicKey.findProgramAddressSync(
      [Buffer.from("vault"), program.provider.publicKey.toBuffer()],
      program.programId
    );

    await program.methods.initializeVault(new anchor.BN(1500000000))
      .accounts({
        vault: vaultPDA,
        user: program.provider.publicKey,
        systemProgram: anchor.web3.SystemProgram.programId,
      })
      .rpc();
  });
});
