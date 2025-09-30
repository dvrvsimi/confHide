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

  const arciumEnv = getArciumEnv();

  it("can submit and match orders privately!", async () => {
    const payer = readKpJson(`${os.homedir()}/.config/solana/id.json`);
    const trader1 = anchor.web3.Keypair.generate();
    const trader2 = anchor.web3.Keypair.generate();

    // Airdrop SOL to test accounts
    console.log("Setting up test accounts...");
    await provider.connection.requestAirdrop(trader1.publicKey, 2 * LAMPORTS_PER_SOL);
    await provider.connection.requestAirdrop(trader2.publicKey, 2 * LAMPORTS_PER_SOL);
    await new Promise(resolve => setTimeout(resolve, 2000));

    // Create token mints
    const baseMint = await createMint(
      provider.connection,
      payer,
      payer.publicKey,
      null,
      9 // 9 decimals like SOL
    );

    const quoteMint = await createMint(
      provider.connection,
      payer,
      payer.publicKey,
      null,
      6 // 6 decimals like USDC
    );

    // Create token accounts for traders
    const trader1BaseAccount = await createAccount(
      provider.connection,
      payer,
      baseMint,
      trader1.publicKey
    );

    const trader1QuoteAccount = await createAccount(
      provider.connection,
      payer,
      quoteMint,
      trader1.publicKey
    );

    const trader2BaseAccount = await createAccount(
      provider.connection,
      payer,
      baseMint,
      trader2.publicKey
    );

    const trader2QuoteAccount = await createAccount(
      provider.connection,
      payer,
      quoteMint,
      trader2.publicKey
    );

    // Mint tokens to traders
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

    const mxePublicKey = await getMXEPublicKeyWithRetry(provider, program.programId);
    console.log("MXE x25519 pubkey is", mxePublicKey);

    // Initialize computation definitions
    console.log("Initializing computation definitions...");
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

    // Initialize trading pair
    console.log("Creating SOL/USDC trading pair...");
    const tradingPairId = new anchor.BN(1);
    const [tradingPairPDA] = PublicKey.findProgramAddressSync(
      [Buffer.from("trading_pair"), tradingPairId.toArrayLike(Buffer, "le", 8)],
      program.programId
    );

    const pairComputationOffset = new anchor.BN(randomBytes(8), "hex");
    const mxeNonce = new anchor.BN(randomBytes(16), "hex");

    const initEventPromise = awaitEvent("tradingPairInitializedEvent");

    const pairSig = await program.methods
      .initializeTradingPair(pairComputationOffset, tradingPairId, mxeNonce)
      .accountsPartial({
        tradingPair: tradingPairPDA,
        baseMint: baseMint,
        quoteMint: quoteMint,
        computationAccount: getComputationAccAddress(
          program.programId,
          pairComputationOffset
        ),
        clusterAccount: arciumEnv.arciumClusterPubkey,
        mxeAccount: getMXEAccAddress(program.programId),
        mempoolAccount: getMempoolAccAddress(program.programId),
        executingPool: getExecutingPoolAccAddress(program.programId),
        compDefAccount: getCompDefAccAddress(
          program.programId,
          Buffer.from(getCompDefAccOffset("init_order_book")).readUInt32LE()
        ),
        payer: payer.publicKey,
      })
      .signers([payer])
      .rpc({ skipPreflight: true, commitment: "confirmed" });

    console.log("Queue sig:", pairSig);

    const finalizePairSig = await awaitComputationFinalization(
      provider,
      pairComputationOffset,
      program.programId,
      "confirmed"
    );
    console.log("Finalize sig:", finalizePairSig);

    const initEvent = await initEventPromise;
    expect(initEvent.tradingPairId.toString()).to.equal(tradingPairId.toString());
    console.log("✅ Trading pair created with encrypted order book");

    // Submit buy order
    console.log("Submitting encrypted buy order...");
    const buyComputationOffset = new anchor.BN(randomBytes(8), "hex");
    const buyClientNonce = randomBytes(16);
    const buyPrivateKey = x25519.utils.randomSecretKey();
    const buyPublicKey = x25519.getPublicKey(buyPrivateKey);
    const buyClientPubkey = Array.from(buyPublicKey);

    const buyPrice = new anchor.BN(100_000_000); // 100 USDC
    const buyQuantity = new anchor.BN(10_000_000_000); // 10 tokens
    const isBuy = true;

    const buySharedSecret = x25519.getSharedSecret(buyPrivateKey, mxePublicKey);
    const buyCipher = new RescueCipher(buySharedSecret);

    const plaintext = [
      BigInt(buyPrice.toString()),
      BigInt(buyQuantity.toString()),
      BigInt(isBuy ? 1 : 0),
      BigInt(new anchor.BN(payer.publicKey.toBuffer()).shln(96).toString()),
    ];
    const ciphertext = buyCipher.encrypt(plaintext, buyClientNonce);

    const buyOrderEventPromise = awaitEvent("orderSubmittedEvent");

    const buySig = await program.methods
      .submitOrder(
        buyComputationOffset,
        tradingPairId,
        buyClientPubkey,
        new anchor.BN(Buffer.from(buyClientNonce).toString('hex'), 'hex'),
        Array.from(ciphertext[0]),
        Array.from(ciphertext[1]),
        Array.from(ciphertext[2]),
        Array.from(ciphertext[3])
      )
      .accountsPartial({
        tradingPair: tradingPairPDA,
        computationAccount: getComputationAccAddress(
          program.programId,
          buyComputationOffset
        ),
        clusterAccount: arciumEnv.arciumClusterPubkey,
        mxeAccount: getMXEAccAddress(program.programId),
        mempoolAccount: getMempoolAccAddress(program.programId),
        executingPool: getExecutingPoolAccAddress(program.programId),
        compDefAccount: getCompDefAccAddress(
          program.programId,
          Buffer.from(getCompDefAccOffset("submit_order")).readUInt32LE()
        ),
        payer: payer.publicKey,
      })
      .signers([payer])
      .rpc({ skipPreflight: true, commitment: "confirmed" });

    console.log("Submit buy order sig:", buySig);

    const buyFinalizeSig = await awaitComputationFinalization(
      provider,
      buyComputationOffset,
      program.programId,
      "confirmed"
    );
    console.log("Buy order finalize sig:", buyFinalizeSig);

    const buyOrderEvent = await buyOrderEventPromise;
    expect(buyOrderEvent.tradingPairId.toString()).to.equal(tradingPairId.toString());
    expect(buyOrderEvent.totalOrders.toString()).to.equal("1");
    console.log("✅ Buy order submitted and encrypted");

    // Submit sell order
    console.log("Submitting encrypted sell order...");
    const sellComputationOffset = new anchor.BN(randomBytes(8), "hex");
    const sellClientNonce = randomBytes(16);
    const sellPrivateKey = x25519.utils.randomSecretKey();
    const sellPublicKey = x25519.getPublicKey(sellPrivateKey);
    const sellClientPubkey = Array.from(sellPublicKey);

    const sellPrice = new anchor.BN(95_000_000); // 95 USDC
    const sellQuantity = new anchor.BN(5_000_000_000); // 5 tokens
    const isSell = false;

    const sellSharedSecret = x25519.getSharedSecret(sellPrivateKey, mxePublicKey);
    const sellCipher = new RescueCipher(sellSharedSecret);

    const sellPlaintext = [
      BigInt(sellPrice.toString()),
      BigInt(sellQuantity.toString()),
      BigInt(isSell ? 1 : 0),
      BigInt(new anchor.BN(payer.publicKey.toBuffer()).shln(96).toString()),
    ];
    const sellCiphertext = sellCipher.encrypt(sellPlaintext, sellClientNonce);

    const sellOrderEventPromise = awaitEvent("orderSubmittedEvent");

    const sellSig = await program.methods
      .submitOrder(
        sellComputationOffset,
        tradingPairId,
        sellClientPubkey,
        new anchor.BN(Buffer.from(sellClientNonce).toString('hex'), 'hex'),
        Array.from(sellCiphertext[0]),
        Array.from(sellCiphertext[1]),
        Array.from(sellCiphertext[2]),
        Array.from(sellCiphertext[3])
      )
      .accountsPartial({
        tradingPair: tradingPairPDA,
        computationAccount: getComputationAccAddress(
          program.programId,
          sellComputationOffset
        ),
        clusterAccount: arciumEnv.arciumClusterPubkey,
        mxeAccount: getMXEAccAddress(program.programId),
        mempoolAccount: getMempoolAccAddress(program.programId),
        executingPool: getExecutingPoolAccAddress(program.programId),
        compDefAccount: getCompDefAccAddress(
          program.programId,
          Buffer.from(getCompDefAccOffset("submit_order")).readUInt32LE()
        ),
        payer: payer.publicKey,
      })
      .signers([payer])
      .rpc({ skipPreflight: true, commitment: "confirmed" });

    console.log("Submit sell order sig:", sellSig);

    const sellFinalizeSig = await awaitComputationFinalization(
      provider,
      sellComputationOffset,
      program.programId,
      "confirmed"
    );
    console.log("Sell order finalize sig:", sellFinalizeSig);

    const sellOrderEvent = await sellOrderEventPromise;
    expect(sellOrderEvent.totalOrders.toString()).to.equal("2");
    console.log("✅ Sell order submitted and encrypted");

    // Match orders
    console.log("Triggering private order matching...");
    const matchComputationOffset = new anchor.BN(randomBytes(8), "hex");
    const matchEventPromise = awaitEvent("ordersMatchedEvent");

    const matchSig = await program.methods
      .matchOrders(matchComputationOffset, tradingPairId)
      .accountsPartial({
        tradingPair: tradingPairPDA,
        computationAccount: getComputationAccAddress(
          program.programId,
          matchComputationOffset
        ),
        clusterAccount: arciumEnv.arciumClusterPubkey,
        mxeAccount: getMXEAccAddress(program.programId),
        mempoolAccount: getMempoolAccAddress(program.programId),
        executingPool: getExecutingPoolAccAddress(program.programId),
        compDefAccount: getCompDefAccAddress(
          program.programId,
          Buffer.from(getCompDefAccOffset("match_orders")).readUInt32LE()
        ),
        payer: payer.publicKey,
      })
      .signers([payer])
      .rpc({ skipPreflight: true, commitment: "confirmed" });

    console.log("Match orders sig:", matchSig);

    const matchFinalizeSig = await awaitComputationFinalization(
      provider,
      matchComputationOffset,
      program.programId,
      "confirmed"
    );
    console.log("Match finalize sig:", matchFinalizeSig);

    const matchEvent = await matchEventPromise;
    expect(matchEvent.tradingPairId.toString()).to.equal(tradingPairId.toString());
    console.log("✅ Orders matched privately - trades revealed");

    // Execute trade
    console.log("Executing token transfers...");
    const buyerId = new anchor.BN(1);
    const sellerId = new anchor.BN(2);
    const tradePrice = new anchor.BN(95_000_000); // 95 USDC
    const tradeQuantity = new anchor.BN(5_000_000_000); // 5 tokens

    const tradeEventPromise = awaitEvent("tradeExecutedEvent");

    const tradeSig = await program.methods
      .executeTrade(buyerId, sellerId, tradePrice, tradeQuantity)
      .accountsPartial({
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

    console.log("Execute trade sig:", tradeSig);

    const tradeEvent = await tradeEventPromise;
    expect(tradeEvent.buyerId.toString()).to.equal(buyerId.toString());
    expect(tradeEvent.sellerId.toString()).to.equal(sellerId.toString());
    console.log("✅ Trade executed with token transfers");

    // Verify balances
    const trader1Base = await getAccount(provider.connection, trader1BaseAccount);
    const trader1Quote = await getAccount(provider.connection, trader1QuoteAccount);
    const trader2Base = await getAccount(provider.connection, trader2BaseAccount);
    const trader2Quote = await getAccount(provider.connection, trader2QuoteAccount);

    console.log("Final balances:");
    console.log("Trader 1 Base:", trader1Base.amount.toString());
    console.log("Trader 1 Quote:", trader1Quote.amount.toString());
    console.log("Trader 2 Base:", trader2Base.amount.toString());
    console.log("Trader 2 Quote:", trader2Quote.amount.toString());

    expect(Number(trader1Base.amount)).to.be.greaterThan(100_000_000_000);
    expect(Number(trader1Quote.amount)).to.be.lessThan(10_000_000_000);
    expect(Number(trader2Base.amount)).to.be.lessThan(50_000_000_000);
    expect(Number(trader2Quote.amount)).to.be.greaterThan(20_000_000_000);

    console.log("✅ Token balances updated correctly");
    console.log("\n🎉 All tests passed! ConfHide privacy trading platform is working!");
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