import { useMemo, type KeyboardEvent, type ReactNode } from 'react';
import { useNavigate, useParams } from 'react-router-dom';
import { useQuery } from '@tanstack/react-query';
import {
  ArrowLeft,
  Box,
  CheckCircle,
  Loader2,
  XCircle,
} from 'lucide-react';
import { getBlockWithReceipts, getBlockWithTxs } from '@/lib/rpc-client';
import { formatTimestamp } from '@/lib/utils';
import { CopyableHash } from '@/components/CopyableHash';

interface BlockTransactionEntry {
  index: number;
  hash: string;
  txType: string;
  tx: Record<string, unknown>;
  receipt?: Record<string, unknown>;
}

export default function BlockDetail() {
  const { blockNumber } = useParams<{ blockNumber: string }>();
  const navigate = useNavigate();

  const { data: block, isLoading, error } = useQuery({
    queryKey: ['block', blockNumber, 'full-with-receipts'],
    queryFn: async () => {
      const id = Number(blockNumber);
      try {
        return await getBlockWithReceipts(id);
      } catch {
        return getBlockWithTxs(id);
      }
    },
    enabled: !!blockNumber,
  });

  const txEntries = useMemo(
    () => normalizeTransactions(Array.isArray((block as any)?.transactions) ? (block as any).transactions : []),
    [block],
  );

  if (isLoading) {
    return (
      <div className="flex items-center justify-center h-full">
        <Loader2 className="animate-spin text-starknet-purple" size={32} />
      </div>
    );
  }

  if (error || !block) return <div className="p-8 text-red-400">Block not found</div>;

  const rawBlock = block as any;
  const hasCommitments = Boolean(
    rawBlock.transaction_commitment
    || rawBlock.event_commitment
    || rawBlock.receipt_commitment
    || rawBlock.state_diff_commitment,
  );

  return (
    <div className="page">
      <button onClick={() => navigate('/blocks')} className="flex items-center gap-1 text-gray-400 hover:text-gray-200 text-sm transition-colors">
        <ArrowLeft size={14} /> Back to Blocks
      </button>

      <div className="page-header">
        <div>
          <h1 className="page-title flex items-center gap-2">
            <Box size={24} className="text-starknet-purple" />
            Block <span className="font-mono">#{blockNumber}</span>
          </h1>
          <p className="page-subtitle">Block header, gas prices, commitments, and transactions</p>
        </div>
        {rawBlock.status && <BlockStatusBadge status={rawBlock.status} />}
      </div>

      <div className="card">
        <h2 className="font-semibold mb-4">Block Header</h2>
        <dl className="grid grid-cols-1 md:grid-cols-2 xl:grid-cols-3 gap-3 text-sm">
          <InfoRow label="Block Hash" value={rawBlock.block_hash} hash />
          <InfoRow label="Parent Hash" value={rawBlock.parent_hash} hash />
          <InfoRow label="Block Number" value={`#${rawBlock.block_number}`} />
          <InfoRow label="Timestamp" value={formatTimestamp(rawBlock.timestamp)} />
          <InfoRow label="Starknet Version" value={rawBlock.starknet_version || 'N/A'} />
          <InfoRow label="Sequencer" value={rawBlock.sequencer_address} hash />
          <InfoRow label="New Root" value={rawBlock.new_root || 'N/A'} hash />
          <InfoRow label="Tx Count" value={String(rawBlock.transaction_count ?? txEntries.length)} />
          <InfoRow label="Event Count" value={String(rawBlock.event_count ?? 'N/A')} />
          <InfoRow label="L1 DA Mode" value={rawBlock.l1_da_mode || 'N/A'} />
          <InfoRow label="State Diff Length" value={String(rawBlock.state_diff_length ?? 'N/A')} />
        </dl>
      </div>

      <div className="card">
        <h2 className="font-semibold mb-4">Gas Prices</h2>
        <dl className="grid grid-cols-1 md:grid-cols-2 xl:grid-cols-3 gap-3 text-sm">
          {rawBlock.l1_gas_price?.price_in_wei && <InfoRow label="L1 Gas (WEI)" value={rawBlock.l1_gas_price.price_in_wei} mono />}
          {rawBlock.l1_gas_price?.price_in_fri && <InfoRow label="L1 Gas (FRI)" value={rawBlock.l1_gas_price.price_in_fri} mono />}
          {rawBlock.l2_gas_price?.price_in_wei && <InfoRow label="L2 Gas (WEI)" value={rawBlock.l2_gas_price.price_in_wei} mono />}
          {rawBlock.l2_gas_price?.price_in_fri && <InfoRow label="L2 Gas (FRI)" value={rawBlock.l2_gas_price.price_in_fri} mono />}
          {rawBlock.l1_data_gas_price?.price_in_wei && <InfoRow label="L1 Data Gas (WEI)" value={rawBlock.l1_data_gas_price.price_in_wei} mono />}
          {rawBlock.l1_data_gas_price?.price_in_fri && <InfoRow label="L1 Data Gas (FRI)" value={rawBlock.l1_data_gas_price.price_in_fri} mono />}
        </dl>
      </div>

      {hasCommitments && (
        <div className="card">
          <h2 className="font-semibold mb-4">Commitments</h2>
          <dl className="grid grid-cols-1 md:grid-cols-2 gap-3 text-sm">
            {rawBlock.transaction_commitment && <InfoRow label="Transaction" value={rawBlock.transaction_commitment} hash />}
            {rawBlock.event_commitment && <InfoRow label="Event" value={rawBlock.event_commitment} hash />}
            {rawBlock.receipt_commitment && <InfoRow label="Receipt" value={rawBlock.receipt_commitment} hash />}
            {rawBlock.state_diff_commitment && <InfoRow label="State Diff" value={rawBlock.state_diff_commitment} hash />}
          </dl>
        </div>
      )}

      <div className="card p-0 overflow-hidden">
        <div className="border-b border-white/10 px-5 py-4">
          <h2 className="font-semibold">Transactions ({txEntries.length})</h2>
        </div>

        {txEntries.length === 0 ? (
          <p className="px-5 py-6 text-gray-500 text-sm">No transactions in this block</p>
        ) : (
          <div>
            <div className="hidden border-b border-white/[0.07] bg-black/10 px-4 py-2 text-[10px] font-semibold uppercase tracking-wide text-slate-500 md:grid md:grid-cols-[minmax(150px,0.45fr)_minmax(260px,1.45fr)_minmax(180px,0.75fr)_minmax(190px,0.75fr)] md:gap-4">
              <span>Tx</span>
              <span>Hash</span>
              <span>Sender</span>
              <span className="md:text-right">Status</span>
            </div>
            {txEntries.map((entry) => (
              <TransactionCard key={`${entry.hash}-${entry.index}`} entry={entry} />
            ))}
          </div>
        )}
      </div>
    </div>
  );
}

