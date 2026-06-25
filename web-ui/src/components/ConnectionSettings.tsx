import { useState } from 'react';
import { getRpcUrl, setRpcUrl as saveRpcUrl } from '@/lib/rpc-client';
import { useQueryClient } from '@tanstack/react-query';
import { Wifi, WifiOff, Settings, Check, Loader2 } from 'lucide-react';
import { useDevnet } from '@/lib/DevnetContext';

export default function ConnectionSettings() {
  const [editing, setEditing] = useState(false);
  const [url, setUrl] = useState(getRpcUrl());
  const [testing, setTesting] = useState(false);
  const queryClient = useQueryClient();
  const { setConnected } = useDevnet();

  const testConnection = async () => {
    setTesting(true);
    try {
      const isAliveUrl = url.replace(/\/rpc$/, '') + '/is_alive';
      const resp = await fetch(isAliveUrl);
      if (resp.ok) {
        saveRpcUrl(url);
        setConnected(true);
        queryClient.invalidateQueries();
        setEditing(false);
      } else {
        setConnected(false);
      }
    } catch {
      setConnected(false);
    }
    setTesting(false);
  };

  return (
    <div className="flex items-center gap-2 min-w-0">
      {editing ? (
        <div className="flex flex-col sm:flex-row sm:items-center gap-2 w-full">
          <input
            type="text"
            value={url}
            onChange={(e) => setUrl(e.target.value)}
            className="input text-sm w-full sm:w-80"
            placeholder="http://127.0.0.1:5050/rpc"
          />
          <button
            onClick={testConnection}
            disabled={testing}
            className="btn-primary text-xs py-1.5 px-3"
          >
            {testing ? <Loader2 size={14} className="animate-spin" /> : <Check size={14} />}
          </button>
          <button
            onClick={() => setEditing(false)}
            className="text-gray-400 hover:text-gray-200 text-xs"
          >
            Cancel
          </button>
        </div>
      ) : (
        <button
          onClick={() => setEditing(true)}
          className="flex min-w-0 items-center gap-1.5 rounded-lg border border-white/10 bg-white/[0.04] px-3 py-2 text-slate-400 hover:text-white text-xs transition-colors"
          title="Change RPC URL"
        >
          <Settings size={14} />
          <span className="truncate">{getRpcUrl()}</span>
        </button>
      )}
    </div>
  );
}
