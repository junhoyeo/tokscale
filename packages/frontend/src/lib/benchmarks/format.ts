/**
 * Formatting for benchmark scores, prices and specs. Pure and deterministic
 * (no locale or clock), so server HTML and client hydration match.
 */
import { effortLabel, type Effort } from "./conditions";
import type { BenchmarkUnit } from "./definitions";

const MONTHS = ["Jan", "Feb", "Mar", "Apr", "May", "Jun", "Jul", "Aug", "Sep", "Oct", "Nov", "Dec"] as const;

/** Score in the benchmark's unit: "64.8%", "168.8", "1,807", "+0.20". */
export function formatScore(unit: BenchmarkUnit, value: number): string {
  if (!Number.isFinite(value)) return "\u2014";
  if (unit === "percent") return `${(value * 100).toFixed(1)}%`;
  if (unit === "index") return value.toFixed(1);
  if (unit === "rating") return new Intl.NumberFormat("en-US", { maximumFractionDigits: 0 }).format(Math.round(value));
  const fixed = Math.abs(value).toFixed(2);
  return value > 0 ? `+${fixed}` : value < 0 ? `\u2212${fixed}` : fixed;
}

/** Confidence interval in the unit: "61.7\u201368.0%", "165.3\u2013174.0". Null without both bounds. */
export function formatInterval(unit: BenchmarkUnit, low: number | null, high: number | null): string | null {
  if (low === null || high === null || !Number.isFinite(low) || !Number.isFinite(high)) return null;
  if (unit === "percent") return `${(low * 100).toFixed(1)}\u2013${(high * 100).toFixed(1)}%`;
  if (unit === "index") return `${low.toFixed(1)}\u2013${high.toFixed(1)}`;
  if (unit === "rating") return `${Math.round(low)}\u2013${Math.round(high)}`;
  return `${low.toFixed(2)} to ${high.toFixed(2)}`;
}

/** Short condition tag for a score cell: "Max", "XHigh · Claude Code", "32K budget". */
export function conditionTag(effort: Effort | null, agent: string | null, thinkingBudget: string | null = null): string | null {
  const parts: string[] = [];
  if (effort) parts.push(effortLabel(effort));
  else if (thinkingBudget) parts.push(`${thinkingBudget} budget`);
  if (agent) parts.push(agent);
  return parts.length > 0 ? parts.join(" \u00b7 ") : null;
}

/** "2026-10-06" -> "Oct 6, 2026"; "2026-06" -> "Jun 2026". */
export function formatIsoDay(value: string | null): string | null {
  if (!value) return null;
  const day = /^(\d{4})-(\d{2})-(\d{2})/.exec(value);
  if (day) return `${MONTHS[Number(day[2]) - 1]} ${Number(day[3])}, ${day[1]}`;
  const month = /^(\d{4})-(\d{2})$/.exec(value);
  if (month) return `${MONTHS[Number(month[2]) - 1]} ${month[1]}`;
  return value;
}

/** USD per 1M tokens: "$0.32", "$4", "$12.50", "$0.004". */
export function formatPricePerMillion(value: number | null): string {
  if (value === null || !Number.isFinite(value) || value < 0) return "\u2014";
  if (value === 0) return "$0";
  if (value < 0.01) return `$${value.toFixed(3)}`;
  if (value < 10 && !Number.isInteger(value)) return `$${value.toFixed(2)}`;
  if (Number.isInteger(value)) return `$${value}`;
  return `$${value.toFixed(2)}`;
}

/** Whole-dollar amounts with separators for run costs: "$4,719", "$17.28". */
export function formatUsd(value: number | null): string {
  if (value === null || !Number.isFinite(value)) return "\u2014";
  if (value >= 100) return `$${new Intl.NumberFormat("en-US", { maximumFractionDigits: 0 }).format(Math.round(value))}`;
  return `$${value.toFixed(2)}`;
}

/** Token windows: 1_000_000 -> "1M", 262_144 -> "262K", 128_000 -> "128K". */
export function formatContext(value: number | null): string {
  if (value === null || !Number.isFinite(value) || value <= 0) return "\u2014";
  if (value >= 1_000_000) {
    const m = value / 1_000_000;
    return `${Number.isInteger(m) ? m : m.toFixed(m >= 10 ? 0 : 2).replace(/\.?0+$/, "")}M`;
  }
  if (value >= 1_000) return `${Math.round(value / 1_000)}K`;
  return String(value);
}

const MODALITY_LABELS: Record<string, string> = { text: "Text", image: "Image", pdf: "PDF", audio: "Audio", video: "Video" };

export function formatModalities(values: readonly string[]): string {
  if (values.length === 0) return "\u2014";
  return values.map((v) => MODALITY_LABELS[v] ?? v).join(", ");
}
