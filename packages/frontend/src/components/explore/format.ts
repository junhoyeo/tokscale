/**
 * Pure formatting and math helpers for the Humans / Models / Agents surfaces.
 *
 * No React and no styled-components here, so server pages, client components
 * and tests can all import it. Every function is deterministic: nothing reads
 * the clock or the locale, so server HTML and client hydration always match.
 */
import type { Coverage, ExplorePeriod, ExploreSort, TokenMix } from "@/lib/explore";

export const EXPLORE_PERIODS: readonly ExplorePeriod[] = ["7d", "30d", "all"];
export const EXPLORE_SORTS: readonly ExploreSort[] = ["users", "tokens"];
export const DEFAULT_EXPLORE_PERIOD: ExplorePeriod = "30d";
export const DEFAULT_EXPLORE_SORT: ExploreSort = "users";

/**
 * Below this many active users in the previous window, a percentage says more
 * about the small base than about the model ("+900%" from 3 users). We show
 * the added users only.
 */
export const MIN_PERCENT_BASE = 25;

/** Percent changes at or above this size are dropped; the user delta stays. */
export const MAX_SHOWN_PERCENT = 1000;

type RawParam = string | string[] | undefined | null;

function firstParam(raw: RawParam): string | undefined {
  if (Array.isArray(raw)) return raw[0];
  return raw ?? undefined;
}

export function resolveExplorePeriod(raw: RawParam, fallback: ExplorePeriod = DEFAULT_EXPLORE_PERIOD): ExplorePeriod {
  const value = firstParam(raw);
  return EXPLORE_PERIODS.includes(value as ExplorePeriod) ? (value as ExplorePeriod) : fallback;
}

export function resolveExploreSort(raw: RawParam): ExploreSort {
  const value = firstParam(raw);
  return EXPLORE_SORTS.includes(value as ExploreSort) ? (value as ExploreSort) : DEFAULT_EXPLORE_SORT;
}

export function periodLabel(period: ExplorePeriod): string {
  if (period === "7d") return "Last 7 days";
  if (period === "30d") return "Last 30 days";
  return "All time";
}

/** Label for a period control option. */
export function periodShortLabel(period: ExplorePeriod): string {
  if (period === "7d") return "7 days";
  if (period === "30d") return "30 days";
  return "All time";
}

/** "previous 30 days" — the window a change is measured against. */
export function previousWindowLabel(period: ExplorePeriod): string | null {
  if (period === "7d") return "previous 7 days";
  if (period === "30d") return "previous 30 days";
  return null;
}

export function sortLabel(sort: ExploreSort): string {
  return sort === "tokens" ? "Tokens" : "Active users";
}

function finite(value: number): number {
  return Number.isFinite(value) ? value : 0;
}

/** Exact integer with thousands separators: 1406 -> "1,406". */
export function formatCount(value: number): string {
  return new Intl.NumberFormat("en-US").format(Math.max(0, Math.round(finite(value))));
}

/** Compact token count: 2_600_000_000_000 -> "2.6T", 950 -> "950". */
export function formatTokens(value: number): string {
  const safe = Math.max(0, finite(value));
  if (safe < 1000) return formatCount(safe);
  return new Intl.NumberFormat("en-US", {
    notation: "compact",
    maximumFractionDigits: safe >= 100_000 && safe < 1_000_000 ? 0 : 1,
  }).format(safe);
}

/**
 * Compact estimated cost. The "Est." wording belongs to the label next to the
 * number, so this returns the amount only: 3_500_000 -> "$3.5M".
 */
/**
 * Estimated cost at public list prices. When there is usage (`tokens` > 0) but no
 * cost, the submitted usage carried no price (a free model, or one without a
 * list price), so it reads "—" instead of claiming "$0".
 */
export function formatEstimatedCost(value: number, tokens?: number): string {
  const safe = Math.max(0, finite(value));
  if (safe === 0) return tokens !== undefined && finite(tokens) > 0 ? NO_PRICE : "$0";
  if (safe < 1) return "<$1";
  if (safe < 1000) return `$${Math.round(safe)}`;
  return `$${new Intl.NumberFormat("en-US", { notation: "compact", maximumFractionDigits: 1 }).format(safe)}`;
}

export const NO_PRICE = "—";

