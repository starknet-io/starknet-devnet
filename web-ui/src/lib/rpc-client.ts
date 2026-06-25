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

let rpcUrl = localStorage.getItem('devnet-rpc-url') || 'http://127.0.0.1:5050/rpc';
let wsUrl = localStorage.getItem('devnet-ws-url') || 'ws://127.0.0.1:5050/ws';

export function getRpcUrl(): string {
  return rpcUrl;
}

export function setRpcUrl(url: string): void {
  rpcUrl = url;
  localStorage.setItem('devnet-rpc-url', url);
  const ws = url.replace(/^http/, 'ws').replace(/\/rpc$/, '/ws');
  wsUrl = ws;
  localStorage.setItem('devnet-ws-url', ws);
}

export function getWsUrl(): string {
  return wsUrl;
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

// ---- WebSocket ----

export function createWebSocket(
  onMessage: (data: unknown) => void,
  onOpen?: () => void,
  onClose?: () => void,
): WebSocket {
  const ws = new WebSocket(wsUrl);

  ws.addEventListener('open', () => {
    console.log('[WS] Connected to', wsUrl);
    onOpen?.();
  });

  ws.addEventListener('message', (event) => {
    try {
      const parsed = JSON.parse(event.data);
      onMessage(parsed);
    } catch {
      // ignore non-JSON messages
    }
  });

  ws.addEventListener('close', () => {
    console.log('[WS] Disconnected');
    onClose?.();
  });

  return ws;
}

export function subscribeNewHeads(ws: WebSocket): void {
  ws.send(
    JSON.stringify({
      jsonrpc: '2.0',
      id: ++idCounter,
      method: 'starknet_subscribeNewHeads',
    }),
  );
}
