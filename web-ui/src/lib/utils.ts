export function shortenHex(hex: string, chars = 8): string {
  if (!hex) return '';
  const stripped = hex.startsWith('0x') ? hex.slice(2) : hex;
  if (stripped.length <= chars * 2 + 2) return hex;
  return `${hex.slice(0, chars + 2)}...${hex.slice(-chars)}`;
}

export function formatTimestamp(ts: number): string {
  const date = new Date(ts * 1000);
  const now = new Date();
  const diffMs = now.getTime() - date.getTime();
  const diffSec = Math.floor(diffMs / 1000);

  if (diffSec < 60) return `${diffSec}s ago`;
  if (diffSec < 3600) return `${Math.floor(diffSec / 60)}m ago`;
  if (diffSec < 86400) return `${Math.floor(diffSec / 3600)}h ago`;
  return date.toLocaleDateString();
}

export function formatBalance(amount: string, decimals = 18): string {
  const num = parseFloat(amount) / 10 ** decimals;
  if (num >= 1_000_000) return `${(num / 1_000_000).toFixed(2)}M`;
  if (num >= 1_000) return `${(num / 1_000).toFixed(2)}K`;
  return num.toFixed(4);
}

export function isHex(value: string): boolean {
  return /^0x[0-9a-fA-F]+$/.test(value);
}

export function isBlockNumber(value: string): boolean {
  return /^\d+$/.test(value);
}
