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

export function isHex(value: string): boolean {
  return /^0x[0-9a-fA-F]+$/.test(value);
}

export function isBlockNumber(value: string): boolean {
  return /^\d+$/.test(value);
}

export function isRecord(value: unknown): value is Record<string, unknown> {
  return typeof value === 'object' && value !== null && !Array.isArray(value);
}

export function stringifyValue(value: unknown): string | undefined {
  if (value == null) return undefined;
  if (typeof value === 'string') return value;
  if (typeof value === 'number' || typeof value === 'boolean' || typeof value === 'bigint') return String(value);
  return undefined;
}

/** Coerces an unknown value to a string, or returns undefined. */
export function asString(v: unknown): string | undefined {
  if (typeof v === 'string') return v;
  if (typeof v === 'number' || typeof v === 'bigint') return String(v);
  return undefined;
}

/** Coerces an unknown value to a finite number, or returns undefined. */
export function asNumber(v: unknown): number | undefined {
  if (typeof v !== 'number' || !Number.isFinite(v)) return undefined;
  return v;
}

/** Coerces an unknown value to an array of strings, or returns undefined. */
export function asStringArray(v: unknown): string[] | undefined {
  return Array.isArray(v) ? v.filter((x): x is string => typeof x === 'string') : undefined;
}

/** Parses a non-negative safe integer in base 10 without truncating invalid input. */
export function parseInt10(value: string): number | undefined {
  const trimmed = value.trim();
  if (!/^\d+$/.test(trimmed)) return undefined;
  const n = Number(trimmed);
  return Number.isSafeInteger(n) ? n : undefined;
}

export function isSnapshotId(value: string): boolean {
  return isHex(value) && BigInt(value) <= 0xffffffffffffffffn;
}

/** Converts decimal Gwei/Gfri exactly to a positive integer safe to send as JSON. */
export function parseGasPrice(value: string): number | undefined {
  const trimmed = value.trim();
  if (!/^\d+(?:\.\d{1,9})?$/.test(trimmed)) return undefined;
  const [whole, fraction = ''] = trimmed.split('.');
  const amount = BigInt(whole) * 1000000000n + BigInt(fraction.padEnd(9, '0'));
  return amount > 0n && amount <= BigInt(Number.MAX_SAFE_INTEGER) ? Number(amount) : undefined;
}

export function formatBlockGeneration(mode: 'transaction' | 'demand' | { interval: number }): string {
  return typeof mode === 'object' && mode !== null ? `Every ${mode.interval}s` : mode === 'demand' ? 'On demand' : 'On transaction';
}
