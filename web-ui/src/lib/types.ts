export interface DevnetStatus {
  block_count: number;
  transaction_count: number;
  pre_confirmed_tx_count: number;
  chain_id: string;
  protocol_version: string;
  is_forked: boolean;
  fork_config?: {
    url: string;
    block: number;
  };
  impersonated_accounts: string[];
  auto_impersonate: boolean;
}

export interface DevnetConfig {
  seed: number;
  total_accounts: number;
  account_contract_class_hash: string;
  predeployed_accounts_initial_balance: string;
  start_time: number | null;
  gas_price_wei: number;
  gas_price_fri: number;
  data_gas_price_wei: number;
  data_gas_price_fri: number;
  l2_gas_price_wei: number;
  l2_gas_price_fri: number;
  chain_id: string;
  dump_on: string | null;
  dump_path: string | null;
  state_archive: string;
  fork_config: {
    url: string | null;
    block_number: number | null;
    caching_enabled: boolean | null;
  };
  server_config: {
    host: string;
    port: number;
    timeout: number;
    restricted_methods: string[] | null;
  };
  block_generation: string | null;
  lite_mode: boolean;
  eth_erc20_class_hash: string;
  strk_erc20_class_hash: string;
}


export interface ResourcePrice {
  price_in_wei?: string;
  price_in_fri: string;
}

export interface BlockWithTxHashes {
  status: string;
  block_hash: string;
  block_number: number;
  parent_hash: string;
  sequencer_address: string;
  new_root: string;
  timestamp: number;
  starknet_version: string;
  l1_gas_price: ResourcePrice;
  l1_data_gas_price: ResourcePrice;
  l1_da_mode: string;
  l2_gas_price: ResourcePrice;
  transactions: string[];
  transaction_count: number;
  event_count?: number;
  state_diff_length?: number;
  transaction_commitment?: string;
  event_commitment?: string;
  receipt_commitment?: string;
  state_diff_commitment?: string;
}

export interface BlockWithTxs {
  status: string;
  block_hash: string;
  block_number: number;
  parent_hash: string;
  sequencer_address: string;
  new_root: string;
  timestamp: number;
  starknet_version: string;
  l1_gas_price: ResourcePrice;
  l1_data_gas_price: ResourcePrice;
  l1_da_mode: string;
  l2_gas_price: ResourcePrice;
  transactions: Transaction[];
  transaction_count: number;
  event_count?: number;
  state_diff_length?: number;
  transaction_commitment?: string;
  event_commitment?: string;
  receipt_commitment?: string;
  state_diff_commitment?: string;
}

export interface TransactionWithReceipt {
  transaction: Transaction;
  receipt: TransactionReceipt;
}

export interface BlockWithReceipts extends Omit<BlockWithTxs, 'transactions'> {
  transactions: TransactionWithReceipt[];
}

export interface Transaction {
  transaction_hash: string;
  type: string;
  version: string;
  nonce: string;
  max_fee?: string;
  sender_address?: string;
  signature?: string[];
  calldata?: string[];
  entry_point_selector?: string;
  [key: string]: unknown;
}

export interface TransactionReceipt {
  transaction_hash: string;
  type?: string;
  actual_fee: { amount: string; unit: string };
  execution_status: string;
  finality_status: string;
  block_hash?: string | null;
  block_number?: number | null;
  contract_address?: string;
  message_hash?: string;
  revert_reason?: string;
  messages_sent: Array<unknown>;
  events: Array<{
    from_address: string;
    keys: string[];
    data: string[];
  }>;
  execution_resources: unknown;
}

export interface TransactionStatus {
  finality_status: string;
  execution_status: string;
}

export interface SerializableAccount {
  initial_balance: string;
  address: string;
  public_key: string;
  private_key: string;
  balance?: {
    eth: { amount: string; unit: string };
    strk: { amount: string; unit: string };
  };
}

// JSON-RPC types
export interface JsonRpcRequest {
  jsonrpc: '2.0';
  id: number;
  method: string;
  params?: unknown;
}

export interface JsonRpcResponse<T = unknown> {
  jsonrpc: '2.0';
  id: number;
  result?: T;
  error?: {
    code: number;
    message: string;
  };
}
