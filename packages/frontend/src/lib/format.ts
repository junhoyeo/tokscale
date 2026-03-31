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
