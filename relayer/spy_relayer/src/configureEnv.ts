import {
  ChainId,
  CHAIN_ID_SOLANA,
  CHAIN_ID_TERRA,
  nativeToHexString,
} from "@certusone/wormhole-sdk";
import { PublicKey } from "@solana/web3.js";
import { getLogger } from "./helpers/logHelper";

export type SupportedToken = {
  chainId: ChainId;
  address: string;
};

export type CommonEnvironment = {
  logLevel: string;
  promPort: number;
  readinessPort?: number;
  logDir?: string;
  redisHost: string;
  redisPort: number;
};

let loggingEnv: CommonEnvironment | undefined = undefined;

export const getCommonEnvironment: () => CommonEnvironment = () => {
  if (loggingEnv) {
    return loggingEnv;
  } else {
    const env = createCommonEnvironment();
    loggingEnv = env;
    return loggingEnv;
  }
};

function createCommonEnvironment(): CommonEnvironment {
  let logLevel;
  let promPort;
  let readinessPort;
  let logDir;
  let redisHost;
  let redisPort;

  if (!process.env.LOG_LEVEL) {
    throw new Error("Missing required environment variable: LOG_LEVEL");
  } else {
    logLevel = process.env.LOG_LEVEL;
  }

  if (!process.env.LOG_DIR) {
    //Not mandatory
  } else {
    logDir = process.env.LOG_DIR;
  }

  if (!process.env.PROM_PORT) {
    throw new Error("Missing required environment variable: PROM_PORT");
  } else {
    promPort = parseInt(process.env.PROM_PORT);
  }

  if (!process.env.READINESS_PORT) {
    //do nothing
  } else {
    readinessPort = parseInt(process.env.READINESS_PORT);
  }

  if (!process.env.REDIS_HOST) {
    throw new Error("Missing required environment variable: REDIS_HOST");
  } else {
    redisHost = process.env.REDIS_HOST;
  }

  if (!process.env.REDIS_PORT) {
    throw new Error("Missing required environment variable: REDIS_PORT");
  } else {
    redisPort = parseInt(process.env.REDIS_PORT);
  }

  return { logLevel, promPort, readinessPort, logDir, redisHost, redisPort };
}

export type RelayerEnvironment = {
  supportedChains: ChainConfigInfo[];
  redisHost: string;
  redisPort: number;
  clearRedisOnInit: boolean;
  demoteWorkingOnInit: boolean;
  gasTokens: TokensArray;
};

export type ChainConfigInfo = {
  chainId: ChainId;
  chainName: string;
  nativeCurrencySymbol: string;
  nodeUrl: string;
  tokenBridgeAddress: string;
  xRaydiumAddress: string;
  walletPrivateKey?: string[];
  solanaPrivateKey?: Uint8Array[];
  bridgeAddress?: string;
  terraName?: string;
  terraChainId?: string;
  terraCoin?: string;
  terraGasPriceUrl?: string;
  wrappedAsset?: string | null;
};

export type ListenerEnvironment = {
  spyServiceHost: string;
  spyServiceFilters: { chainId: ChainId; emitterAddress: string }[];
  restPort: number;
  numSpyWorkers: number;
  supportedTokens: TokensArray | undefined;
};
export type TokensArray = { chainId: ChainId; address: string }[];

let listenerEnv: ListenerEnvironment | undefined = undefined;

export const getListenerEnvironment: () => ListenerEnvironment = () => {
  if (!listenerEnv) {
    listenerEnv = createListenerEnvironment();
  }
  return listenerEnv;
};