function TransactionCard({ entry }: { entry: BlockTransactionEntry }) {
  const navigate = useNavigate();
  const { tx, receipt } = entry;
  const primary = getPrimaryAddress(tx, receipt);
  const executionStatus = stringifyValue(receipt?.execution_status);
  const finalityStatus = stringifyValue(receipt?.finality_status);
  const openTransaction = () => navigate(`/tx/${entry.hash}`);
  const handleKeyDown = (event: KeyboardEvent<HTMLDivElement>) => {
    if (event.key === 'Enter' || event.key === ' ') {
      event.preventDefault();
      openTransaction();
    }
  };

  return (
    <div
      role="link"
      tabIndex={0}
      onClick={openTransaction}
      onKeyDown={handleKeyDown}
      aria-label={`Open transaction ${entry.hash}`}
      className="group grid cursor-pointer gap-3 border-b border-white/[0.07] bg-starknet-card/35 px-4 py-4 transition-colors last:border-b-0 hover:bg-white/[0.035] focus:outline-none focus:ring-2 focus:ring-starknet-accent/35 md:grid-cols-[minmax(150px,0.45fr)_minmax(260px,1.45fr)_minmax(180px,0.75fr)_minmax(190px,0.75fr)] md:items-center md:gap-4"
    >
      <MetaBlock label="Tx">
        <div className="flex items-center gap-2">
          <span className="rounded-md bg-black/20 px-2 py-0.5 font-mono text-xs text-slate-500">#{entry.index}</span>
          <TxTypeBadge type={entry.txType} />
        </div>
      </MetaBlock>

      <MetaBlock label="Hash">
        <CopyableHash value={entry.hash} short={18} className="text-sm" />
      </MetaBlock>

      <MetaBlock label="Sender">
        {primary ? <CopyableHash value={primary.value} short={8} className="text-[11px]" /> : <EmptyValue />}
      </MetaBlock>

      <MetaBlock label="Status" align="right">
        <div className="flex flex-wrap gap-1.5 md:justify-end">
          {executionStatus && <ExecutionBadge status={executionStatus} />}
          {finalityStatus && <StatusBadge status={finalityStatus} />}
          {!executionStatus && !finalityStatus && <EmptyValue />}
        </div>
      </MetaBlock>
    </div>
  );
}

function InfoRow({
  label,
  value,
  hash,
  mono,
}: {
  label: string;
  value?: string | number | null;
  hash?: boolean;
  mono?: boolean;
}) {
  if (value == null || value === '') return null;

  return (
    <div className="flex items-start justify-between gap-3 rounded-lg border border-white/10 bg-black/15 px-3 py-2">
      <dt className="text-gray-400 text-sm shrink-0">{label}</dt>
      <dd className="min-w-0 text-right text-gray-200 text-sm">
        {hash ? (
          <CopyableHash value={String(value)} short={12} />
        ) : mono ? (
          <span className="font-mono text-sm break-all">{String(value)}</span>
        ) : (
          String(value)
        )}
      </dd>
    </div>
  );
}

