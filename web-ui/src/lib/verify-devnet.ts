import { devnetGetStatus, devnetGetConfig } from './rpc-client';

/** Required devnet methods that must NOT be in restricted_methods */
export const REQUIRED_METHODS = [
  'devnet_getStatus',
  'devnet_getConfig',
  'devnet_getPredeployedAccounts',
  'starknet_getBlockWithTxHashes',
  'starknet_getBlockWithTxs',
  'starknet_getTransactionByHash',
  'starknet_getTransactionReceipt',
  'starknet_getTransactionStatus',
  'starknet_traceTransaction',
  'starknet_getClass',
  'starknet_getClassHashAt',
  'starknet_blockNumber',
];

export interface VerificationResult {
  ok: boolean;
  reason?: string;
}

/**
 * Verifies the RPC endpoint is actually starknet-devnet by calling
 * devnet_getStatus and devnet_getConfig, and checking that required
 * methods are not restricted.
 */
export async function verifyDevnet(): Promise<VerificationResult> {
  const helpText = 'This explorer only supports starknet-devnet. Make sure you are connecting to a running starknet-devnet instance (default: http://127.0.0.1:5050/rpc).';

  // Check devnet_getStatus first
  try {
    const status = await devnetGetStatus();
    if (!status || typeof status.chain_id !== 'string') {
      return { ok: false, reason: `devnet_getStatus returned unexpected format — this endpoint does not appear to be starknet-devnet. ${helpText}` };
    }
  } catch (e) {
    const msg = e instanceof Error ? e.message : String(e);
    if (msg.includes('Failed to fetch') || msg.includes('NetworkError')) {
      return { ok: false, reason: `Cannot reach endpoint. ${helpText}` };
    }
    return { ok: false, reason: `devnet_getStatus failed: ${msg}. ${helpText}` };
  }

  // Check devnet_getConfig for restricted methods
  try {
    const config = await devnetGetConfig();
    if (!config) {
      return { ok: false, reason: 'devnet_getConfig returned no data' };
    }
    const restricted = config.server_config?.restricted_methods ?? [];
    const blocked = REQUIRED_METHODS.filter((m) => restricted.includes(m));
    if (blocked.length > 0) {
      return { ok: false, reason: `Required devnet methods are restricted on this instance: ${blocked.join(', ')}. Remove them from --restrict-methods to use the explorer.` };
    }
  } catch (e) {
    return { ok: false, reason: `devnet_getConfig failed: ${e instanceof Error ? e.message : String(e)}` };
  }

  return { ok: true };
}
