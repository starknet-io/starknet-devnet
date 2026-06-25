import { useState } from 'react';
import { useQuery } from '@tanstack/react-query';
import { Users, Loader2, Eye, EyeOff } from 'lucide-react';
import { devnetGetPredeployedAccounts } from '@/lib/rpc-client';
import { CopyableHash } from '@/components/CopyableHash';

export default function AccountsPage() {
  const [showBalances, setShowBalances] = useState(true);
  const [showKeys, setShowKeys] = useState<Record<string, boolean>>({});

  const { data: accounts, isLoading } = useQuery({
    queryKey: ['accounts', showBalances],
    queryFn: () => devnetGetPredeployedAccounts(showBalances),
  });

  const toggleKey = (addr: string) => setShowKeys((p) => ({ ...p, [addr]: !p[addr] }));

  if (isLoading) return <div className="flex items-center justify-center h-full"><Loader2 className="animate-spin text-starknet-purple" size={32} /></div>;

  return (
    <div className="page">
      <div className="page-header">
        <div>
          <h1 className="page-title flex items-center gap-2">
            <Users size={24} className="text-starknet-purple" />
            Predeployed Accounts
          </h1>
          <p className="page-subtitle">{accounts?.length ?? 0} accounts</p>
        </div>
        <button
          onClick={() => setShowBalances((v) => !v)}
          className={`btn text-sm ${showBalances ? 'btn-primary' : 'btn-secondary'}`}
        >
          {showBalances ? 'Hide Live Balances' : 'Show Live Balances'}
        </button>
      </div>

      <div className="grid grid-cols-1 xl:grid-cols-2 2xl:grid-cols-3 gap-4">
        {accounts?.map((acct, i) => (
          <div key={acct.address} className="card">
            <div className="flex flex-col gap-4">
              <div className="space-y-3 min-w-0">
                <div className="flex items-center gap-2 min-w-0">
                  <span className="text-xs text-gray-500 font-mono">#{i}</span>
                  <CopyableHash value={acct.address} short={14} className="text-sm" />
                </div>
                <div className="text-xs text-slate-500 space-y-2">
                  <div>
                    <span className="block mb-1">Public key</span>
                    <CopyableHash value={acct.public_key} short={14} className="text-xs" />
                  </div>
                  <div>
                    <span className="block mb-1">Private key</span>
                    <div className="flex items-center gap-2 min-w-0">
                      <span className="font-mono text-slate-300 break-all">
                        {showKeys[acct.address] ? acct.private_key : '••••••••••••••••••••••••'}
                      </span>
                      <button onClick={() => toggleKey(acct.address)} className="text-gray-500 hover:text-gray-200">
                        {showKeys[acct.address] ? <EyeOff size={12} /> : <Eye size={12} />}
                      </button>
                    </div>
                  </div>
                </div>
              </div>
              <div className="grid grid-cols-2 gap-3 text-sm">
                <div className="rounded-lg border border-white/10 bg-black/15 p-3">
                  <span className="text-gray-500 text-xs">Initial</span>
                  <div className="font-mono text-gray-400 text-xs">{formatHuman(acct.initial_balance, 18)}</div>
                </div>
                {showBalances && acct.balance && (
                  <div className="rounded-lg border border-white/10 bg-black/15 p-3">
                    <span className="text-green-400 text-xs">Current</span>
                    <div className="space-y-0.5 mt-0.5">
                      <div className="font-mono text-xs text-gray-200">{formatHuman(acct.balance.eth.amount, 18)} <span className="text-gray-500">ETH</span></div>
                      <div className="font-mono text-xs text-gray-200">{formatHuman(acct.balance.strk.amount, 18)} <span className="text-gray-500">STRK</span></div>
                    </div>
                  </div>
                )}
              </div>
            </div>
          </div>
        ))}
      </div>
    </div>
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
