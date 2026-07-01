/** Shared status badge used across block lists, block details, and transaction details. */
export function StatusBadge({ status }: { status: string }) {
  const color =
    status === 'ACCEPTED_ON_L2'
      ? 'bg-green-500/20 text-green-400 border-green-500/30'
      : status === 'ACCEPTED_ON_L1'
        ? 'bg-blue-500/20 text-blue-400 border-blue-500/30'
        : status === 'PRE_CONFIRMED'
          ? 'bg-yellow-500/20 text-yellow-400 border-yellow-500/30'
          : status === 'ABORTED'
            ? 'bg-red-500/20 text-red-400 border-red-500/30'
            : 'bg-gray-500/20 text-gray-400 border-gray-500/30';

  return (
    <span className={`px-2 py-0.5 rounded text-xs font-medium border ${color}`}>
      {status.replace(/_/g, ' ')}
    </span>
  );
}
