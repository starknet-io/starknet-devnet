import { CheckCircle, XCircle } from 'lucide-react';

/** Shared execution status badge (SUCCEEDED / REVERTED). */
export function ExecutionBadge({ status }: { status: string }) {
  const ok = status === 'SUCCEEDED';
  return (
    <span
      className={`px-2 py-0.5 rounded text-xs font-medium border inline-flex items-center gap-1 ${
        ok
          ? 'bg-green-500/20 text-green-400 border-green-500/30'
          : 'bg-red-500/20 text-red-400 border-red-500/30'
      }`}
    >
      {ok ? <CheckCircle size={10} /> : <XCircle size={10} />}
      {status}
    </span>
  );
}