const createListenerEnvironment: () => ListenerEnvironment = () => {
  let spyServiceHost: string;
  let spyServiceFilters: { chainId: ChainId; emitterAddress: string }[] = [];
  let restPort: number;
  let numSpyWorkers: number;
  let supportedTokens: { chainId: ChainId; address: string }[] | undefined = [];
  const logger = getLogger();

  if (!process.env.SPY_SERVICE_HOST) {
    throw new Error("Missing required environment variable: SPY_SERVICE_HOST");
  } else {
    spyServiceHost = process.env.SPY_SERVICE_HOST;
  }

  logger.info("Getting SPY_SERVICE_FILTERS...");
  if (!process.env.SPY_SERVICE_FILTERS) {
    throw new Error(
      "Missing required environment variable: SPY_SERVICE_FILTERS"
    );
  } else {
    const array = JSON.parse(process.env.SPY_SERVICE_FILTERS);
    // if (!array.foreach) {
    if (!array || !Array.isArray(array)) {
      throw new Error("Spy service filters is not an array.");
    } else {
      array.forEach((filter: any) => {
        if (filter.chainId && filter.emitterAddress) {
          logger.info(
            "nativeToHexString: " +
              nativeToHexString(filter.emitterAddress, filter.chainId)
          );
          spyServiceFilters.push({
            chainId: filter.chainId as ChainId,
            emitterAddress: filter.emitterAddress,
          });
        } else {
          throw new Error("Invalid filter record. " + filter.toString());
        }
      });
    }
  }

  logger.info("Getting REST_PORT...");
  if (!process.env.REST_PORT) {
    throw new Error("Missing required environment variable: REST_PORT");
  } else {
    restPort = parseInt(process.env.REST_PORT);
  }

  logger.info("Getting SPY_NUM_WORKERS...");
  if (!process.env.SPY_NUM_WORKERS) {
    throw new Error("Missing required environment variable: SPY_NUM_WORKERS");
  } else {
    numSpyWorkers = parseInt(process.env.SPY_NUM_WORKERS);
  }

  logger.info("Getting SUPPORTED_TOKENS...");
  logger.debug("Supported tokens: %o", process.env.SUPPORTED_TOKENS);
  if (!process.env.SUPPORTED_TOKENS) {
    throw new Error("Missing required environment variable: SUPPORTED_TOKENS");
  } else if (process.env.SUPPORTED_TOKENS.toLowerCase() === "all") {
    logger.info("All tokens supported");
    supportedTokens = undefined;
  } else {
    // const array = JSON.parse(process.env.SUPPORTED_TOKENS);
    logger.debug("eval array");
    const array = eval(process.env.SUPPORTED_TOKENS);
    logger.debug("below eval");
    if (!array || !Array.isArray(array)) {
      throw new Error("SUPPORTED_TOKENS is not an array.");
    } else {
      array.forEach((token: any) => {
        if (token.chainId && token.address) {
          (<TokensArray>supportedTokens).push({
            chainId: token.chainId,
            address: token.address,
          });
        } else {
          throw new Error("Invalid token record. " + token.toString());
        }
      });
    }
  }
  logger.debug("SUPPORTED_TOKENS: %o", supportedTokens);

  return {
    spyServiceHost,
    spyServiceFilters,
    restPort,
    numSpyWorkers,
    supportedTokens,
  };
};

let relayerEnv: RelayerEnvironment | undefined = undefined;

export const getRelayerEnvironment: () => RelayerEnvironment = () => {
  if (relayerEnv) {
    return relayerEnv;
  } else {
    const env = createRelayerEnvironment();
    relayerEnv = env;
    return relayerEnv;
  }
};

