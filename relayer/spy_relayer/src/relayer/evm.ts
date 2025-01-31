import {
  Bridge__factory,
  CHAIN_ID_CELO,
  CHAIN_ID_FANTOM,
  CHAIN_ID_KLAYTN,
  CHAIN_ID_POLYGON,
  getIsTransferCompletedEth,
  hexToUint8Array,
  redeemOnEth,
  redeemOnEthNative,
  importCoreWasm,
} from "@certusone/wormhole-sdk";
import {
  BigNumber,
  ContractReceipt,
  Contract,
  providers,
  Signer,
  ethers,
} from "ethers";
import { ChainConfigInfo } from "../configureEnv";
import { getScopedLogger, ScopedLogger } from "../helpers/logHelper";
import { PromHelper } from "../helpers/promHelpers";
import { CeloProvider, CeloWallet } from "@celo-tools/celo-ethers-wrapper";
import * as types from "../xRaydium/solana-proxy/generated_client/types";
import xRaydium_abi from "../utils/xRaydium_abi.json";
import * as xApp from "../xRaydium/scripts/lib";
import { parseTransferPayload } from "../utils/wormhole";
import { redeemResponseEVM } from "../xRaydium/scripts/relay";
import { info } from "console";
import * as web3 from "@solana/web3.js";
import { AvaxPIDS, mainnetSolanaRPC } from "./solana";

export function newProvider(
  url: string,
  batch: boolean = false
  //@ts-ignore
): ethers.providers.JsonRpcProvider | ethers.providers.JsonRpcBatchProvider {
  // only support http(s), not ws(s) as the websocket constructor can blow up the entire process
  // it uses a nasty setTimeout(()=>{},0) so we are unable to cleanly catch its errors
  if (url.startsWith("http")) {
    if (batch) {
      return new ethers.providers.JsonRpcBatchProvider(url);
    }
    return new ethers.providers.JsonRpcProvider(url);
  }
  throw new Error("url does not start with http/https!");
}

export type SignerWithAddress = Signer & {address: string}

export async function chainConfigToEvmProviderAndSigner(
  chainConfigInfo: ChainConfigInfo,
  walletPrivateKey?: string
): Promise<{ provider: providers.Provider; signer: SignerWithAddress }> {
  if (!walletPrivateKey) {
    walletPrivateKey = xApp._undef(
      chainConfigInfo.walletPrivateKey,
      "expected chainConfigInfo to have associated private key"
    )[0];
  }
  if (chainConfigInfo.chainId === CHAIN_ID_CELO) {
    const provider = new CeloProvider(chainConfigInfo.nodeUrl);
    await provider.ready;
    return { provider, signer: new CeloWallet(walletPrivateKey, provider) };
  } else {
    const provider = newProvider(chainConfigInfo.nodeUrl);
    return { provider, signer: new ethers.Wallet(walletPrivateKey, provider) };
  }
}

export async function relayEVM(
  chainConfigInfo: ChainConfigInfo,
  solanaChainConfigInfo: ChainConfigInfo,
  signedVAA: string,
  unwrapNative: boolean,
  checkOnly: boolean,
  walletPrivateKey: string,
  relayLogger: ScopedLogger,
  metrics: PromHelper
) {
  const logger = getScopedLogger(
    ["evm", chainConfigInfo.chainName],
    relayLogger
  );
  const signedVaaArray = hexToUint8Array(signedVAA);
  const { provider, signer } = await chainConfigToEvmProviderAndSigner(
    chainConfigInfo,
    walletPrivateKey
  );

  const { parse_vaa } = await importCoreWasm();
  const parsed = parse_vaa(signedVaaArray);

  logger.debug("Checking to see if vaa has already been redeemed.");
  const alreadyRedeemed = await getIsTransferCompletedEth(
    chainConfigInfo.tokenBridgeAddress,
    provider,
    signedVaaArray
  );

  if (alreadyRedeemed) {
    logger.info("VAA has already been redeemed!");
    return { redeemed: true, result: "already redeemed" };
  }
  if (checkOnly) {
    return { redeemed: false, result: "not redeemed" };
  }
  if (unwrapNative) {
    logger.info(
      "Will redeem and unwrap using pubkey: %s",
      await signer.getAddress()
    );
  } else {
    logger.info("Will redeem using pubkey: %s", await signer.getAddress());
  }

  //@ts-ignore
  let transferPayload = parseTransferPayload(
    Buffer.from(parsed.payload)
  ) as xApp.TransferPayloadWithData;
  console.log("transferPayload: ", transferPayload);
  console.log("relayEVM fromAddress: ", transferPayload.originAddress);
  transferPayload["payload3"] = Buffer.from(parsed["payload"].slice(133));
  logger.info(parsed, "Parsed VAA");

  // TODO: check sender of payload 3 is solana proxy via sender field
  //const XRaydiumBridge = await ethers.getContractFactory(xRaydium_abi.abi);
  //const contract = await XRaydiumBridge.attach("0xD768Ffbc3904F89f53Af2A640e3b6C640D85D6B9");

  logger.debug("Before load addrs");
  const addrs = await xApp.loadAddrs();
  logger.debug("After load addrs");
  let ctx: xApp.Context<xApp.EvmContext, xApp.SolanaContextNoSigner>;
  if (process.env.ENV_TYPE === "DEV_NET") {
    xApp.info(process.env.ENV_TYPE, "ENV_TYPE")
    ctx = xApp.getDevNetCtx(
      signer,
      chainConfigInfo.chainId,
      walletPrivateKey,
      addrs.fuji.XRaydiumBridge
    );
  } else {
    ctx = mainnetEVMContext(walletPrivateKey, chainConfigInfo.xRaydiumAddress, solanaChainConfigInfo.xRaydiumAddress)
  }
  await redeemResponseEVM(ctx.evm, signedVaaArray);

  logger.info("=============done redeem responses to EVM!!!...!!!");

  metrics.incSuccesses(chainConfigInfo.chainId);
  return { redeemed: true, result: "redeemed" };
}

function mainnetEVMContext(
  evmWalletPrivateKey: string,
  xRaydiumEvmAddr: string,
  xRaydiumSolanaAddr: string
): xApp.Context<xApp.EvmContext, xApp.SolanaContextNoSigner> {
  // const avaxKey = parseEnvVar("AVAX_KEY");
  const evmChainId = 6;
  const provider = new ethers.providers.JsonRpcProvider(
    "https://api.avax.network/ext/bc/C/rpc",
    43114
  );
  const signer = new ethers.Wallet(
    evmWalletPrivateKey,
    provider
  );
  const pids = {
    ...AvaxPIDS,
    xRaydiumEvmAddr,
    solanaProxy: new web3.PublicKey(xRaydiumSolanaAddr),
  };
  const evm: xApp.EvmContext = {
    signer,
    provider,
    evmWalletAddr: signer.address,
    chainId: evmChainId,
    ...pids,
  };
  const overrides = {
    commitment: "confirmed" as web3.Commitment,
    skipPreflight: false,
  };
  const conn = new web3.Connection(mainnetSolanaRPC, {
    commitment: overrides.commitment,
  });
  return xApp.newContext(
    new xApp.SolanaContextNoSigner(conn, overrides, pids, true),
    evm
  );
}