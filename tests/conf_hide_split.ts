import * as anchor from "@coral-xyz/anchor";
import { Program } from "@coral-xyz/anchor";
import { PublicKey, LAMPORTS_PER_SOL } from "@solana/web3.js";
import {
  TOKEN_PROGRAM_ID,
  createMint,
  createAccount,
  mintTo,
  getAccount
} from "@solana/spl-token";
import { ConfHide } from "../target/types/conf_hide";
import { randomBytes } from "crypto";
import {
  awaitComputationFinalization,
  getArciumEnv,
  getCompDefAccOffset,
  getArciumAccountBaseSeed,
  getArciumProgAddress,
  buildFinalizeCompDefTx,
  RescueCipher,
  getMXEPublicKey,
  getMXEAccAddress,
  getMempoolAccAddress,
  getCompDefAccAddress,
  getExecutingPoolAccAddress,
  getComputationAccAddress,
  getClusterAccAddress,
  x25519,
} from "@arcium-hq/client";
import * as fs from "fs";
import * as os from "os";
import { expect } from "chai";

describe("ConfHide - Privacy Trading Platform", () => {
  anchor.setProvider(anchor.AnchorProvider.env());
  const program = anchor.workspace.ConfHide as Program<ConfHide>;
  const provider = anchor.getProvider() as anchor.AnchorProvider;

  const isLocalnet = provider.connection.rpcEndpoint.includes("127.0.0.1") ||
                     provider.connection.rpcEndpoint.includes("localhost");

  const DEVNET_CLUSTER_OFFSET = 1078779259;
  const arciumEnv = isLocalnet ? getArciumEnv() : null;
  const clusterAccount = isLocalnet
    ? arciumEnv!.arciumClusterPubkey
    : getClusterAccAddress(DEVNET_CLUSTER_OFFSET);

  // Shared test state
  let payer: anchor.web3.Keypair;
  let trader1: anchor.web3.Keypair;
  let trader2: anchor.web3.Keypair;
  let baseMint: PublicKey;
  let quoteMint: PublicKey;
  let trader1BaseAccount: PublicKey;
  let trader1QuoteAccount: PublicKey;
  let trader2BaseAccount: PublicKey;
  let trader2QuoteAccount: PublicKey;
  let mxePublicKey: Uint8Array;

  function readKpJson(path: string): anchor.web3.Keypair {
    const secret = JSON.parse(fs.readFileSync(path, "utf8"));
    return anchor.web3.Keypair.fromSecretKey(Uint8Array.from(secret));
  }

  before(async () => {
    console.log("=== Test Environment Setup ===");
    console.log("RPC Endpoint:", provider.connection.rpcEndpoint);
    console.log("Is Localnet:", isLocalnet);
    console.log("Cluster Account:", clusterAccount.toBase58());
    console.log("Program ID:", program.programId.toBase58());
  });

  it("1. Setup: Fund test accounts", async () => {
    console.log("\n[Test 1] Setting up accounts");

    payer = readKpJson(`${os.homedir()}/.config/solana/id.json`);
    console.log("Loaded payer:", payer.publicKey.toBase58());

    trader1 = anchor.web3.Keypair.generate();
    trader2 = anchor.web3.Keypair.generate();
    console.log("Generated trader1:", trader1.publicKey.toBase58());
    console.log("Generated trader2:", trader2.publicKey.toBase58());

    console.log("Funding test accounts from payer...");
    const transferIx1 = anchor.web3.SystemProgram.transfer({
      fromPubkey: payer.publicKey,
      toPubkey: trader1.publicKey,
      lamports: 2 * LAMPORTS_PER_SOL,
    });
    const transferIx2 = anchor.web3.SystemProgram.transfer({
      fromPubkey: payer.publicKey,
      toPubkey: trader2.publicKey,
      lamports: 2 * LAMPORTS_PER_SOL,
    });

    const tx = new anchor.web3.Transaction().add(transferIx1, transferIx2);
    const sig = await provider.sendAndConfirm(tx, [payer]).catch(err => {
      console.error("Failed to fund accounts:", err.message);
      throw err;
    });
    console.log("Funded traders successfully. Sig:", sig);
  });

  it("2. Setup: Create token mints and accounts", async () => {
    console.log("\n[Test 2] Creating token mints");

    baseMint = await createMint(
      provider.connection,
      payer,
      payer.publicKey,
      null,
      9
    ).catch(err => {
      console.error("Failed to create base mint:", err.message);
      throw err;
    });
    console.log("Base mint created:", baseMint.toBase58());

    quoteMint = await createMint(
      provider.connection,
      payer,
      payer.publicKey,
      null,
      6
    ).catch(err => {
      console.error("Failed to create quote mint:", err.message);
      throw err;
    });
    console.log("Quote mint created:", quoteMint.toBase58());

    console.log("Creating token accounts...");
    trader1BaseAccount = await createAccount(
      provider.connection,
      payer,
      baseMint,
      trader1.publicKey
    );
    trader1QuoteAccount = await createAccount(
      provider.connection,
      payer,
      quoteMint,
      trader1.publicKey
    );
    trader2BaseAccount = await createAccount(
      provider.connection,
      payer,
      baseMint,
      trader2.publicKey
    );
    trader2QuoteAccount = await createAccount(
      provider.connection,
      payer,
      quoteMint,
      trader2.publicKey
    );
    console.log("Created all token accounts");

    console.log("Minting tokens to traders...");
    await mintTo(
      provider.connection,
      payer,
      baseMint,
      trader1BaseAccount,
      payer.publicKey,
      100_000_000_000
    );
    await mintTo(
      provider.connection,
      payer,
      quoteMint,
      trader1QuoteAccount,
      payer.publicKey,
      10_000_000_000
    );
    await mintTo(
      provider.connection,
      payer,
      baseMint,
      trader2BaseAccount,
      payer.publicKey,
      50_000_000_000
    );
    await mintTo(
      provider.connection,
      payer,
      quoteMint,
      trader2QuoteAccount,
      payer.publicKey,
      20_000_000_000
    );
    console.log("Tokens minted successfully");
  });

  it("3. Initialize computation definitions", async () => {
    console.log("\n[Test 3] Initializing computation definitions");

    mxePublicKey = await getMXEPublicKeyWithRetry(provider, program.programId);
    console.log("MXE x25519 pubkey:", mxePublicKey);

    const initOrderBookSig = await initCompDef(
      program,
      payer,
      "init_order_book",
      "initOrderBookCompDef"
    );
    console.log("Order book comp def initialized:", initOrderBookSig);

    const initSubmitOrderSig = await initCompDef(
      program,
      payer,
      "submit_order",
      "initSubmitOrderCompDef"
    );
    console.log("Submit order comp def initialized:", initSubmitOrderSig);

    const initCancelOrderSig = await initCompDef(
      program,
      payer,
      "cancel_order",
      "initCancelOrderCompDef"
    );
    console.log("Cancel order comp def initialized:", initCancelOrderSig);

    const initMatchOrdersSig = await initCompDef(
      program,
      payer,
      "match_orders",
      "initMatchOrdersCompDef"
    );
    console.log("Match orders comp def initialized:", initMatchOrdersSig);
  });

  async function initCompDef(
    program: Program<ConfHide>,
    owner: anchor.web3.Keypair,
    functionName: string,
    methodName: string
  ): Promise<string> {
    const baseSeedCompDefAcc = getArciumAccountBaseSeed(
      "ComputationDefinitionAccount"
    );
    const offset = getCompDefAccOffset(functionName);

    const compDefPDA = PublicKey.findProgramAddressSync(
      [baseSeedCompDefAcc, program.programId.toBuffer(), offset],
      getArciumProgAddress()
    )[0];

    console.log(`${functionName} CompDef PDA:`, compDefPDA.toBase58());

    try {
      await program.account.computationDefinitionAccount.fetch(compDefPDA);
      console.log(`${functionName} CompDef already initialized.`);
      return "Already Initialized";
    } catch (e) {
      // Not initialized, proceed
    }

    const sig = await program.methods[methodName]()
      .accounts({
        compDefAccount: compDefPDA,
        payer: owner.publicKey,
        mxeAccount: getMXEAccAddress(program.programId),
      })
      .signers([owner])
      .rpc({ commitment: "confirmed" });

    console.log(`Finalizing ${functionName} CompDef...`);
    const finalizeTx = await buildFinalizeCompDefTx(
      provider,
      Buffer.from(offset).readUInt32LE(),
      program.programId
    );

    const latestBlockhash = await provider.connection.getLatestBlockhash();
    finalizeTx.recentBlockhash = latestBlockhash.blockhash;
    finalizeTx.lastValidBlockHeight = latestBlockhash.lastValidBlockHeight;
    finalizeTx.sign(owner);
    await provider.sendAndConfirm(finalizeTx, [owner], {
      commitment: "confirmed",
    });
    console.log(`${functionName} CompDef finalized.`);

    return sig;
  }
});

async function getMXEPublicKeyWithRetry(
  provider: anchor.AnchorProvider,
  programId: PublicKey,
  maxRetries: number = 10,
  retryDelayMs: number = 500
): Promise<Uint8Array> {
  for (let attempt = 1; attempt <= maxRetries; attempt++) {
    try {
      const mxePublicKey = await getMXEPublicKey(provider, programId);
      if (mxePublicKey) return mxePublicKey;
    } catch (error) {
      console.log(`Attempt ${attempt} failed to fetch MXE public key:`, error);
    }

    if (attempt < maxRetries) {
      console.log(`Retrying in ${retryDelayMs}ms... (attempt ${attempt}/${maxRetries})`);
      await new Promise((resolve) => setTimeout(resolve, retryDelayMs));
    }
  }

  throw new Error(`Failed to fetch MXE public key after ${maxRetries} attempts`);
}