const createRelayerEnvironment: () => RelayerEnvironment = () => {
  let supportedChains: ChainConfigInfo[] = [];
  let redisHost: string;
  let redisPort: number;
  let clearRedisOnInit: boolean;
  let demoteWorkingOnInit: boolean;
  let gasTokens: TokensArray = [];

  if (!process.env.REDIS_HOST) {
    throw new Error("Missing required environment variable: REDIS_HOST");
  } else {
    redisHost = process.env.REDIS_HOST;
  }

  if (!process.env.REDIS_PORT) {
    throw new Error("Missing required environment variable: REDIS_PORT");
  } else {
    redisPort = parseInt(process.env.REDIS_PORT);
  }

  if (process.env.CLEAR_REDIS_ON_INIT === undefined) {
    throw new Error(
      "Missing required environment variable: CLEAR_REDIS_ON_INIT"
    );
  } else {
    if (process.env.CLEAR_REDIS_ON_INIT.toLowerCase() === "true") {
      clearRedisOnInit = true;
    } else {
      clearRedisOnInit = false;
    }
  }

  if (process.env.DEMOTE_WORKING_ON_INIT === undefined) {
    throw new Error(
      "Missing required environment variable: DEMOTE_WORKING_ON_INIT"
    );
  } else {
    if (process.env.DEMOTE_WORKING_ON_INIT.toLowerCase() === "true") {
      demoteWorkingOnInit = true;
    } else {
      demoteWorkingOnInit = false;
    }
  }

  supportedChains = loadChainConfig();

  let gasTokensRawArray: Array<any>;
  if (process.env.GAS_TOKENS) {
    const array = JSON.parse(process.env.GAS_TOKENS);
    if (!array || !Array.isArray(array)) {
      throw new Error("GAS_TOKENS is not an array.");
    }
    gasTokensRawArray = array;
  } else if (process.env.SUPPORTED_TOKENS) {
    const array = JSON.parse(process.env.SUPPORTED_TOKENS);
    if (!array || !Array.isArray(array)) {
      throw new Error("SUPPORTED_TOKENS is not an array.");
    }
    gasTokensRawArray = array;
  } else {
    throw new Error(
      "Missing required environment variables: one of SUPPORTED_TOKENS or GAS_TOKENS"
    );
  }
  gasTokensRawArray.forEach((token: any) => {
    if (token.chainId && token.address) {
      (<TokensArray>gasTokens).push({
        chainId: token.chainId,
        address: token.address,
      });
    } else {
      throw new Error("Invalid token record. " + token.toString());
    }
  });

  return {
    supportedChains,
    redisHost,
    redisPort,
    clearRedisOnInit,
    demoteWorkingOnInit,
    gasTokens,
  };
};

//Polygon is not supported on local Tilt network atm.
export function loadChainConfig(): ChainConfigInfo[] {
  if (!process.env.SUPPORTED_CHAINS) {
    throw new Error("Missing required environment variable: SUPPORTED_CHAINS");
  }
  if (!process.env.PRIVATE_KEYS) {
    throw new Error("Missing required environment variable: PRIVATE_KEYS");
  }

  const unformattedChains = JSON.parse(process.env.SUPPORTED_CHAINS);
  const unformattedPrivateKeys = JSON.parse(process.env.PRIVATE_KEYS);
  const supportedChains: ChainConfigInfo[] = [];

  if (!unformattedChains.forEach) {
    throw new Error("SUPPORTED_CHAINS arg was not an array.");
  }
  if (!unformattedPrivateKeys.forEach) {
    throw new Error("PRIVATE_KEYS arg was not an array.");
  }

  unformattedChains.forEach((element: any) => {
    if (!element.chainId) {
      throw new Error("Invalid chain config: " + element);
    }

    const privateKeyObj = unformattedPrivateKeys.find(
      (x: any) => x.chainId === element.chainId
    );
    if (!privateKeyObj) {
      throw new Error(
        "Failed to find private key object for configured chain ID: " +
          element.chainId
      );
    }

    if (element.chainId === CHAIN_ID_SOLANA) {
      supportedChains.push(
        createSolanaChainConfig(element, privateKeyObj.privateKeys)
      );
    } else {
      supportedChains.push(
        createEvmChainConfig(element, privateKeyObj.privateKeys)
      );
    }
  });

  return supportedChains;
}

