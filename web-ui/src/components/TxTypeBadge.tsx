/** Shared transaction type badge used across block details and transaction details. */
export function TxTypeBadge({ type }: { type: string }) {
  const colors: Record<string, string> = {
    INVOKE: 'bg-purple-500/20 text-purple-400 border-purple-500/30',
    DEPLOY: 'bg-green-500/20 text-green-400 border-green-500/30',
    DECLARE: 'bg-blue-500/20 text-blue-400 border-blue-500/30',
    DEPLOY_ACCOUNT: 'bg-teal-500/20 text-teal-400 border-teal-500/30',
    L1_HANDLER: 'bg-orange-500/20 text-orange-400 border-orange-500/30',
  };

  return (
    <span
      className={`px-2 py-0.5 rounded text-xs font-medium border uppercase ${
        colors[type] || 'bg-gray-500/20 text-gray-400 border-gray-500/30'
      }`}
    >
      {type}
    </span>
  );
}
