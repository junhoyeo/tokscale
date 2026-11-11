import { formatCompact } from "@/lib/format";
import type { Coverage, ExplorePeriod, ModelUsageRow } from "@/lib/explore";

const MONTH_DAY = new Intl.DateTimeFormat("en-US", { month: "short", day: "numeric", timeZone: "UTC" });
const MONTH_DAY_YEAR = new Intl.DateTimeFormat("en-US", {
  month: "short",
  day: "numeric",
  year: "numeric",
  timeZone: "UTC",
});
const UPDATED = new Intl.DateTimeFormat("en-US", {
  month: "short",
  day: "numeric",
  year: "numeric",
  hour: "2-digit",
  minute: "2-digit",
  hourCycle: "h23",
  timeZone: "UTC",
});

function parseIsoDate(value: string | null | undefined): Date | null {
  if (!value) return null;
  const date = new Date(value.length === 10 ? `${value}T00:00:00.000Z` : value);
  return Number.isNaN(date.getTime()) ? null : date;
}

/** "Last 30 days", "Last 7 days", or "All time". */
export function periodLabel(period: ExplorePeriod): string {
  if (period === "7d") return "Last 7 days";
  if (period === "30d") return "Last 30 days";
  return "All time";
}

/** Inclusive UTC range, e.g. "Sep 19 – Oct 18, 2026". */
export function coverageRangeLabel(coverage: Pick<Coverage, "start" | "end">): string | null {
  const start = parseIsoDate(coverage.start);
  const end = parseIsoDate(coverage.end);
  if (!start || !end) return null;
  if (start.getUTCFullYear() === end.getUTCFullYear()) {
    return `${MONTH_DAY.format(start)} – ${MONTH_DAY_YEAR.format(end)}`;
  }
  return `${MONTH_DAY_YEAR.format(start)} – ${MONTH_DAY_YEAR.format(end)}`;
}

/** "Oct 19, 2026, 04:12 UTC", or null when there is no data time. */
export function updatedLabel(iso: string | null | undefined): string | null {
  const date = parseIsoDate(iso);
  if (!date) return null;
  return `${UPDATED.format(date)} UTC`;
}

/** Number of days in the coverage window, inclusive. */
export function coverageDays(coverage: Pick<Coverage, "start" | "end">): number | null {
  const start = parseIsoDate(coverage.start);
  const end = parseIsoDate(coverage.end);
  if (!start || !end) return null;
  return Math.round((end.getTime() - start.getTime()) / 86_400_000) + 1;
}

export function compactNumber(value: number): string {
  return formatCompact(value, "number");
}

export function compactCurrency(value: number): string {
  return formatCompact(value, "currency");
}

/** Estimated cost, or "—" when there is usage but the submission carried no price. */
export function compactCost(value: number, tokens: number): string {
  if (!(Number.isFinite(value) && value > 0) && Number.isFinite(tokens) && tokens > 0) return "—";
  return compactCurrency(value);
}

export function wholeNumber(value: number): string {
  return new Intl.NumberFormat("en-US").format(Math.max(0, Math.round(Number.isFinite(value) ? value : 0)));
}

/** Share (0..1) as a whole percent. Tiny non-zero shares read "<1%". */
export function percent(share: number): string {
  if (!Number.isFinite(share) || share <= 0) return "0%";
  if (share < 0.01) return "<1%";
  return `${Math.round(share * 100)}%`;
}

export type UserChange =
  | { kind: "new"; label: string }
  | { kind: "up" | "down" | "flat"; delta: number; label: string }
  | { kind: "none"; label: string };

/**
 * Change in active users against the previous window of the same length.
 * Shows added users, not a percentage, so small bases do not read as huge growth.
 */
export function userChange(row: Pick<ModelUsageRow, "activeUsers" | "previousActiveUsers" | "isNew">): UserChange {
  if (row.isNew) return { kind: "new", label: "New" };
  if (row.previousActiveUsers === null) return { kind: "none", label: "—" };
  const delta = row.activeUsers - row.previousActiveUsers;
  if (delta > 0) return { kind: "up", delta, label: `+${wholeNumber(delta)}` };
  if (delta < 0) return { kind: "down", delta, label: `−${wholeNumber(-delta)}` };
  return { kind: "flat", delta: 0, label: "0" };
}

/** Display names in order, without repeats (rows can list the same key twice). */
export function uniqueNames(items: ReadonlyArray<{ key: string; name: string }>): string[] {
  const seen = new Set<string>();
  const names: string[] = [];
  for (const item of items) {
    if (seen.has(item.key)) continue;
    seen.add(item.key);
    names.push(item.name);
  }
  return names;
}

/** Items without repeated keys, first occurrence wins. */
export function uniqueByKey<T extends { key: string }>(items: readonly T[]): T[] {
  const seen = new Set<string>();
  return items.filter((item) => {
    if (seen.has(item.key)) return false;
    seen.add(item.key);
    return true;
  });
}