function createSolanaChainConfig(
  config: any,
  privateKeys: any[]
): ChainConfigInfo {
  let chainId: ChainId;
  let chainName: string;
  let nativeCurrencySymbol: string;
  let nodeUrl: string;
  let tokenBridgeAddress: string;
  let solanaPrivateKey: Uint8Array[] = [];
  let bridgeAddress: string;
  let wrappedAsset: string | null;

  if (!config.chainId) {
    throw new Error("Missing required field in chain config: chainId");
  }
  if (!config.chainName) {
    throw new Error("Missing required field in chain config: chainName");
  }
  if (!config.nativeCurrencySymbol) {
    throw new Error(
      "Missing required field in chain config: nativeCurrencySymbol"
    );
  }
  if (!config.nodeUrl) {
    throw new Error("Missing required field in chain config: nodeUrl");
  }
  if (!config.tokenBridgeAddress) {
    throw new Error(
      "Missing required field in chain config: tokenBridgeAddress"
    );
  }
  if (!config.xRaydiumAddress) {
    throw new Error("Missing required field in chain config: xRaydiumAddress");
  }
  if (!(privateKeys && privateKeys.length && privateKeys.forEach)) {
    throw new Error(
      "Ill formatted object received as private keys for Solana."
    );
  }
  if (!config.bridgeAddress) {
    throw new Error("Missing required field in chain config: bridgeAddress");
  }
  if (!config.wrappedAsset) {
    throw new Error("Missing required field in chain config: wrappedAsset");
  }

  chainId = config.chainId;
  chainName = config.chainName;
  nativeCurrencySymbol = config.nativeCurrencySymbol;
  nodeUrl = config.nodeUrl;
  tokenBridgeAddress = config.tokenBridgeAddress;
  bridgeAddress = config.bridgeAddress;
  wrappedAsset = config.wrappedAsset;

  privateKeys.forEach((item: any) => {
    try {
      const uint = Uint8Array.from(item);
      solanaPrivateKey.push(uint);
    } catch (e) {
      throw new Error(
        "Failed to coerce Solana private keys into a uint array. ENV JSON is possibly incorrect."
      );
    }
  });

  return {
    chainId,
    chainName,
    nativeCurrencySymbol,
    nodeUrl,
    tokenBridgeAddress,
    xRaydiumAddress: config.xRaydiumAddress,
    bridgeAddress,
    solanaPrivateKey,
    wrappedAsset,
  };
}

function createEvmChainConfig(
  config: any,
  privateKeys: any[]
): ChainConfigInfo {
  let chainId: ChainId;
  let chainName: string;
  let nativeCurrencySymbol: string;
  let nodeUrl: string;
  let tokenBridgeAddress: string;
  let walletPrivateKey: string[];
  let wrappedAsset: string;
  let xRaydiumAddress: string;

  if (!config.chainId) {
    throw new Error("Missing required field in chain config: chainId");
  }
  if (!config.chainName) {
    throw new Error("Missing required field in chain config: chainName");
  }
  if (!config.nativeCurrencySymbol) {
    throw new Error(
      "Missing required field in chain config: nativeCurrencySymbol"
    );
  }
  if (!config.nodeUrl) {
    throw new Error("Missing required field in chain config: nodeUrl");
  }
  if (!config.tokenBridgeAddress) {
    throw new Error(
      "Missing required field in chain config: tokenBridgeAddress"
    );
  }
  if (!config.xRaydiumAddress) {
    throw new Error("Missing required field in chain config: xRaydiumAddress");
  }
  if (!(privateKeys && privateKeys.length && privateKeys.forEach)) {
    throw new Error(
      `Private keys for chain id ${config.chainId} are length zero or not an array.`
    );
  }

  if (!config.wrappedAsset) {
    throw new Error("Missing required field in chain config: wrappedAsset");
  }
  chainId = config.chainId;
  chainName = config.chainName;
  nativeCurrencySymbol = config.nativeCurrencySymbol;
  nodeUrl = config.nodeUrl;
  tokenBridgeAddress = config.tokenBridgeAddress;
  walletPrivateKey = privateKeys;
  wrappedAsset = config.wrappedAsset;
  xRaydiumAddress = config.xRaydiumAddress;

  return {
    chainId,
    chainName,
    nativeCurrencySymbol,
    nodeUrl,
    xRaydiumAddress,
    tokenBridgeAddress,
    walletPrivateKey,
    wrappedAsset,
  };
}
