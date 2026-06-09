export function escapeXml(value: string): string {
  return value
    .replace(/&/g, "&amp;")
    .replace(/</g, "&lt;")
    .replace(/>/g, "&gt;")
    .replace(/"/g, "&quot;")
    .replace(/'/g, "&apos;");
}

function safeNumber(value: number): number {
  return Number.isFinite(value) ? value : 0;
}

export function formatCompact(value: number, kind: "number" | "currency"): string {
  const clamped = Math.max(0, safeNumber(value));

  if (kind === "currency") {
    const formatted = new Intl.NumberFormat("en-US", {
      notation: "compact",
      maximumFractionDigits: clamped >= 100 ? 1 : 2,
    }).format(clamped);
    return `$${formatted}`;
  }

  return new Intl.NumberFormat("en-US", {
    notation: "compact",
    maximumFractionDigits: clamped >= 100 ? 1 : 2,
  }).format(Math.round(clamped));
}

export function formatNumber(value: number, compact = false): string {
  if (compact) return formatCompact(value, "number");
  return new Intl.NumberFormat("en-US").format(Math.max(0, Math.round(safeNumber(value))));
}

export function formatCurrency(value: number, compact = false): string {
  if (compact) return formatCompact(value, "currency");
  return new Intl.NumberFormat("en-US", {
    style: "currency",
    currency: "USD",
    minimumFractionDigits: 2,
    maximumFractionDigits: 2,
  }).format(Math.max(0, safeNumber(value)));
}

/**
 * Format milliseconds into a human-readable duration string.
 * e.g. 3661000 → "1h 1m", 90000 → "1m 30s", 500 → "<1s"
 */
export function formatDuration(ms: number | null | undefined): string {
  if (ms == null || !Number.isFinite(ms) || ms <= 0) return "—";

  const totalSeconds = Math.floor(ms / 1000);
  const hours = Math.floor(totalSeconds / 3600);
  const minutes = Math.floor((totalSeconds % 3600) / 60);
  const seconds = totalSeconds % 60;

  if (hours > 0) {
    return minutes > 0 ? `${hours}h ${minutes}m` : `${hours}h`;
  }
  if (minutes > 0) {
    return seconds > 0 && minutes < 10 ? `${minutes}m ${seconds}s` : `${minutes}m`;
  }
  if (seconds > 0) {
    return `${seconds}s`;
  }
  return "<1s";
}
