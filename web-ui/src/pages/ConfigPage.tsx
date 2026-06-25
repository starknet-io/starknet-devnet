import { useQuery } from '@tanstack/react-query';
import { Link } from 'react-router-dom';
import { Settings, GitFork, Server, Loader2, Users, Flame, ArrowRight } from 'lucide-react';
import { devnetGetConfig, devnetGetStatus, getBlockWithTxHashes } from '@/lib/rpc-client';
import { CopyableHash } from '@/components/CopyableHash';

function fmtGwei(n: number) { return `${(n / 1e9).toFixed(1)} Gwei`; }
function fmtGfri(n: number) { return `${(n / 1e9).toFixed(1)} Gfri`; }

export default function ConfigPage() {
  const { data: config, isLoading } = useQuery({ queryKey: ['config'], queryFn: devnetGetConfig });
  const { data: status } = useQuery({ queryKey: ['config-status'], queryFn: devnetGetStatus });
  const { data: latestBlock } = useQuery({ queryKey: ['latest-block-config'], queryFn: () => getBlockWithTxHashes('latest') });

  if (isLoading) return <div className="flex items-center justify-center h-full"><Loader2 className="animate-spin text-starknet-purple" size={32} /></div>;
  if (!config) return <div className="p-8 text-red-400">Could not load config</div>;

  const hexToGwei = (h?: string) => h ? `${(parseInt(h, 16) / 1e9).toFixed(1)} Gwei` : null;
  const hexToGfri = (h?: string) => h ? `${(parseInt(h, 16) / 1e9).toFixed(1)} Gfri` : null;

  return (
    <div className="page">
      <div className="page-header">
        <div>
          <h1 className="page-title flex items-center gap-2"><Settings size={24} className="text-starknet-purple" />Devnet Configuration</h1>
          <p className="page-subtitle">Startup settings &amp; current state</p>
        </div>
      </div>

      {status && (
        <div className="card">
          <h2 className="font-semibold mb-4 flex items-center gap-2"><Settings size={18} />Overview</h2>
          <dl className="grid grid-cols-2 md:grid-cols-4 gap-3 text-sm">
            <IR label="Blocks" value={status.block_count.toString()} />
            <IR label="Transactions" value={status.transaction_count.toString()} />
            <IR label="Chain ID" value={status.chain_id} mono />
            <IR label="Forking" value={status.is_forked ? 'Active' : 'Off'} />
          </dl>
        </div>
      )}

      <div className="card">
        <h2 className="font-semibold mb-4 flex items-center gap-2"><Server size={18} />Server</h2>
        <dl className="grid grid-cols-2 md:grid-cols-3 gap-3 text-sm">
          <IR label="Host" value={config.server_config.host} />
          <IR label="Port" value={config.server_config.port.toString()} />
          <IR label="Timeout" value={`${config.server_config.timeout}s`} />
          <IR label="Block Generation" value={config.block_generation ?? 'on transaction'} />
          <IR label="Lite Mode" value={config.lite_mode ? 'Yes' : 'No'} />
          <IR label="State Archive" value={config.state_archive} />
          <IR label="Dump On" value={config.dump_on ?? 'none'} />
          {config.dump_path && <IR label="Dump Path" value={config.dump_path} />}
        </dl>
      </div>

      <div className="card">
        <h2 className="font-semibold mb-4 flex items-center gap-2"><Flame size={18} />Gas Prices</h2>
        <p className="text-xs text-gray-500 mb-3">Initial vs current block. Yellow = modified via devnet_setGasPrice.</p>
        <div className="overflow-x-auto">
          <table className="w-full text-sm">
            <thead>
              <tr className="text-gray-400 border-b border-starknet-border">
                <th className="text-left py-2 font-medium">Parameter</th>
                <th className="text-right py-2 font-medium">Initial</th>
                <th className="text-right py-2 font-medium">Current</th>
              </tr>
            </thead>
            <tbody>
              <GasRow label="L1 Gas (ETH)" initial={fmtGwei(config.gas_price_wei)} live={hexToGwei(latestBlock?.l1_gas_price?.price_in_wei)} />
              <GasRow label="L1 Gas (STRK)" initial={fmtGfri(config.gas_price_fri)} live={hexToGfri(latestBlock?.l1_gas_price?.price_in_fri)} />
              <GasRow label="L1 Data Gas (ETH)" initial={fmtGwei(config.data_gas_price_wei)} live={hexToGwei(latestBlock?.l1_data_gas_price?.price_in_wei)} />
              <GasRow label="L1 Data Gas (STRK)" initial={fmtGfri(config.data_gas_price_fri)} live={hexToGfri(latestBlock?.l1_data_gas_price?.price_in_fri)} />
              <GasRow label="L2 Gas (ETH)" initial={fmtGwei(config.l2_gas_price_wei)} live={hexToGwei(latestBlock?.l2_gas_price?.price_in_wei)} />
              <GasRow label="L2 Gas (STRK)" initial={fmtGfri(config.l2_gas_price_fri)} live={hexToGfri(latestBlock?.l2_gas_price?.price_in_fri)} />
            </tbody>
          </table>
        </div>
      </div>

      <div className="card">
        <h2 className="font-semibold mb-4 flex items-center gap-2"><GitFork size={18} />Forking</h2>
        {config.fork_config.url ? (
          <dl className="grid grid-cols-1 md:grid-cols-2 gap-3 text-sm">
            <IR label="Fork URL" value={config.fork_config.url} />
            <IR label="Fork Block" value={`#${config.fork_config.block_number}`} />
            <IR label="Upstream Caching" value={config.fork_config.caching_enabled ? 'Yes' : 'No'} />
          </dl>
        ) : <p className="text-gray-500 text-sm">Not forking — running standalone</p>}
      </div>

      <div className="card">
        <h2 className="font-semibold mb-4 flex items-center gap-2"><Users size={18} />Accounts</h2>
        <dl className="grid grid-cols-2 md:grid-cols-3 gap-3 text-sm">
          <IR label="Predeployed" value={config.total_accounts.toString()} />
          <IR label="Seed" value={config.seed.toString()} />
          <IR label="Initial Balance" value={config.predeployed_accounts_initial_balance} mono />
          <IR label="Class Hash" value={config.account_contract_class_hash} mono />
        </dl>
        <div className="mt-3 pt-3 border-t border-starknet-border">
          <Link to="/accounts" className="text-sm text-starknet-purple hover:underline flex items-center gap-1">
            View with live balances <ArrowRight size={14} />
          </Link>
        </div>
      </div>

      <div className="card">
        <h2 className="font-semibold mb-4 flex items-center gap-2"><Flame size={18} />Fee Tokens</h2>
        <dl className="grid grid-cols-1 md:grid-cols-2 gap-3 text-sm">
          <div><span className="text-gray-400 text-xs block mb-1">ETH ERC20</span><CopyableHash value={config.eth_erc20_class_hash} short={12} /></div>
          <div><span className="text-gray-400 text-xs block mb-1">STRK ERC20</span><CopyableHash value={config.strk_erc20_class_hash} short={12} /></div>
        </dl>
      </div>
    </div>
  );
}

function GasRow({ label, initial, live }: { label: string; initial: string; live: string | null }) {
  const changed = live !== null && live !== initial;
  return (
    <tr className="border-b border-starknet-border/30">
      <td className="py-2 text-gray-400 text-xs">{label}</td>
      <td className="py-2 text-right font-mono text-gray-500 text-xs">{initial}</td>
      <td className={`py-2 text-right font-mono text-xs ${changed ? 'text-yellow-400 font-semibold' : 'text-gray-300'}`}>
        {live ?? '—'}
        {changed && <span className="ml-2 px-1.5 py-0.5 rounded bg-yellow-500/10 text-yellow-400 border border-yellow-500/20 text-[10px]">modified</span>}
      </td>
    </tr>
  );
}

function IR({ label, value, mono }: { label: string; value: string; mono?: boolean }) {
  return (
    <div className="flex flex-col gap-0.5">
      <dt className="text-gray-400 text-xs">{label}</dt>
      <dd className={`text-gray-200 text-sm break-all ${mono ? 'font-mono' : ''}`} title={value}>{value}</dd>
    </div>
  );
}
