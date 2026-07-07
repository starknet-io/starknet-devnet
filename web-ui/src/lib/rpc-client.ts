import { useSyncExternalStore } from 'react';
import type {
  BlockWithTxHashes,
  BlockWithReceipts,
  BlockWithTxs,
  Transaction,
  TransactionReceipt,
  TransactionStatus,
  DevnetStatus,
  DevnetConfig,
  SerializableAccount,
  JsonRpcResponse,
} from './types';

const DEFAULT_RPC_URL = 'http://127.0.0.1:5050/rpc';
const RPC_URL_STORAGE_KEY = 'devnet-rpc-url';

let rpcUrl: string;
try {
  rpcUrl = localStorage.getItem(RPC_URL_STORAGE_KEY) || DEFAULT_RPC_URL;
} catch {
  // localStorage may be unavailable (e.g. private mode); fall back to default.
  rpcUrl = DEFAULT_RPC_URL;
}

const rpcUrlListeners = new Set<() => void>();

/** Returns the current RPC URL. Safe to call from non-React code (e.g. fetch). */
export function getRpcUrl(): string {
  return rpcUrl;
}

/**
 * Persist a new RPC URL and notify subscribers so React components re-render.
 * `callRpc` reads through `getRpcUrl`, so the next request will use the new URL.
 */
export function setRpcUrl(url: string): void {
  if (url === rpcUrl) return;
  rpcUrl = url;
  try {
    localStorage.setItem(RPC_URL_STORAGE_KEY, url);
  } catch {
    // ignore quota / private-mode failures; in-memory state is still updated.
  }
  rpcUrlListeners.forEach((listener) => listener());
}

function subscribeRpcUrl(listener: () => void): () => void {
  rpcUrlListeners.add(listener);
  return () => {
    rpcUrlListeners.delete(listener);
  };
}

/**
 * React hook returning the current RPC URL. Re-renders the caller when the URL
 * changes via `setRpcUrl`. Uses `useSyncExternalStore` so it works correctly
 * with React 18+ concurrent rendering and React Fast Refresh.
 */
export function useRpcUrl(): string {
  return useSyncExternalStore(subscribeRpcUrl, getRpcUrl, getRpcUrl);
}

let idCounter = 0;

export async function callRpc<T = unknown>(
  method: string,
  params?: unknown,
): Promise<T> {
  const response = await fetch(rpcUrl, {
    method: 'POST',
    headers: { 'Content-Type': 'application/json' },
    body: JSON.stringify({
      jsonrpc: '2.0',
      id: ++idCounter,
      method,
      params,
    }),
  });

  if (!response.ok) {
    throw new Error(`HTTP ${response.status}: ${response.statusText}`);
  }

  const data: JsonRpcResponse<T> = await response.json();

  if (data.error) {
    throw new Error(`RPC Error ${data.error.code}: ${data.error.message}`);
  }

  return data.result as T;
}

// ---- Starknet spec methods ----

export function getBlockWithTxHashes(blockId: string | number) {
  const block =
    blockId === 'latest' || blockId === 'pre_confirmed'
      ? { block_id: blockId }
      : { block_id: { block_number: Number(blockId) } };
  return callRpc<BlockWithTxHashes>('starknet_getBlockWithTxHashes', block);
}

export function getBlockWithTxs(blockId: string | number) {
  const block =
    blockId === 'latest' || blockId === 'pre_confirmed'
      ? { block_id: blockId }
      : { block_id: { block_number: Number(blockId) } };
  return callRpc<BlockWithTxs>('starknet_getBlockWithTxs', block);
}

export function getBlockWithReceipts(blockId: string | number) {
  const block =
    blockId === 'latest' || blockId === 'pre_confirmed'
      ? { block_id: blockId }
      : { block_id: { block_number: Number(blockId) } };
  return callRpc<BlockWithReceipts>('starknet_getBlockWithReceipts', block);
}

export function getTransactionByHash(txHash: string) {
  return callRpc<Transaction>('starknet_getTransactionByHash', {
    transaction_hash: txHash,
  });
}

export function getTransactionReceipt(txHash: string) {
  return callRpc<TransactionReceipt>('starknet_getTransactionReceipt', {
    transaction_hash: txHash,
  });
}

export function getTransactionStatus(txHash: string) {
  return callRpc<TransactionStatus>('starknet_getTransactionStatus', {
    transaction_hash: txHash,
  });
}

export function getTransactionTrace(txHash: string) {
  return callRpc<unknown>('starknet_traceTransaction', {
    transaction_hash: txHash,
  });
}

export function getBlockTransactionCount(blockId: string | number) {
  const block =
    blockId === 'latest' || blockId === 'pre_confirmed'
      ? { block_id: blockId }
      : { block_id: { block_number: Number(blockId) } };
  return callRpc<number>('starknet_getBlockTransactionCount', block);
}

export function blockNumber() {
  return callRpc<number>('starknet_blockNumber');
}

export function chainId() {
  return callRpc<string>('starknet_chainId');
}

// ---- Devnet spec methods ----

export function devnetGetStatus() {
  return callRpc<DevnetStatus>('devnet_getStatus');
}

export function devnetGetConfig() {
  return callRpc<DevnetConfig>('devnet_getConfig');
}

export function devnetGetPredeployedAccounts(withBalance = false) {
  return callRpc<SerializableAccount[]>(
    'devnet_getPredeployedAccounts',
    withBalance ? { with_balance: true } : {},
  );
}
