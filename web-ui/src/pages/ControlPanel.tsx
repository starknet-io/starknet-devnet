import { useState } from 'react';
import { Gamepad2, Loader2, AlertCircle, Copy, Check } from 'lucide-react';
import { callRpc } from '@/lib/rpc-client';
import { CopyableHash } from '@/components/CopyableHash';

type ActionResult = { type: 'success'; data: unknown } | { type: 'error'; message: string } | null;

function ActionCard({
  title,
  method,
  children,
  getParams,
  label,
  renderResult,
}: {
  title: string;
  method: string;
  children: React.ReactNode;
  getParams: () => Record<string, unknown> | null;
  label?: string;
  renderResult?: (data: any) => React.ReactNode;
}) {
  const [loading, setLoading] = useState(false);
  const [result, setResult] = useState<ActionResult>(null);
  const [copied, setCopied] = useState(false);

  const execute = async () => {
    setLoading(true);
    setResult(null);
    try {
      const params = getParams();
      const data = await callRpc(method, params ?? {});
      setResult({ type: 'success', data });
    } catch (e: any) {
      setResult({ type: 'error', message: e.message });
    }
    setLoading(false);
  };

  const handleCopy = () => {
    if (result?.type === 'success') {
      navigator.clipboard.writeText(JSON.stringify(result.data, null, 2));
      setCopied(true);
      setTimeout(() => setCopied(false), 1500);
    }
  };

  return (
    <div className="card">
      <div className="flex items-center justify-between mb-3">
        <h3 className="font-semibold text-sm">{title}</h3>
        <code className="text-[11px] px-2 py-1 rounded-md bg-black/20 text-starknet-mint font-mono break-all">{method}</code>
      </div>

      {children}

      <button onClick={execute} disabled={loading} className="btn-primary text-xs py-1.5 px-3 mt-3">
        {loading && <Loader2 size={12} className="animate-spin" />}
        {label ?? 'Execute'}
      </button>

      {result && (
        <div className={`mt-3 rounded-lg text-xs overflow-hidden ${result.type === 'success' ? 'bg-green-500/10 border border-green-500/30' : 'bg-red-500/10 border border-red-500/30'}`}>
          <div className="flex items-center justify-between px-3 py-2 border-b border-inherit">
            <span className={result.type === 'success' ? 'text-green-400' : 'text-red-400'}>
              {result.type === 'success' ? 'Response' : 'Error'}
            </span>
            {result.type === 'success' && !renderResult && (
              <button onClick={handleCopy} className="text-gray-400 hover:text-gray-200 transition-colors">
                {copied ? <Check size={12} className="text-green-400" /> : <Copy size={12} />}
              </button>
            )}
          </div>
          <div className="p-3 max-h-64 overflow-y-auto">
            {result.type === 'success' ? (
              renderResult ? renderResult(result.data) : (
                <pre className="whitespace-pre-wrap break-all font-mono text-gray-300">{JSON.stringify(result.data, null, 2)}</pre>
              )
            ) : (
              <div className="flex items-start gap-2 text-red-400">
                <AlertCircle size={14} className="mt-0.5 shrink-0" />
                <span>{result.message}</span>
              </div>
            )}
          </div>
        </div>
      )}
    </div>
  );
}

function InputField({ placeholder, value, onChange }: { placeholder: string; value: string; onChange: (v: string) => void }) {
  return <input className="input w-full text-xs" placeholder={placeholder} value={value} onChange={(e) => onChange(e.target.value)} />;
}

function ControlGroup({
  title,
  description,
  children,
}: {
  title: string;
  description: string;
  children: React.ReactNode;
}) {
  return (
    <section className="space-y-3">
      <div>
        <h2 className="text-base font-semibold text-white">{title}</h2>
        <p className="mt-1 text-xs text-slate-500">{description}</p>
      </div>
      <div className="grid grid-cols-1 lg:grid-cols-2 2xl:grid-cols-3 gap-4">
        {children}
      </div>
    </section>
  );
}