function MetaBlock({
  label,
  children,
  align = 'left',
}: {
  label: string;
  children: ReactNode;
  align?: 'left' | 'right';
}) {
  return (
    <div className={`min-w-0 ${align === 'right' ? 'md:text-right' : ''}`}>
      <div className="mb-1 text-[10px] font-semibold uppercase tracking-wide text-slate-500 md:hidden">{label}</div>
      <div className="min-w-0 text-sm text-slate-200">{children}</div>
    </div>
  );
}

function EmptyValue() {
  return <span className="font-mono text-xs text-slate-600">--</span>;
}

function TxTypeBadge({ type }: { type: string }) {
  const colors: Record<string, string> = {
    INVOKE: 'bg-purple-500/20 text-purple-300 border-purple-500/30',
    DEPLOY: 'bg-green-500/20 text-green-300 border-green-500/30',
    DECLARE: 'bg-blue-500/20 text-blue-300 border-blue-500/30',
    DEPLOY_ACCOUNT: 'bg-teal-500/20 text-teal-300 border-teal-500/30',
    L1_HANDLER: 'bg-orange-500/20 text-orange-300 border-orange-500/30',
  };

  return (
    <span className={`px-2 py-0.5 rounded text-xs font-medium border uppercase ${colors[type] || 'bg-gray-500/20 text-gray-400 border-gray-500/30'}`}>
      {type}
    </span>
  );
}

function ExecutionBadge({ status }: { status: string }) {
  const ok = status === 'SUCCEEDED';
  return (
    <span className={`px-2 py-0.5 rounded text-xs font-medium border inline-flex items-center gap-1 ${ok ? 'bg-green-500/20 text-green-300 border-green-500/30' : 'bg-red-500/20 text-red-300 border-red-500/30'}`}>
      {ok ? <CheckCircle size={10} /> : <XCircle size={10} />}
      {status}
    </span>
  );
}

function StatusBadge({ status }: { status: string }) {
  const color =
    status === 'ACCEPTED_ON_L2'
      ? 'bg-green-500/20 text-green-300 border-green-500/30'
      : status === 'ACCEPTED_ON_L1'
        ? 'bg-blue-500/20 text-blue-300 border-blue-500/30'
        : status === 'PRE_CONFIRMED'
          ? 'bg-yellow-500/20 text-yellow-300 border-yellow-500/30'
          : 'bg-gray-500/20 text-gray-400 border-gray-500/30';

  return <span className={`px-2 py-0.5 rounded text-xs font-medium border ${color}`}>{status.replace(/_/g, ' ')}</span>;
}

function BlockStatusBadge({ status }: { status: string }) {
  const color =
    status === 'ACCEPTED_ON_L2'
      ? 'text-green-400'
      : status === 'ACCEPTED_ON_L1'
        ? 'text-blue-400'
        : status === 'PRE_CONFIRMED'
          ? 'text-yellow-400'
          : 'text-slate-300';
  return <span className={`pill ${color}`}>{status.replace(/_/g, ' ')}</span>;
}

function normalizeTransactions(items: unknown[]): BlockTransactionEntry[] {
  return items.map((item, index) => {
    if (typeof item === 'string') {
      return {
        index,
        hash: item,
        txType: 'TX',
        tx: { transaction_hash: item },
      };
    }

    const raw = isRecord(item) ? item : {};
    const tx = isRecord(raw.transaction) ? raw.transaction : raw;
    const receipt = isRecord(raw.receipt) ? raw.receipt : undefined;
    const hash =
      stringifyValue(tx.transaction_hash)
      ?? stringifyValue(tx.hash)
      ?? stringifyValue(receipt?.transaction_hash)
      ?? `tx-${index}`;
    const txType = stringifyValue(tx.type) ?? stringifyValue(receipt?.type) ?? 'TX';

    return { index, hash, txType, tx, receipt };
  });
}

function getPrimaryAddress(tx: Record<string, unknown>, receipt?: Record<string, unknown>) {
  const candidates = [
    tx.sender_address,
    tx.contract_address,
    receipt?.contract_address,
    tx.class_hash,
  ] as const;
  const found = candidates.find((value) => typeof value === 'string' && value.length > 0);
  if (!found) return undefined;
  return { value: found as string };
}

function stringifyValue(value: unknown): string | undefined {
  if (value == null) return undefined;
  if (typeof value === 'string') return value;
  if (typeof value === 'number' || typeof value === 'boolean' || typeof value === 'bigint') return String(value);
  return undefined;
}

function isRecord(value: unknown): value is Record<string, unknown> {
  return typeof value === 'object' && value !== null && !Array.isArray(value);
}