/** "$1.2K est. cost", or "no price" when the usage carried no cost. */
export function estimatedCostPhrase(value: number, tokens: number): string {
  const text = formatEstimatedCost(value, tokens);
  return text === NO_PRICE ? "no price" : `${text} est. cost`;
}

/** Clamp a share to 0..1. NaN and negative values become 0. */
export function clampShare(value: number): number {
  if (!Number.isFinite(value) || value <= 0) return 0;
  return value >= 1 ? 1 : value;
}

/**
 * Adoption = active users / accounts with any usage in the same period.
 * Returns 0 for an empty period instead of NaN or Infinity.
 */
export function adoptionShare(activeUsers: number, accounts: number): number {
  if (!Number.isFinite(accounts) || accounts <= 0) return 0;
  return clampShare(activeUsers / accounts);
}

/** "36.4%", "0.4%", "<0.1%", "0%", "100%". */
export function formatShare(share: number): string {
  const value = clampShare(share) * 100;
  if (value === 0) return "0%";
  if (value < 0.1) return "<0.1%";
  if (value >= 99.95) return "100%";
  return `${value.toFixed(value >= 10 ? 0 : 1)}%`;
}

/**
 * Sum of independent shares (for example adoption across models). One account
 * can use many models, so this sum is expected to exceed 1.
 */
export function sumShares(shares: number[]): number {
  return shares.reduce((total, share) => total + clampShare(share), 0);
}

/**
 * Footnote for an adoption column. Says what the denominator is and, when the
 * listed shares add up past 100%, says so with the real total.
 */
export function adoptionNote(shares: number[], accounts: number, noun: "model" | "agent"): string {
  const base = `Adoption is the share of the ${formatCount(accounts)} ${accounts === 1 ? "account" : "accounts"} active in this period that used the ${noun}.`;
  const total = sumShares(shares);
  if (total > 1) {
    return `${base} One account can use several ${noun}s, so the shares above add up to ${Math.round(total * 100)}%, not 100%.`;
  }
  return `${base} One account can use several ${noun}s, so shares can add up to more than 100%.`;
}

export type UsersChangeKind = "none" | "new" | "up" | "down" | "flat";

export interface UsersChange {
  kind: UsersChangeKind;
  /** Signed difference in active users. 0 for "none". */
  delta: number;
  /** Short visible text, e.g. "+42", "−18", "No change", "". */
  text: string;
  /** Optional relative change, only when the base is large enough. */
  percentText: string | null;
  /** Full sentence for screen readers and titles. */
  description: string;
}

const MINUS = "\u2212";

function signed(delta: number): string {
  if (delta > 0) return `+${formatCount(delta)}`;
  if (delta < 0) return `${MINUS}${formatCount(Math.abs(delta))}`;
  return "0";
}

/**
 * Describe the change in active users against the previous window.
 *
 * Rules: lead with the number of added (or lost) users; add a percentage only
 * when the previous window had at least MIN_PERCENT_BASE users; mark models
 * with no usage before this window as new. "All time" has no previous window.
 */
export function describeUsersChange(
  current: number,
  previous: number | null,
  isNew: boolean,
  period: ExplorePeriod,
): UsersChange {
  const windowLabel = previousWindowLabel(period);
  const safeCurrent = Math.max(0, Math.round(finite(current)));

  if (previous === null || windowLabel === null) {
    return { kind: "none", delta: 0, text: "", percentText: null, description: "No previous window for all time." };
  }

  const safePrevious = Math.max(0, Math.round(finite(previous)));
  const delta = safeCurrent - safePrevious;

  if (isNew || safePrevious === 0) {
    return {
      kind: "new",
      delta,
      text: signed(delta),
      percentText: null,
      description: `New: no active users in the ${windowLabel}, ${formatCount(safeCurrent)} now.`,
    };
  }

  if (delta === 0) {
    return {
      kind: "flat",
      delta: 0,
      text: "No change",
      percentText: null,
      description: `No change from ${formatCount(safePrevious)} active users in the ${windowLabel}.`,
    };
  }

  const percent = (delta / safePrevious) * 100;
  const showPercent = safePrevious >= MIN_PERCENT_BASE && Math.abs(percent) < MAX_SHOWN_PERCENT;
  const percentText = showPercent
    ? `${percent > 0 ? "+" : MINUS}${Math.abs(percent).toFixed(Math.abs(percent) >= 10 ? 0 : 1)}%`
    : null;

  return {
    kind: delta > 0 ? "up" : "down",
    delta,
    text: signed(delta),
    percentText,
    description: `${delta > 0 ? "Up" : "Down"} ${formatCount(Math.abs(delta))} from ${formatCount(safePrevious)} active users in the ${windowLabel}.`,
  };
}

