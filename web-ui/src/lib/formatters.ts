// Formatting helpers for token amounts and fees. Kept in lib/ (not in a
// component file) so that React Fast Refresh works correctly: a component
// file should only export components, and these are plain functions.

/** Formats a hex or decimal string as human-readable with proper units. */
export function formatTokenAmount(hexOrDec: string, decimals = 18): string {
  try {
    const value = BigInt(hexOrDec);
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

/** Formats fee amount (hex, 18 decimals for both WEI and FRI/STRK). */
export function formatFee(hexAmount: string): string {
  return formatTokenAmount(hexAmount, 18);
}
