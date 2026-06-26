import { useState } from 'react';
import { Copy, Check } from 'lucide-react';

/** Renders a hash/address with a copy icon that appears on hover. */
export function CopyableHash({ value, short, className = '' }: { value: string; short?: number; className?: string }) {
  const [copied, setCopied] = useState(false);

  const handleCopy = (e: React.MouseEvent) => {
    e.preventDefault();
    e.stopPropagation();
    navigator.clipboard.writeText(value);
    setCopied(true);
    setTimeout(() => setCopied(false), 1500);
  };

  const shouldShorten = short != null && value.length > short * 2 + 5;
  const display = shouldShorten ? `${value.slice(0, short + 2)}...${value.slice(-short)}` : value;

  return (
    <span className={`group inline-flex min-w-0 max-w-full items-center gap-1.5 rounded-md border border-white/10 bg-black/20 px-1.5 py-0.5 text-starknet-mint ${className}`}>
      <span className="font-mono text-[inherit] leading-relaxed break-all" title={value}>
        {display}
      </span>
      <button
        onClick={handleCopy}
        className="opacity-70 group-hover:opacity-100 transition-opacity text-slate-500 hover:text-white shrink-0"
        title="Copy to clipboard"
      >
        {copied ? <Check size={12} className="text-green-400" /> : <Copy size={12} />}
      </button>
    </span>
  );
}

/** Formats a hex or decimal string as human-readable with proper units. */
export function formatTokenAmount(hexOrDec: string, decimals = 18): string {
  try {
    let value: bigint;
    if (hexOrDec.startsWith('0x')) {
      value = BigInt(hexOrDec);
    } else {
      value = BigInt(hexOrDec);
    }
    const divisor = BigInt(10 ** decimals);
    const whole = value / divisor;
    const frac = value % divisor;
    const fracStr = frac.toString().padStart(decimals, '0').replace(/0+$/, '');
    if (fracStr.length === 0) return whole.toLocaleString();
    return `${whole.toLocaleString()}.${fracStr.slice(0, 6)}`;
  } catch {
    return hexOrDec;
  }
}

/** Formats fee amount (hex, 18 decimals for both WEI and FRI/STRK) */
export function formatFee(hexAmount: string): string {
  return formatTokenAmount(hexAmount, 18);
}
