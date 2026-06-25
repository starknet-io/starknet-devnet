import { useQuery } from '@tanstack/react-query';
import { Link } from 'react-router-dom';
import { Activity, Box, ArrowLeftRight, GitFork, ShieldCheck, Loader2, Wifi, WifiOff } from 'lucide-react';
import { devnetGetStatus, getBlockWithTxHashes } from '@/lib/rpc-client';
import { useDevnet } from '@/lib/DevnetContext';
import { formatTimestamp } from '@/lib/utils';
import SearchBar from '@/components/SearchBar';
import ConnectionSettings from '@/components/ConnectionSettings';
import { useEffect } from 'react';

export default function Dashboard() {
  const { connected, setConnected } = useDevnet();

  const { data: status, isLoading, error } = useQuery({
    queryKey: ['status'],
    queryFn: devnetGetStatus,
    refetchInterval: 2000,
    retry: 1,
  });

  // Automatically track connection state from RPC success/failure
  useEffect(() => {
    if (status) setConnected(true);
    else if (error) setConnected(false);
  }, [status, error, setConnected]);

  const { data: latestBlock } = useQuery({
    queryKey: ['block', 'latest-hashes'],
    queryFn: () => getBlockWithTxHashes('latest'),
    refetchInterval: 2000,
    enabled: !!status,
  });

  if (isLoading) {
    return (
      <div className="flex items-center justify-center h-full">
        <Loader2 className="animate-spin text-starknet-purple" size={32} />
      </div>
    );
  }

  if (error || !status) {
    return (
      <div className="p-8">
        <ConnectionSettings />
        <div className="text-center mt-12">
          <WifiOff size={48} className="mx-auto text-red-500 mb-4" />
          <h2 className="text-xl font-semibold mb-2">Cannot Connect to Devnet</h2>
          <p className="text-gray-400">Make sure starknet-devnet is running and the RPC URL is correct.</p>
        </div>
      </div>
    );
  }

  return (
    <div className="page">
      <div className="page-header">
        <div>
          <h1 className="page-title">Dashboard</h1>
          <p className="page-subtitle">Starknet Devnet overview</p>
        </div>
        <div className="flex flex-col sm:flex-row sm:items-center gap-3">
          {connected ? (
            <span className="flex items-center gap-1.5 text-green-400 text-sm">
              <Wifi size={14} /> Connected
            </span>
          ) : (
            <span className="flex items-center gap-1.5 text-red-400 text-sm">
              <WifiOff size={14} /> Disconnected
            </span>
          )}
          <SearchBar />
        </div>
      </div>

      {status.is_forked && (
        <div className="flex items-center gap-2 p-3 rounded-lg bg-yellow-500/10 border border-yellow-500/30 text-yellow-400 text-sm">
          <GitFork size={16} />
          <span>
            Forking from <span className="font-mono">{status.fork_config?.url}</span> at block{' '}
            <span className="font-mono">#{status.fork_config?.block}</span>
          </span>
        </div>
      )}

      {/* Stats Cards */}
      <div className="grid grid-cols-1 sm:grid-cols-2 xl:grid-cols-4 gap-4">
        <StatCard icon={Box} label="Block Count" value={status.block_count.toLocaleString()} color="text-blue-400" />
        <StatCard icon={ArrowLeftRight} label="Total Transactions" value={status.transaction_count.toLocaleString()} color="text-green-400" />
        <StatCard icon={ShieldCheck} label="Chain ID" value={status.chain_id} color="text-purple-400" isString />
        <StatCard icon={GitFork} label="Forking" value={status.is_forked ? 'Active' : 'Off'} color={status.is_forked ? 'text-yellow-400' : 'text-gray-500'} isString />
      </div>

      {/* Latest Block Card */}
      <div className="grid grid-cols-1 lg:grid-cols-2 gap-6">
        <div className="card">
          <div className="flex items-center justify-between mb-4">
            <h2 className="font-semibold text-lg">Latest Blocks</h2>
            <Link to="/blocks" className="text-sm text-starknet-purple hover:underline">View all</Link>
          </div>
          {latestBlock ? (
            <div className="space-y-3">
              <BlockRow block={latestBlock} />
            </div>
          ) : (
            <p className="text-gray-500 text-sm">No blocks found</p>
          )}
        </div>

        <div className="card">
          <div className="flex items-center justify-between mb-4">
            <h2 className="font-semibold text-lg">Devnet Info</h2>
            <Link to="/config" className="text-sm text-starknet-purple hover:underline">Full config</Link>
          </div>
          <dl className="space-y-2 text-sm">
            <InfoRow label="Protocol Version" value={status.protocol_version} />
            <InfoRow label="Pre-confirmed TXs" value={status.pre_confirmed_tx_count.toString()} />
            <InfoRow label="Auto-Impersonate" value={status.auto_impersonate ? 'Yes' : 'No'} />
            <InfoRow label="Impersonated Accounts" value={status.impersonated_accounts.length > 0 ? status.impersonated_accounts.length.toString() : 'None'} />
          </dl>
        </div>
      </div>
    </div>
  );
}

function StatCard({ icon: Icon, label, value, color, isString = false }: {
  icon: React.ElementType;
  label: string;
  value: string;
  color: string;
  isString?: boolean;
}) {
  return (
    <div className="metric-card">
      <div className="flex items-center gap-2 mb-2">
        <Icon size={18} className={color} />
        <span className="text-xs text-slate-400">{label}</span>
      </div>
      <p className={`text-2xl font-semibold text-white ${isString ? 'text-base' : ''}`}>{value}</p>
    </div>
  );
}

function BlockRow({ block }: { block: { block_number: number; block_hash: string; timestamp: number; transactions: string[] } }) {
  return (
    <Link
      to={`/blocks/${block.block_number}`}
      className="flex items-center justify-between p-3 rounded-lg bg-starknet-surface hover:bg-starknet-border transition-colors"
    >
      <div className="flex items-center gap-3">
        <Box size={16} className="text-starknet-purple" />
        <div>
          <span className="font-mono text-sm font-medium">#{block.block_number}</span>
          <span className="text-gray-500 text-xs ml-2">{formatTimestamp(block.timestamp)}</span>
        </div>
      </div>
      <div className="text-right">
        <span className="text-xs text-gray-400">{block.transactions.length} tx{block.transactions.length !== 1 ? 's' : ''}</span>
      </div>
    </Link>
  );
}

function InfoRow({ label, value }: { label: string; value: string }) {
  return (
    <div className="flex justify-between">
      <dt className="text-gray-400">{label}</dt>
      <dd className="font-mono text-gray-200">{value}</dd>
    </div>
  );
}
