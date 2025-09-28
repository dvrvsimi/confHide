import * as anchor from "@coral-xyz/anchor";
import { Program } from "@coral-xyz/anchor";
import { PublicKey, SystemProgram, LAMPORTS_PER_SOL } from "@solana/web3.js";
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
  uploadCircuit,
  buildFinalizeCompDefTx,
  RescueCipher,
  deserializeLE,
  getMXEPublicKey,
  getMXEAccAddress,
  getMempoolAccAddress,
  getCompDefAccAddress,
  getExecutingPoolAccAddress,
  getComputationAccAddress,
  x25519,
} from "@arcium-hq/client";
import * as fs from "fs";
import * as os from "os";
import { expect } from "chai";

describe("ConfHide - Privacy Trading Platform", () => {
  // Configure the client to use the local cluster
  anchor.setProvider(anchor.AnchorProvider.env());
  const program = anchor.workspace.ConfHide as Program<ConfHide>;
  const provider = anchor.getProvider() as anchor.AnchorProvider;

  // Test keypairs
  const payer = readKpJson(`${os.homedir()}/.config/solana/id.json`);
  const trader1 = anchor.web3.Keypair.generate();
  const trader2 = anchor.web3.Keypair.generate();

  // Token accounts
  let baseMint: PublicKey; // SOL-like token
  let quoteMint: PublicKey; // USDC-like token
  let trader1BaseAccount: PublicKey;
  let trader1QuoteAccount: PublicKey;
  let trader2BaseAccount: PublicKey;
  let trader2QuoteAccount: PublicKey;

  // Trading pair
  const tradingPairId = new anchor.BN(1);
  let tradingPairPDA: PublicKey;

  // Arcium setup
  const arciumEnv = getArciumEnv();
  let mxePublicKey: Uint8Array;
  let cipher: RescueCipher;

  type Event = anchor.IdlEvents<(typeof program)["idl"]>;
  const awaitEvent = async <E extends keyof Event>(
    eventName: E
  ): Promise<Event[E]> => {
    let listenerId: number;
    const event = await new Promise<Event[E]>((res) => {
      listenerId = program.addEventListener(eventName, (event) => {
        res(event);
      });
    });
    await program.removeEventListener(listenerId);
    return event;
  };

  before(async () => {
    console.log("🔄 Setting up test environment...");

    // Airdrop SOL to test accounts
    await provider.connection.requestAirdrop(trader1.publicKey, 2 * LAMPORTS_PER_SOL);
    await provider.connection.requestAirdrop(trader2.publicKey, 2 * LAMPORTS_PER_SOL);

    // Wait for airdrop confirmations
    await new Promise(resolve => setTimeout(resolve, 2000));

    // Create token mints
    baseMint = await createMint(
      provider.connection,
      payer,
      payer.publicKey,
      null,
      9 // 9 decimals like SOL
    );

    quoteMint = await createMint(
      provider.connection,
      payer,
      payer.publicKey,
      null,
      6 // 6 decimals like USDC
    );

    // Create token accounts for traders
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

    // Mint tokens to traders
    // Trader 1: 100 base tokens, 10,000 quote tokens
    await mintTo(
      provider.connection,
      payer,
      baseMint,
      trader1BaseAccount,
      payer.publicKey,
      100_000_000_000 // 100 tokens with 9 decimals
    );

    await mintTo(
      provider.connection,
      payer,
      quoteMint,
      trader1QuoteAccount,
      payer.publicKey,
      10_000_000_000 // 10,000 tokens with 6 decimals
    );

    // Trader 2: 50 base tokens, 20,000 quote tokens
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

    // Setup Arcium MPC
    mxePublicKey = await getMXEPublicKeyWithRetry(provider, program.programId);
    const privateKey = x25519.utils.randomSecretKey();
    const publicKey = x25519.getPublicKey(privateKey);
    const sharedSecret = x25519.getSharedSecret(privateKey, mxePublicKey);
    cipher = new RescueCipher(sharedSecret);

    // Trading pair PDA
    [tradingPairPDA] = PublicKey.findProgramAddressSync(
      [Buffer.from("trading_pair"), tradingPairId.toArrayLike(Buffer, "le", 8)],
      program.programId
    );

    console.log("✅ Test environment setup complete");
  });

  describe("🏗️ Computation Definition Setup", () => {
    it("Initialize order book computation definition", async () => {
      console.log("Initializing order book comp def...");
      const sig = await initCompDef(program, payer, "init_order_book");
      console.log("✅ Order book comp def initialized:", sig);
    });

    it("Initialize submit order computation definition", async () => {
      console.log("Initializing submit order comp def...");
      const sig = await initCompDef(program, payer, "submit_order");
      console.log("✅ Submit order comp def initialized:", sig);
    });

    it("Initialize match orders computation definition", async () => {
      console.log("Initializing match orders comp def...");
      const sig = await initCompDef(program, payer, "match_orders");
      console.log("✅ Match orders comp def initialized:", sig);
    });
  });

  describe("🏪 Trading Pair Management", () => {
    it("Initialize SOL/USDC trading pair", async () => {
      console.log("Creating SOL/USDC trading pair...");

      const computationOffset = new anchor.BN(randomBytes(8), "hex");
      const mxeNonce = new anchor.BN(randomBytes(16), "hex");

      const initEventPromise = awaitEvent("tradingPairInitializedEvent");

      const sig = await program.methods
        .initializeTradingPair(computationOffset, tradingPairId, mxeNonce)
        .accounts({
          tradingPair: tradingPairPDA,
          baseMint: baseMint,
          quoteMint: quoteMint,
          computationAccount: getComputationAccAddress(
            program.programId,
            computationOffset
          ),
          clusterAccount: arciumEnv.arciumClusterPubkey,
          mxeAccount: getMXEAccAddress(program.programId),
          mempoolAccount: getMempoolAccAddress(program.programId),
          executingPool: getExecutingPoolAccAddress(program.programId),
          compDefAccount: getCompDefAccAddress(
            program.programId,
            Buffer.from(getCompDefAccOffset("init_order_book")).readUInt32LE()
          ),
        })
        .signers([payer])
        .rpc({ skipPreflight: true, commitment: "confirmed" });

      console.log("Queue sig:", sig);

      // Wait for MPC computation
      const finalizeSig = await awaitComputationFinalization(
        provider,
        computationOffset,
        program.programId,
        "confirmed"
      );
      console.log("Finalize sig:", finalizeSig);

      const initEvent = await initEventPromise;
      expect(initEvent.tradingPairId.toString()).to.equal(tradingPairId.toString());

      console.log("✅ Trading pair created with encrypted order book");
    });

    it("Verify trading pair state", async () => {
      const tradingPair = await program.account.tradingPair.fetch(tradingPairPDA);

      expect(tradingPair.tradingPairId.toString()).to.equal(tradingPairId.toString());
      expect(tradingPair.baseMint.toString()).to.equal(baseMint.toString());
      expect(tradingPair.quoteMint.toString()).to.equal(quoteMint.toString());
      expect(tradingPair.isActive).to.be.true;
      expect(tradingPair.totalOrders.toString()).to.equal("0");

      console.log("✅ Trading pair state verified");
    });
  });

  describe("📝 Order Submission", () => {
    it("Submit buy order (encrypted)", async () => {
      console.log("Submitting encrypted buy order...");

      const computationOffset = new anchor.BN(randomBytes(8), "hex");
      const clientNonce = new anchor.BN(randomBytes(16), "hex");
      const clientKeyPair = x25519.generateKeyPair();
      const clientPubkey = Array.from(clientKeyPair.publicKey);

      // Order: Buy 10 tokens at 100 USDC each
      const price = new anchor.BN(100_000_000); // 100 USDC with 6 decimals
      const quantity = new anchor.BN(10_000_000_000); // 10 tokens with 9 decimals
      const isBuy = true;

      // Create shared secret with MXE
      const mxePublicKey = getMXEPublicKey(program.programId);
      const sharedSecret = x25519.computeSharedSecret(clientKeyPair.secretKey, mxePublicKey);

      // Encrypt order data using RescueCipher
      const cipher = new RescueCipher(sharedSecret, clientNonce.toBuffer());
      const encryptedPrice = cipher.encrypt(price.toBuffer("le", 8));
      const encryptedQuantity = cipher.encrypt(quantity.toBuffer("le", 8));
      const encryptedIsBuy = cipher.encrypt(Buffer.from([isBuy ? 1 : 0]));
      const traderId = new anchor.BN(payer.publicKey.toBuffer()).shln(96); // Use public key as trader ID
      const encryptedTraderId = cipher.encrypt(traderId.toBuffer("le", 16));

      const orderEventPromise = awaitEvent("orderSubmittedEvent");

      const sig = await program.methods
        .submitOrder(
          computationOffset,
          tradingPairId,
          clientPubkey,
          clientNonce,
          Array.from(encryptedPrice),
          Array.from(encryptedQuantity),
          Array.from(encryptedIsBuy),
          Array.from(encryptedTraderId)
        )
        .accounts({
          tradingPair: tradingPairPDA,
          computationAccount: getComputationAccAddress(
            program.programId,
            computationOffset
          ),
          clusterAccount: arciumEnv.arciumClusterPubkey,
          mxeAccount: getMXEAccAddress(program.programId),
          mempoolAccount: getMempoolAccAddress(program.programId),
          executingPool: getExecutingPoolAccAddress(program.programId),
          compDefAccount: getCompDefAccAddress(
            program.programId,
            Buffer.from(getCompDefAccOffset("submit_order")).readUInt32LE()
          ),
        })
        .signers([payer])
        .rpc({ skipPreflight: true, commitment: "confirmed" });

      console.log("Submit order sig:", sig);

      // Wait for MPC computation
      const finalizeSig = await awaitComputationFinalization(
        provider,
        computationOffset,
        program.programId,
        "confirmed"
      );
      console.log("Order finalize sig:", finalizeSig);

      const orderEvent = await orderEventPromise;
      expect(orderEvent.tradingPairId.toString()).to.equal(tradingPairId.toString());
      expect(orderEvent.totalOrders.toString()).to.equal("1");

      console.log("✅ Buy order submitted and encrypted");
    });

    it("Submit sell order (encrypted)", async () => {
      console.log("Submitting encrypted sell order...");

      const computationOffset = new anchor.BN(randomBytes(8), "hex");
      const clientNonce = new anchor.BN(randomBytes(16), "hex");
      const clientKeyPair = x25519.generateKeyPair();
      const clientPubkey = Array.from(clientKeyPair.publicKey);

      // Order: Sell 5 tokens at 95 USDC each (lower price, should match)
      const price = new anchor.BN(95_000_000); // 95 USDC with 6 decimals
      const quantity = new anchor.BN(5_000_000_000); // 5 tokens with 9 decimals
      const isBuy = false;

      // Create shared secret with MXE
      const mxePublicKey = getMXEPublicKey(program.programId);
      const sharedSecret = x25519.computeSharedSecret(clientKeyPair.secretKey, mxePublicKey);

      // Encrypt order data using RescueCipher
      const cipher = new RescueCipher(sharedSecret, clientNonce.toBuffer());
      const encryptedPrice = cipher.encrypt(price.toBuffer("le", 8));
      const encryptedQuantity = cipher.encrypt(quantity.toBuffer("le", 8));
      const encryptedIsBuy = cipher.encrypt(Buffer.from([isBuy ? 1 : 0]));
      const traderId = new anchor.BN(payer.publicKey.toBuffer()).shln(96); // Use public key as trader ID
      const encryptedTraderId = cipher.encrypt(traderId.toBuffer("le", 16));

      const orderEventPromise = awaitEvent("orderSubmittedEvent");

      const sig = await program.methods
        .submitOrder(
          computationOffset,
          tradingPairId,
          clientPubkey,
          clientNonce,
          Array.from(encryptedPrice),
          Array.from(encryptedQuantity),
          Array.from(encryptedIsBuy),
          Array.from(encryptedTraderId)
        )
        .accounts({
          tradingPair: tradingPairPDA,
          computationAccount: getComputationAccAddress(
            program.programId,
            computationOffset
          ),
          clusterAccount: arciumEnv.arciumClusterPubkey,
          mxeAccount: getMXEAccAddress(program.programId),
          mempoolAccount: getMempoolAccAddress(program.programId),
          executingPool: getExecutingPoolAccAddress(program.programId),
          compDefAccount: getCompDefAccAddress(
            program.programId,
            Buffer.from(getCompDefAccOffset("submit_order")).readUInt32LE()
          ),
        })
        .signers([payer])
        .rpc({ skipPreflight: true, commitment: "confirmed" });

      console.log("Submit sell order sig:", sig);

      const finalizeSig = await awaitComputationFinalization(
        provider,
        computationOffset,
        program.programId,
        "confirmed"
      );
      console.log("Sell order finalize sig:", finalizeSig);

      const orderEvent = await orderEventPromise;
      expect(orderEvent.totalOrders.toString()).to.equal("2");

      console.log("✅ Sell order submitted and encrypted");
    });
  });

  describe("🔄 Order Matching", () => {
    it("Execute batch matching (private)", async () => {
      console.log("Triggering private order matching...");

      const computationOffset = new anchor.BN(randomBytes(8), "hex");

      const matchEventPromise = awaitEvent("ordersMatchedEvent");

      const sig = await program.methods
        .matchOrders(computationOffset, tradingPairId)
        .accounts({
          tradingPair: tradingPairPDA,
          computationAccount: getComputationAccAddress(
            program.programId,
            computationOffset
          ),
          clusterAccount: arciumEnv.arciumClusterPubkey,
          mxeAccount: getMXEAccAddress(program.programId),
          mempoolAccount: getMempoolAccAddress(program.programId),
          executingPool: getExecutingPoolAccAddress(program.programId),
          compDefAccount: getCompDefAccAddress(
            program.programId,
            Buffer.from(getCompDefAccOffset("match_orders")).readUInt32LE()
          ),
        })
        .signers([payer])
        .rpc({ skipPreflight: true, commitment: "confirmed" });

      console.log("Match orders sig:", sig);

      const finalizeSig = await awaitComputationFinalization(
        provider,
        computationOffset,
        program.programId,
        "confirmed"
      );
      console.log("Match finalize sig:", finalizeSig);

      const matchEvent = await matchEventPromise;
      expect(matchEvent.tradingPairId.toString()).to.equal(tradingPairId.toString());

      console.log("✅ Orders matched privately - trades revealed");
    });
  });

  describe("💰 Trade Execution", () => {
    it("Execute token transfers for matched trades", async () => {
      console.log("Executing token transfers for matched trades...");

      // Mock trade data (in real implementation, this would come from MPC results)
      const buyerId = new anchor.BN(1);
      const sellerId = new anchor.BN(2);
      const tradePrice = new anchor.BN(95_000_000); // 95 USDC
      const tradeQuantity = new anchor.BN(5_000_000_000); // 5 tokens

      const tradeEventPromise = awaitEvent("tradeExecutedEvent");

      const sig = await program.methods
        .executeTrade(buyerId, sellerId, tradePrice, tradeQuantity)
        .accounts({
          buyer: trader1.publicKey,
          seller: trader2.publicKey,
          buyerBaseAccount: trader1BaseAccount,
          buyerQuoteAccount: trader1QuoteAccount,
          sellerBaseAccount: trader2BaseAccount,
          sellerQuoteAccount: trader2QuoteAccount,
          tokenProgram: TOKEN_PROGRAM_ID,
        })
        .signers([trader1, trader2])
        .rpc({ commitment: "confirmed" });

      console.log("Execute trade sig:", sig);

      const tradeEvent = await tradeEventPromise;
      expect(tradeEvent.buyerId.toString()).to.equal(buyerId.toString());
      expect(tradeEvent.sellerId.toString()).to.equal(sellerId.toString());
      expect(tradeEvent.price.toString()).to.equal(tradePrice.toString());
      expect(tradeEvent.quantity.toString()).to.equal(tradeQuantity.toString());

      console.log("✅ Trade executed with token transfers");
    });

    it("Verify final token balances", async () => {
      const trader1Base = await getAccount(provider.connection, trader1BaseAccount);
      const trader1Quote = await getAccount(provider.connection, trader1QuoteAccount);
      const trader2Base = await getAccount(provider.connection, trader2BaseAccount);
      const trader2Quote = await getAccount(provider.connection, trader2QuoteAccount);

      console.log("Final balances:");
      console.log("Trader 1 Base:", trader1Base.amount.toString());
      console.log("Trader 1 Quote:", trader1Quote.amount.toString());
      console.log("Trader 2 Base:", trader2Base.amount.toString());
      console.log("Trader 2 Quote:", trader2Quote.amount.toString());

      // Trader 1 should have more base tokens, less quote tokens
      expect(Number(trader1Base.amount)).to.be.greaterThan(100_000_000_000);
      expect(Number(trader1Quote.amount)).to.be.lessThan(10_000_000_000);

      // Trader 2 should have less base tokens, more quote tokens
      expect(Number(trader2Base.amount)).to.be.lessThan(50_000_000_000);
      expect(Number(trader2Quote.amount)).to.be.greaterThan(20_000_000_000);

      console.log("✅ Token balances updated correctly");
    });
  });

  // Helper functions
  async function initCompDef(
    program: Program<ConfHide>,
    owner: anchor.web3.Keypair,
    functionName: string
  ): Promise<string> {
    const baseSeedCompDefAcc = getArciumAccountBaseSeed("ComputationDefinitionAccount");
    const offset = getCompDefAccOffset(functionName);

    const compDefPDA = PublicKey.findProgramAddressSync(
      [baseSeedCompDefAcc, program.programId.toBuffer(), offset],
      getArciumProgAddress()
    )[0];

    const methodName = `init${functionName.charAt(0).toUpperCase() + functionName.slice(1).replace(/_([a-z])/g, (_, letter) => letter.toUpperCase())}CompDef`;

    const sig = await program.methods[methodName]()
      .accounts({
        compDefAccount: compDefPDA,
        payer: owner.publicKey,
        mxeAccount: getMXEAccAddress(program.programId),
      })
      .signers([owner])
      .rpc({ commitment: "confirmed" });

    const finalizeTx = await buildFinalizeCompDefTx(
      provider,
      Buffer.from(offset).readUInt32LE(),
      program.programId
    );

    const latestBlockhash = await provider.connection.getLatestBlockhash();
    finalizeTx.recentBlockhash = latestBlockhash.blockhash;
    finalizeTx.lastValidBlockHeight = latestBlockhash.lastValidBlockHeight;
    finalizeTx.sign(owner);
    await provider.sendAndConfirm(finalizeTx);

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

function readKpJson(path: string): anchor.web3.Keypair {
  const file = fs.readFileSync(path);
  return anchor.web3.Keypair.fromSecretKey(new Uint8Array(JSON.parse(file.toString())));
}