const MONTHS = ["Jan", "Feb", "Mar", "Apr", "May", "Jun", "Jul", "Aug", "Sep", "Oct", "Nov", "Dec"] as const;

interface DateParts {
  year: number;
  month: number;
  day: number;
}

function parseIsoDate(value: string | null | undefined): DateParts | null {
  if (!value) return null;
  const match = /^(\d{4})-(\d{2})-(\d{2})/.exec(value);
  if (!match) return null;
  const year = Number(match[1]);
  const month = Number(match[2]);
  const day = Number(match[3]);
  if (month < 1 || month > 12 || day < 1 || day > 31) return null;
  return { year, month, day };
}

/** "2026-10-18" -> "Oct 18, 2026". Returns the input for unparseable values. */
export function formatDay(value: string): string {
  const parts = parseIsoDate(value);
  if (!parts) return value;
  return `${MONTHS[parts.month - 1]} ${parts.day}, ${parts.year}`;
}

/** "Sep 19 – Oct 18, 2026", "Oct 12–18, 2026", "Jun 1, 2025 – Oct 18, 2026". */
export function formatDateRange(start: string, end: string): string {
  const a = parseIsoDate(start);
  const b = parseIsoDate(end);
  if (!a || !b) return `${start} – ${end}`;
  if (a.year === b.year && a.month === b.month && a.day === b.day) return formatDay(end);
  if (a.year === b.year && a.month === b.month) {
    return `${MONTHS[a.month - 1]} ${a.day}\u2013${b.day}, ${b.year}`;
  }
  if (a.year === b.year) {
    return `${MONTHS[a.month - 1]} ${a.day} \u2013 ${MONTHS[b.month - 1]} ${b.day}, ${b.year}`;
  }
  return `${formatDay(start)} \u2013 ${formatDay(end)}`;
}

/** ISO timestamp -> "Oct 19, 2026, 04:12 UTC". Fixed UTC, no locale or clock. */
export function formatUpdatedAt(iso: string | null): string | null {
  if (!iso) return null;
  const date = new Date(iso);
  if (Number.isNaN(date.getTime())) return null;
  const hh = String(date.getUTCHours()).padStart(2, "0");
  const mm = String(date.getUTCMinutes()).padStart(2, "0");
  return `${MONTHS[date.getUTCMonth()]} ${date.getUTCDate()}, ${date.getUTCFullYear()}, ${hh}:${mm} UTC`;
}

export interface CoverageFacts {
  range: string;
  accounts: string;
  identified: string;
  updated: string | null;
}

export function describeCoverage(coverage: Coverage): CoverageFacts {
  return {
    range: `${formatDateRange(coverage.start, coverage.end)} (UTC)`,
    accounts: `${formatCount(coverage.accounts)} ${coverage.accounts === 1 ? "account" : "accounts"}`,
    identified: `${formatShare(coverage.modelIdentifiedShare)} of tokens from an identified model`,
    updated: formatUpdatedAt(coverage.lastDataAt),
  };
}

export type TokenMixKey = keyof TokenMix;

export const TOKEN_MIX_ORDER: readonly TokenMixKey[] = ["input", "output", "cacheRead", "cacheWrite", "reasoning"];

export const TOKEN_MIX_LABELS: Record<TokenMixKey, string> = {
  input: "Input",
  output: "Output",
  cacheRead: "Cache read",
  cacheWrite: "Cache write",
  reasoning: "Reasoning",
};

export interface TokenMixPart {
  key: TokenMixKey;
  label: string;
  share: number;
}

/**
 * Normalise a token mix to shares that sum to 1. Works for absolute token
 * counts and for shares, so the UI does not depend on which one the data
 * layer returns. An all-zero mix returns all-zero shares.
 */
export function tokenMixParts(mix: TokenMix): TokenMixPart[] {
  const values = TOKEN_MIX_ORDER.map((key) => Math.max(0, finite(mix[key])));
  const total = values.reduce((sum, value) => sum + value, 0);
  return TOKEN_MIX_ORDER.map((key, index) => ({
    key,
    label: TOKEN_MIX_LABELS[key],
    share: total > 0 ? values[index] / total : 0,
  }));
}
