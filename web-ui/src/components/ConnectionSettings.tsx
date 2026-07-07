import { useState } from 'react';
import { setRpcUrl, useRpcUrl } from '@/lib/rpc-client';
import { verifyDevnet } from '@/lib/verify-devnet';
import { useQueryClient } from '@tanstack/react-query';
import { Settings, Check, Loader2, AlertTriangle } from 'lucide-react';
import { useDevnet } from '@/lib/useDevnet';

interface ConnectionSettingsProps {
  /** Show the URL input inline (no click-to-reveal). Used on error pages. */
  standalone?: boolean;
  /** Called when editing is dismissed (cancel or successful connect). */
  onDone?: () => void;
}

export default function ConnectionSettings({ standalone = false, onDone }: ConnectionSettingsProps) {
  const currentUrl = useRpcUrl();
  const [editing, setEditing] = useState(standalone);
  const [url, setUrl] = useState(currentUrl);
  const [testing, setTesting] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const queryClient = useQueryClient();
  const { setConnected, setVerified } = useDevnet();

  const testConnection = async () => {
    setTesting(true);
    setError(null);
    setRpcUrl(url);

    const result = await verifyDevnet();
    if (result.ok) {
      setConnected(true);
      setVerified(true);
      queryClient.invalidateQueries();
      setEditing(false);
      onDone?.();
    } else {
      setConnected(false);
      setVerified(false);
      setError(result.reason ?? 'Unknown error');
    }
    setTesting(false);
  };

  const handleKeyDown = (e: React.KeyboardEvent) => {
    if (e.key === 'Enter' && !testing) testConnection();
  };

  const form = (
    <div className="w-full">
      <label className="block text-xs font-medium text-gray-400 mb-1.5">
        Devnet RPC Endpoint
      </label>
      <div className="flex gap-2">
        <input
          type="text"
          value={url}
          onChange={(e) => setUrl(e.target.value)}
          onKeyDown={handleKeyDown}
          className="input text-sm flex-1"
          placeholder="http://127.0.0.1:5050/rpc"
          autoFocus={standalone}
        />
        <button
          onClick={testConnection}
          disabled={testing}
          className="btn-primary text-sm px-4 py-2 flex items-center gap-2 shrink-0"
        >
          {testing ? (
            <Loader2 size={14} className="animate-spin" />
          ) : (
            <Check size={14} />
          )}
          Connect
        </button>
        {!standalone && (
          <button
            onClick={() => { setEditing(false); setError(null); onDone?.(); }}
            className="text-gray-400 hover:text-gray-200 text-sm px-3"
          >
            Cancel
          </button>
        )}
      </div>
      {error && (
        <div className="flex items-start gap-1.5 mt-2 text-red-400 text-xs">
          <AlertTriangle size={12} className="shrink-0 mt-0.5" />
          <span>{error}</span>
        </div>
      )}
    </div>
  );

  if (editing) return form;

  return (
    <button
      onClick={() => setEditing(true)}
      className="flex min-w-0 items-center gap-1.5 rounded-lg border border-white/10 bg-white/[0.04] px-3 py-2 text-slate-400 hover:text-white text-xs transition-colors"
      title="Change RPC URL"
    >
      <Settings size={14} />
      <span className="truncate">{currentUrl}</span>
    </button>
  );
}