function formatHuman(raw: string, decimals = 18): string {
  try {
    const b = BigInt(raw || '0');
    const div = BigInt(10) ** BigInt(decimals);
    const w = b / div;
    const f = b % div;
    const fs = f.toString().padStart(decimals, '0').replace(/0+$/, '');
    return fs ? `${w.toLocaleString()}.${fs.slice(0, 6)}` : w.toLocaleString();
  } catch { return raw; }
}

export default function ControlPanel() {
  // Local state for each form
  const [mintAddr, setMintAddr] = useState('');
  const [mintAmount, setMintAmount] = useState('1');
  const [mintUnit, setMintUnit] = useState<'ETH' | 'STRK'>('STRK');

  const [abortBlock, setAbortBlock] = useState('');
  const [acceptBlock, setAcceptBlock] = useState('');
  const [setTimeVal, setSetTimeVal] = useState(Math.floor(Date.now() / 1000).toString());
  const [setTimeGen, setSetTimeGen] = useState(true);
  const [increaseTimeVal, setIncreaseTimeVal] = useState('60');
  const [gasWei, setGasWei] = useState('100');
  const [gasFri, setGasFri] = useState('100');
  const [dataGasWei, setDataGasWei] = useState('100');
  const [dataGasFri, setDataGasFri] = useState('100');
  const [dumpPath, setDumpPath] = useState('');
  const [loadPath, setLoadPath] = useState('');
  const [restartMessaging, setRestartMessaging] = useState(false);
  const [impersonateAddr, setImpersonateAddr] = useState('');
  const [stopImpersonateAddr, setStopImpersonateAddr] = useState('');
  const [balanceAddr, setBalanceAddr] = useState('');
  const [balanceUnit, setBalanceUnit] = useState<'ETH' | 'STRK'>('STRK');
  const [postmanUrl, setPostmanUrl] = useState('');
  const [postmanContract, setPostmanContract] = useState('');
  const [postmanKey, setPostmanKey] = useState('');

  return (
    <div className="page">
      <div className="page-header">
        <div>
        <h1 className="page-title flex items-center gap-2">
          <Gamepad2 size={24} className="text-starknet-purple" />
          Devnet Control Panel
        </h1>
        <p className="page-subtitle">Execute devnet-specific RPC methods with guided forms</p>
        </div>
      </div>

      <div className="space-y-7">
        <ControlGroup title="Funding" description="Mint tokens and fund accounts on the local devnet.">
        {/* Mint */}
        <ActionCard title="Mint Tokens" method="devnet_mint" getParams={() => ({
          address: mintAddr,
          amount: parseFloat(mintAmount || '0') * 1e18,
          unit: mintUnit === 'ETH' ? 'WEI' : 'FRI',
        })} renderResult={(data: any) => {
          if (!data) return null;
          const balance = BigInt(data.new_balance || '0');
          const divisor = BigInt(10) ** BigInt(18);
          const whole = balance / divisor;
          const frac = balance % divisor;
          const fracStr = frac.toString().padStart(18, '0').replace(/0+$/, '');
          const human = fracStr ? `${whole.toLocaleString()}.${fracStr.slice(0, 6)}` : whole.toLocaleString();
          const unit = data.unit === 'WEI' ? 'ETH' : 'STRK';
          return (
            <div className="space-y-1 text-xs">
              <div className="flex justify-between">
                <span className="text-gray-400">Tx Hash</span>
                <CopyableHash value={data.tx_hash} short={12} />
              </div>
              <div className="flex justify-between">
                <span className="text-gray-400">New Balance</span>
                <span className="font-mono text-gray-200">{human} {unit}</span>
              </div>
            </div>
          );
        }}>
          <div className="space-y-2">
            <InputField placeholder="Address (0x...)" value={mintAddr} onChange={setMintAddr} />
            <div className="flex gap-2">
              <input className="input flex-1 text-xs" placeholder="Amount" value={mintAmount} onChange={(e) => setMintAmount(e.target.value)} />
              <select className="input text-xs" value={mintUnit} onChange={(e) => setMintUnit(e.target.value as any)}>
                <option value="STRK">STRK</option>
                <option value="ETH">ETH</option>
              </select>
            </div>
          </div>
        </ActionCard>
        </ControlGroup>

        <ControlGroup title="Blocks & Time" description="Create, abort, accept, and timestamp blocks.">
        {/* Create Block */}
        <ActionCard title="Create New Block" method="devnet_createBlock" getParams={() => ({})} label="Create Block"
          renderResult={(d: any) => d && <div className="text-xs space-y-1"><div className="flex justify-between"><span className="text-gray-400">Block Hash</span><CopyableHash value={d.block_hash} short={14} /></div></div>}
        >
          <p className="text-xs text-gray-400">Generate a new block from pre-confirmed transactions.</p>
        </ActionCard>

        <ActionCard title="Abort Blocks" method="devnet_abortBlocks" getParams={() => ({ starting_block_id: { block_number: parseInt(abortBlock) } })}
          renderResult={(d: any) => d?.aborted?.length > 0 && <div className="text-xs text-gray-200">Aborted {d.aborted.length} block(s): {d.aborted.map((h: string) => <span key={h} className="font-mono ml-1"><CopyableHash value={h} short={8}/></span>)}</div>}
        >
          <InputField placeholder="Starting block number" value={abortBlock} onChange={setAbortBlock} />
        </ActionCard>

        <ActionCard title="Accept Blocks on L1" method="devnet_acceptOnL1" getParams={() => ({ starting_block_id: { block_number: parseInt(acceptBlock) } })}
          renderResult={(d: any) => d?.accepted?.length > 0 && <div className="text-xs text-gray-200">Accepted {d.accepted.length} block(s)</div>}
        >
          <InputField placeholder="Starting block number" value={acceptBlock} onChange={setAcceptBlock} />
        </ActionCard>

        <ActionCard title="Set Time" method="devnet_setTime" getParams={() => ({ time: parseInt(setTimeVal), generate_block: setTimeGen })}
          renderResult={(d: any) => d && <div className="text-xs space-y-1"><div className="flex justify-between"><span className="text-gray-400">Timestamp</span><span className="font-mono text-gray-200">{new Date(d.block_timestamp * 1000).toLocaleString()}</span></div>{d.block_hash && <div className="flex justify-between"><span className="text-gray-400">Block Hash</span><CopyableHash value={d.block_hash} short={12}/></div>}</div>}
        >
          <div className="space-y-2">
            <InputField placeholder="Unix timestamp" value={setTimeVal} onChange={setSetTimeVal} />
            <label className="flex items-center gap-2 text-xs text-gray-400">
              <input type="checkbox" checked={setTimeGen} onChange={(e) => setSetTimeGen(e.target.checked)} className="rounded" />Generate new block</label>
          </div>
        </ActionCard>

        <ActionCard title="Increase Time" method="devnet_increaseTime" getParams={() => ({ time: parseInt(increaseTimeVal) })}
          renderResult={(d: any) => d && <div className="text-xs space-y-1"><div className="flex justify-between"><span className="text-gray-400">+{d.timestamp_increased_by}s</span><span className="text-gray-200">Block: <CopyableHash value={d.block_hash} short={12}/></span></div></div>}
        >
          <div className="flex items-center gap-2"><InputField placeholder="Seconds" value={increaseTimeVal} onChange={setIncreaseTimeVal} /><span className="text-xs text-gray-500">sec</span></div>
        </ActionCard>
        </ControlGroup>

        <ControlGroup title="Gas Prices" description="Adjust live L1, L2, and data gas pricing.">
        {/* Set Gas Price */}
        <ActionCard title="Set Gas Price" method="devnet_setGasPrice" getParams={() => ({
          gas_price_wei: parseInt(gasWei || '0') * 1e9,
          gas_price_fri: parseInt(gasFri || '0') * 1e9,
          data_gas_price_wei: parseInt(dataGasWei || '0') * 1e9,
          data_gas_price_fri: parseInt(dataGasFri || '0') * 1e9,
        })} renderResult={(d: any) => d && <div className="text-xs space-y-1"><div className="flex justify-between"><span className="text-gray-400">Gas (wei)</span><span className="font-mono text-gray-200">{d.gas_price_wei?.toLocaleString()}</span></div><div className="flex justify-between"><span className="text-gray-400">Gas (fri)</span><span className="font-mono text-gray-200">{d.gas_price_fri?.toLocaleString()}</span></div></div>}>
          <div className="grid grid-cols-2 gap-2">
            <div><label className="text-xs text-gray-500">Gas (Gwei)</label><input className="input w-full text-xs" value={gasWei} onChange={(e) => setGasWei(e.target.value)} /></div>
            <div><label className="text-xs text-gray-500">Gas (Gfri)</label><input className="input w-full text-xs" value={gasFri} onChange={(e) => setGasFri(e.target.value)} /></div>
            <div><label className="text-xs text-gray-500">Data Gas (Gwei)</label><input className="input w-full text-xs" value={dataGasWei} onChange={(e) => setDataGasWei(e.target.value)} /></div>
            <div><label className="text-xs text-gray-500">Data Gas (Gfri)</label><input className="input w-full text-xs" value={dataGasFri} onChange={(e) => setDataGasFri(e.target.value)} /></div>
          </div>
        </ActionCard>
        </ControlGroup>

        <ControlGroup title="State & Runtime" description="Persist, restore, or restart the devnet process state.">
        <ActionCard title="Dump State" method="devnet_dump" getParams={() => dumpPath ? { path: dumpPath } : {}} label="Dump"
          renderResult={(d: any) => {
            if (d === null || d === undefined) return <div className="text-xs text-gray-400">Saved to file</div>;
            if (Array.isArray(d)) return <div className="text-xs text-gray-200">{d.length} event(s) in dump</div>;
            return null;
          }}
        >
          <InputField placeholder="Path (optional, returns in response if empty)" value={dumpPath} onChange={setDumpPath} />
        </ActionCard>

        <ActionCard title="Load State" method="devnet_load" getParams={() => ({ path: loadPath })} label="Load">
          <InputField placeholder="File path to load" value={loadPath} onChange={setLoadPath} />
        </ActionCard>

        <ActionCard title="Restart Devnet" method="devnet_restart" getParams={() => ({ restart_l1_to_l2_messaging: restartMessaging })} label="Restart"
          renderResult={() => <div className="text-xs text-green-400">Devnet restarted</div>}
        >
          <label className="flex items-center gap-2 text-xs text-gray-400">
            <input type="checkbox" checked={restartMessaging} onChange={(e) => setRestartMessaging(e.target.checked)} className="rounded" />Restart L1-L2 messaging</label>
        </ActionCard>
        </ControlGroup>

        <ControlGroup title="Accounts & Impersonation" description="Inspect accounts and control impersonation helpers.">
        <ActionCard title="Impersonate Account" method="devnet_impersonateAccount" getParams={() => ({ account_address: impersonateAddr })}
          renderResult={() => <div className="text-xs text-green-400">Account impersonated</div>}
        >
          <InputField placeholder="Account address (0x...)" value={impersonateAddr} onChange={setImpersonateAddr} />
        </ActionCard>

        <ActionCard title="Stop Impersonating" method="devnet_stopImpersonateAccount" getParams={() => ({ account_address: stopImpersonateAddr })} label="Stop"
          renderResult={() => <div className="text-xs text-green-400">Impersonation stopped</div>}
        >
          <InputField placeholder="Account address (0x...)" value={stopImpersonateAddr} onChange={setStopImpersonateAddr} />
        </ActionCard>

        <ActionCard title="Auto-Impersonate" method="devnet_autoImpersonate" getParams={() => ({})} label="Enable">
          <p className="text-xs text-gray-400">Enable auto-impersonation for unknown accounts.</p>
        </ActionCard>
        <ActionCard title="Stop Auto-Impersonate" method="devnet_stopAutoImpersonate" getParams={() => ({})} label="Disable">
          <p className="text-xs text-gray-400">Disable auto-impersonation.</p>
        </ActionCard>

        <ActionCard title="Get Account Balance" method="devnet_getAccountBalance" getParams={() => ({ address: balanceAddr, unit: balanceUnit === 'ETH' ? 'WEI' : 'FRI' })} label="Get Balance"
          renderResult={(d: any) => d && (<div className="text-xs"><div className="flex justify-between"><span className="text-gray-400">Balance</span><span className="font-mono text-gray-200">{formatHuman(d.amount, 18)} {d.unit === 'WEI' ? 'ETH' : 'STRK'}</span></div><div className="flex justify-between mt-1"><span className="text-gray-400">Raw</span><span className="font-mono text-gray-500">{d.amount}</span></div></div>)}
        >
          <div className="flex gap-2">
            <InputField placeholder="Address (0x...)" value={balanceAddr} onChange={setBalanceAddr} />
            <select className="input text-xs" value={balanceUnit} onChange={(e) => setBalanceUnit(e.target.value as any)}>
              <option value="STRK">STRK</option><option value="ETH">ETH</option></select>
          </div>
        </ActionCard>

        <ActionCard title="Get Predeployed Accounts" method="devnet_getPredeployedAccounts" getParams={() => ({ with_balance: false })} label="Fetch"
          renderResult={(d: any) => Array.isArray(d) ? <div className="text-xs text-gray-200">{d.length} accounts listed</div> : null}
        >
          <p className="text-xs text-gray-400">List all predeployed accounts with addresses and keys.</p>
        </ActionCard>
        </ControlGroup>

        <ControlGroup title="Messaging" description="Operate the L1-L2 postman utilities.">
        <ActionCard title="Postman Flush" method="devnet_postmanFlush" getParams={() => ({ dry_run: false })} label="Flush"
          renderResult={(d: any) => d && <div className="text-xs space-y-1"><div className="flex justify-between"><span className="text-gray-400">L2 → L1</span><span className="font-mono text-gray-200">{d.messages_to_l1?.length ?? 0}</span></div><div className="flex justify-between"><span className="text-gray-400">L1 → L2</span><span className="font-mono text-gray-200">{d.messages_to_l2?.length ?? 0}</span></div><div className="flex justify-between"><span className="text-gray-400">L2 TXs generated</span><span className="font-mono text-gray-200">{d.generated_l2_transactions?.length ?? 0}</span></div></div>}
        >
          <p className="text-xs text-gray-400">Flush L1-L2 messages.</p>
        </ActionCard>

        <ActionCard title="Postman Load" method="devnet_postmanLoad" getParams={() => ({
          network_url: postmanUrl, address: postmanContract || undefined, deployer_account_private_key: postmanKey || undefined,
        })} label="Load"
          renderResult={(d: any) => d?.messaging_contract_address && <div className="text-xs flex justify-between"><span className="text-gray-400">Messaging Contract</span><CopyableHash value={d.messaging_contract_address} short={12}/></div>}
        >
          <div className="space-y-2">
            <InputField placeholder="Network URL" value={postmanUrl} onChange={setPostmanUrl} />
            <InputField placeholder="Messaging Contract (optional)" value={postmanContract} onChange={setPostmanContract} />
            <InputField placeholder="Deployer Private Key (optional)" value={postmanKey} onChange={setPostmanKey} />
          </div>
        </ActionCard>
        </ControlGroup>
      </div>
    </div>
  );
}
