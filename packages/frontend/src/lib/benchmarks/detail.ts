/**
 * Pure helpers for the /benchmarks index and the /benchmarks/[slug] pages:
 * index rows, result table rows joined with Tokscale usage, the score vs
 * release date chart, and the model matching summary.
 *
 * No data loading and no snapshot imports, so client components can use the
 * chart geometry. Pages pass in the benchmark model and the usage rows they
 * loaded, then hand plain rows to the client.
 */
import { modelRef } from "@/lib/explore/modelMeta";
import type { ModelUsageRow } from "@/lib/explore/types";
import { modelPath } from "@/lib/seo/explore";
import { sourceNotes } from "./attribution";
import { effortLabel } from "./conditions";
import { CATEGORY_LABELS, type BenchmarkCategory, type BenchmarkDefinition, type BenchmarkUnit } from "./definitions";
import { formatInterval, formatScore, formatUsd } from "./format";
import { compareResults, type BenchmarkModel, type ResolvedBenchmark } from "./model";
import type { BenchmarkResult } from "./schema";
import { benchmarkPath } from "./urls";

export type BenchmarkStatus = BenchmarkDefinition["status"];
export type DateKind = "run" | "published" | "release";

export interface DatedValue {
  /** YYYY-MM-DD */
  date: string;
  kind: DateKind;
}

const DATE_KIND_ORDER: Record<DateKind, number> = { run: 0, published: 1, release: 2 };

const COUNT = new Intl.NumberFormat("en-US");

function count(value: number): string {
  return COUNT.format(Math.round(value));
}

/** The date shown for a result: run date, else published date, else the model's release date. */
export function resultDate(r: BenchmarkResult): DatedValue | null {
  if (r.runDate) return { date: r.runDate, kind: "run" };
  if (r.publishedDate) return { date: r.publishedDate, kind: "published" };
  if (r.modelReleaseDate) return { date: r.modelReleaseDate, kind: "release" };
  return null;
}

/** Newest result date. On the same day a run date wins over a publish date, and both over a release date. */
export function newestResultDate(results: readonly BenchmarkResult[]): DatedValue | null {
  let newest: DatedValue | null = null;
  for (const r of results) {
    const d = resultDate(r);
    if (!d) continue;
    if (!newest || d.date > newest.date || (d.date === newest.date && DATE_KIND_ORDER[d.kind] < DATE_KIND_ORDER[newest.kind])) {
      newest = d;
    }
  }
  return newest;
}

/** Status, saturation and staleness notes, in that order. */
export function benchmarkNotes(b: ResolvedBenchmark): string[] {
  return [b.definition.statusNote, b.saturationNote, b.staleNote].filter((note): note is string => Boolean(note));
}

/** Link-only benchmarks (no republished results) have no page of their own. */
export function hasBenchmarkPage(b: ResolvedBenchmark): boolean {
  return b.definition.status !== "link-only" && b.results.length > 0;
}

export function benchmarkPageSlugs(model: BenchmarkModel): string[] {
  return model.list().filter(hasBenchmarkPage).map((b) => b.definition.slug);
}

/* ------------------------------------------------------------------ index */

export interface BenchmarkIndexItem {
  slug: string;
  name: string;
  /** Path of the detail page; null for link-only benchmarks. */
  href: string | null;
  /** The benchmark's own page or leaderboard. */
  url: string;
  measures: string;
  publisher: string;
  collectedBy: string | null;
  unitLabel: string;
  status: BenchmarkStatus;
  resultCount: number;
  modelCount: number;
  externalModelCount: number;
  lastUpdate: DatedValue | null;
  /** Day the snapshot was retrieved (YYYY-MM-DD); null for link-only benchmarks. */
  retrievedAt: string | null;
  licenseName: string;
  licenseUrl: string | null;
  notes: string[];
}

export interface BenchmarkIndexGroup {
  category: BenchmarkCategory;
  label: string;
  items: BenchmarkIndexItem[];
}

export function benchmarkIndexItem(b: ResolvedBenchmark): BenchmarkIndexItem {
  const d = b.definition;
  return {
    slug: d.slug,
    name: d.name,
    href: hasBenchmarkPage(b) ? benchmarkPath(d.slug) : null,
    url: d.url,
    measures: d.measures,
    publisher: d.publisher,
    collectedBy: d.collectedBy,
    unitLabel: d.unitLabel,
    status: d.status,
    resultCount: b.results.length,
    modelCount: b.modelCount,
    externalModelCount: b.externalModelCount,
    lastUpdate: newestResultDate(b.results),
    retrievedAt: b.snapshot?.retrievedAt.slice(0, 10) ?? null,
    licenseName: d.license.name,
    licenseUrl: d.license.url,
    notes: benchmarkNotes(b),
  };
}

export function buildBenchmarkIndex(model: BenchmarkModel): BenchmarkIndexGroup[] {
  return model.byCategory().map((group) => ({
    category: group.category,
    label: CATEGORY_LABELS[group.category],
    items: group.benchmarks.map(benchmarkIndexItem),
  }));
}

export interface BenchmarkIndexSummary {
  /** Benchmarks with a page of their own. */
  withResults: number;
  linkOnly: number;
  /** Names of the sources whose results are republished, A to Z. */
  sources: string[];
  /** Oldest and newest snapshot retrieval day (YYYY-MM-DD). */
  retrievedFrom: string | null;
  retrievedTo: string | null;
}

export function benchmarkIndexSummary(model: BenchmarkModel): BenchmarkIndexSummary {
  const all = model.list();
  const days = model
    .snapshots()
    .map((s) => s.retrievedAt.slice(0, 10))
    .sort();
  return {
    withResults: all.filter(hasBenchmarkPage).length,
    linkOnly: all.filter((b) => b.definition.status === "link-only").length,
    sources: sourceNotes(model)
      .map((note) => note.name)
      .sort((a, b) => a.localeCompare(b, "en")),
    retrievedFrom: days[0] ?? null,
    retrievedTo: days[days.length - 1] ?? null,
  };
}

/* ----------------------------------------------------------------- detail */

export interface BenchmarkPageInfo {
  slug: string;
  name: string;
  categoryLabel: string;
  status: BenchmarkStatus;
  measures: string;
  publisher: string;
  collectedBy: string | null;
  unit: BenchmarkUnit;
  unitLabel: string;
  conditions: string;
  methodNotes: string[];
  licenseName: string;
  licenseUrl: string | null;
  attribution: string;
  /** The benchmark's own page or leaderboard. */
  url: string;
  /** Who supplied the snapshot, e.g. "Epoch AI"; null when unknown. */
  sourceName: string | null;
  retrievedAt: string | null;
  dataDate: string | null;
  lastUpdate: DatedValue | null;
  resultCount: number;
  modelCount: number;
  externalModelCount: number;
  notes: string[];
}

export function benchmarkPageInfo(model: BenchmarkModel, b: ResolvedBenchmark): BenchmarkPageInfo {
  const d = b.definition;
  const source = d.source ? sourceNotes(model, { slugs: [d.slug] }).find((note) => note.id === d.source) : undefined;
  return {
    slug: d.slug,
    name: d.name,
    categoryLabel: CATEGORY_LABELS[d.category],
    status: d.status,
    measures: d.measures,
    publisher: d.publisher,
    collectedBy: d.collectedBy,
    unit: d.unit,
    unitLabel: d.unitLabel,
    conditions: d.conditions,
    methodNotes: [...d.methodNotes],
    licenseName: d.license.name,
    licenseUrl: d.license.url,
    attribution: d.attribution,
    url: d.url,
    sourceName: source?.name ?? null,
    retrievedAt: b.snapshot?.retrievedAt.slice(0, 10) ?? null,
    dataDate: b.snapshot?.dataDate ?? null,
    lastUpdate: newestResultDate(b.results),
    resultCount: b.results.length,
    modelCount: b.modelCount,
    externalModelCount: b.externalModelCount,
    notes: benchmarkNotes(b),
  };
}

/** First sentence of a plain-text paragraph. */
export function firstSentence(text: string): string {
  const match = /^(.+?[.!?])(?:\s|$)/.exec(text.trim());
  return match ? match[1] : text.trim();
}

/** Meta description: what the benchmark measures, then what the page lists. */
export function benchmarkMetaDescription(b: ResolvedBenchmark): string {
  const d = b.definition;
  const models = b.modelCount === 1 ? "1 model" : `${count(b.modelCount)} models`;
  return `${firstSentence(d.measures)} ${d.name} results for ${models}, with the source, date and conditions of each score, next to Tokscale usage.`;
}

export interface MappingSummary {
  results: number;
  matched: number;
  unmatched: number;
  /** Distinct Tokscale model keys. */
  models: number;
  /** Distinct source models that are not matched. */
  unmatchedModels: number;
  exact: number;
  normalized: number;
  manual: number;
  reviewed: number;
}

export function mappingSummary(results: readonly BenchmarkResult[]): MappingSummary {
  const keys = new Set<string>();
  const unmatchedModels = new Set<string>();
  const summary: MappingSummary = {
    results: results.length,
    matched: 0,
    unmatched: 0,
    models: 0,
    unmatchedModels: 0,
    exact: 0,
    normalized: 0,
    manual: 0,
    reviewed: 0,
  };
  for (const r of results) {
    if (r.modelKey && r.mapping) {
      summary.matched += 1;
      summary[r.mapping] += 1;
      if (r.reviewed) summary.reviewed += 1;
      keys.add(r.modelKey);
    } else {
      summary.unmatched += 1;
      unmatchedModels.add(r.externalModel);
    }
  }
  summary.models = keys.size;
  summary.unmatchedModels = unmatchedModels.size;
  return summary;
}

/* ---------------------------------------------------------- usage context */

export interface ModelContext {
  /** Active users in the last 30 days by model key; null when usage could not be loaded. */
  activeUsers: ReadonlyMap<string, number> | null;
  /** Display name and mark for a matched key. */
  ref(key: string): { name: string; logoUrl: string | null };
  /** Path of the Tokscale model page, or null when there is none. */
  href(key: string): string | null;
  /** Release date from the models.dev catalog, when listed there. */
  catalogRelease(key: string): string | null;
}

/**
 * Join keys with Tokscale data. `recent` is the 30-day model leaderboard and
 * `allTime` the all-time one (either can be null when usage failed to load).
 * A model page exists for a key with any Tokscale usage, a models.dev entry
 * or a matched benchmark result (pages without usage show benchmarks, prices
 * and specs only).
 */
export function buildModelContext(
  model: BenchmarkModel,
  recent: readonly Pick<ModelUsageRow, "key" | "name" | "logoUrl" | "activeUsers">[] | null,
  allTime: readonly Pick<ModelUsageRow, "key" | "name" | "logoUrl">[] | null = null,
): ModelContext {
  const refs = new Map<string, { name: string; logoUrl: string | null }>();
  for (const row of [...(recent ?? []), ...(allTime ?? [])]) {
    if (!refs.has(row.key)) refs.set(row.key, { name: row.name, logoUrl: row.logoUrl });
  }
  const activeUsers = recent ? new Map(recent.filter((row) => row.activeUsers > 0).map((row) => [row.key, row.activeUsers] as const)) : null;
  const researched = model.modelKeys();
  return {
    activeUsers,
    ref: (key) => {
      const known = refs.get(key);
      if (known) return known;
      const fallback = modelRef(key);
      return { name: fallback.name, logoUrl: fallback.logoUrl };
    },
    href: (key) => (refs.has(key) || researched.has(key) ? modelPath(key) : null),
    catalogRelease: (key) => model.spec(key)?.releaseDate ?? null,
  };
}

/* ------------------------------------------------------------ result rows */

export interface ScoreInterval {
  /** "CI" for a confidence interval, "s.e." for a standard error. */
  kind: "ci" | "se";
  text: string;
}

export interface ResultRow {
  /** Stable within one page: the position in the full, best-first list. */
  id: string;
  /** Rank by score among all results (ties share a rank). */
  rank: number;
  /** Rank among each model's best result; null when this is not a model's best. */
  bestRank: number | null;
  modelKey: string | null;
  /** Tokscale name of the matched model; null when unmatched. */
  modelName: string | null;
  logoUrl: string | null;
  href: string | null;
  /** The entry's name in the source. */
  sourceName: string;
  score: number;
  scoreText: string;
  interval: ScoreInterval | null;
  /** "2,925 votes", "11,182 sessions". */
  sample: string | null;
  conditions: string | null;
  note: string | null;
  /** "$4,719 per run". */
  cost: string | null;
  tokens: number | null;
  /** Basis of the cost and tokens, when the source gives one. */
  basis: "per task" | "per run" | null;
  date: DatedValue | null;
  sourceUrl: string;
  /** Active users in the last 30 days; null when none, unmatched, or unavailable. */
  activeUsers: number | null;
}

export interface ResultColumns {
  conditions: boolean;
  cost: boolean;
}

/** Standard error in the unit: "±1.2 pts" for percentages. */
export function formatStderr(unit: BenchmarkUnit, stderr: number): string {
  if (unit === "percent") return `\u00b1${(stderr * 100).toFixed(1)} pts`;
  if (unit === "index") return `\u00b1${stderr.toFixed(1)}`;
  if (unit === "rating") return `\u00b1${Math.round(stderr)}`;
  return `\u00b1${stderr.toFixed(2)}`;
}

export function scoreInterval(unit: BenchmarkUnit, r: BenchmarkResult): ScoreInterval | null {
  const ci = formatInterval(unit, r.ciLow, r.ciHigh);
  if (ci) return { kind: "ci", text: ci };
  if (r.stderr !== null && Number.isFinite(r.stderr)) return { kind: "se", text: formatStderr(unit, r.stderr) };
  return null;
}

/** "Max effort · 32K thinking budget · Claude Code"; null when the source reports no conditions. */
export function describeConditions(r: Pick<BenchmarkResult, "effort" | "thinkingBudget" | "agent">): string | null {
  const parts: string[] = [];
  if (r.effort) parts.push(r.effort === "none" ? effortLabel(r.effort) : `${effortLabel(r.effort)} effort`);
  if (r.thinkingBudget) parts.push(`${r.thinkingBudget} thinking budget`);
  if (r.agent) parts.push(r.agent);
  return parts.length > 0 ? parts.join(" \u00b7 ") : null;
}

function sampleText(r: BenchmarkResult): string | null {
  if (r.votes !== null) return `${count(r.votes)} ${r.votes === 1 ? "vote" : "votes"}`;
  if (r.sessions !== null) return `${count(r.sessions)} ${r.sessions === 1 ? "session" : "sessions"}`;
  return null;
}

/** The group a result belongs to for "best per model": the Tokscale key, else the source's model id. */
export function modelGroupKey(r: BenchmarkResult): string {
  return r.modelKey ? `key:${r.modelKey}` : `source:${r.externalModel}`;
}

/**
 * Positions of each model's best result in `results`. Matched results group
 * by Tokscale key (the same pick as BenchmarkModel.best), unmatched ones by
 * the source's model id, so unmatched entries stay listed too.
 */
export function bestResultIndexes(results: readonly BenchmarkResult[]): Set<number> {
  const best = new Map<string, number>();
  results.forEach((r, index) => {
    const group = modelGroupKey(r);
    const current = best.get(group);
    if (current === undefined || compareResults(r, results[current]) < 0) best.set(group, index);
  });
  return new Set(best.values());
}

/** Competition ranks ("1, 2, 2, 4") for scores where higher is better. */
export function competitionRanks(scores: readonly number[]): number[] {
  const sorted = [...scores].sort((a, b) => b - a);
  const firstIndex = new Map<number, number>();
  sorted.forEach((score, index) => {
    if (!firstIndex.has(score)) firstIndex.set(score, index);
  });
  return scores.map((score) => (firstIndex.get(score) ?? 0) + 1);
}

/** One row per result, best first, with ranks for both views and the usage join. */
export function buildResultRows(b: ResolvedBenchmark, ctx: ModelContext): ResultRow[] {
  const unit = b.definition.unit;
  const results = [...b.results].sort(compareResults);
  const ranks = competitionRanks(results.map((r) => r.score));
  const bestIndexes = bestResultIndexes(results);
  const bestPositions = [...bestIndexes].sort((x, y) => x - y);
  const bestRanks = competitionRanks(bestPositions.map((index) => results[index].score));
  const bestRankByIndex = new Map(bestPositions.map((index, position) => [index, bestRanks[position]] as const));

  return results.map((r, index) => {
    const key = r.modelKey;
    const ref = key ? ctx.ref(key) : null;
    return {
      id: String(index),
      rank: ranks[index],
      bestRank: bestRankByIndex.get(index) ?? null,
      modelKey: key,
      modelName: ref?.name ?? null,
      logoUrl: ref?.logoUrl ?? null,
      href: key ? ctx.href(key) : null,
      sourceName: r.name ?? r.externalId,
      score: r.score,
      scoreText: formatScore(unit, r.score),
      interval: scoreInterval(unit, r),
      sample: sampleText(r),
      conditions: describeConditions(r),
      note: r.notes,
      cost: r.costUsd !== null ? `${formatUsd(r.costUsd)}${r.costBasis ? ` ${r.costBasis}` : ""}` : null,
      tokens: r.tokens,
      basis: r.costBasis,
      date: resultDate(r),
      sourceUrl: r.sourceUrl,
      activeUsers: key && ctx.activeUsers ? ctx.activeUsers.get(key) ?? null : null,
    };
  });
}

/** Optional columns that have a value in at least one row. */
export function resultColumns(rows: readonly ResultRow[]): ResultColumns {
  return {
    conditions: rows.some((row) => row.conditions !== null || row.note !== null),
    cost: rows.some((row) => row.cost !== null || row.tokens !== null),
  };
}

/* --------------------------------------------------------------- timeline */

export interface TimelinePoint {
  key: string;
  name: string;
  href: string | null;
  score: number;
  scoreText: string;
  conditions: string | null;
  /** YYYY-MM-DD */
  releaseDate: string;
  /** "source": the benchmark's source lists the date; "catalog": models.dev does. */
  releaseFrom: "source" | "catalog";
  activeUsers: number | null;
}

export interface TimelineData {
  /** Best result per matched model with a known release date, oldest release first. */
  points: TimelinePoint[];
  /** Matched models in the benchmark. */
  models: number;
  /** Matched models left out because no release date is known. */
  undated: number;
  /** Points whose release date comes from models.dev. */
  catalogDates: number;
}

/** At least this many dated models before a chart is drawn. */
export const TIMELINE_MIN_POINTS = 3;

export function buildTimeline(b: ResolvedBenchmark, best: ReadonlyMap<string, BenchmarkResult>, ctx: ModelContext): TimelineData {
  const unit = b.definition.unit;
  const points: TimelinePoint[] = [];
  let undated = 0;
  for (const [key, r] of best) {
    const catalog = r.modelReleaseDate ? null : ctx.catalogRelease(key);
    const releaseDate = r.modelReleaseDate ?? catalog;
    if (!releaseDate) {
      undated += 1;
      continue;
    }
    points.push({
      key,
      name: ctx.ref(key).name,
      href: ctx.href(key),
      score: r.score,
      scoreText: formatScore(unit, r.score),
      conditions: describeConditions(r),
      releaseDate,
      releaseFrom: r.modelReleaseDate ? "source" : "catalog",
      activeUsers: ctx.activeUsers?.get(key) ?? null,
    });
  }
  points.sort((a, b) => (a.releaseDate < b.releaseDate ? -1 : a.releaseDate > b.releaseDate ? 1 : b.score - a.score || (a.key < b.key ? -1 : 1)));
  return { points, models: best.size, undated, catalogDates: points.filter((p) => p.releaseFrom === "catalog").length };
}

export interface AxisTick {
  value: number;
  label: string;
}

export interface Axis {
  min: number;
  max: number;
  ticks: AxisTick[];
}

export interface TimelineGeometry {
  /** Days since 1970-01-01. */
  x: Axis;
  y: Axis;
  /** Point positions as fractions of the plot: x from the left, y from the top. */
  positions: Array<{ x: number; y: number }>;
  /** The best score so far, as a step line through the record-setting points. */
  frontier: Array<{ x: number; y: number }>;
}

const MONTHS = ["Jan", "Feb", "Mar", "Apr", "May", "Jun", "Jul", "Aug", "Sep", "Oct", "Nov", "Dec"] as const;
const DAY_MS = 86_400_000;

/** "2026-10-06" -> days since 1970-01-01 (UTC). */
export function dayNumber(iso: string): number {
  return Math.round(Date.parse(`${iso.slice(0, 10)}T00:00:00Z`) / DAY_MS);
}

/** 1, 2, 2.5 or 5 times a power of ten, so `span` splits into at most `maxTicks` steps. */
export function niceStep(span: number, maxTicks = 5): number {
  if (!(span > 0) || !Number.isFinite(span)) return 1;
  const raw = span / Math.max(1, maxTicks);
  const magnitude = 10 ** Math.floor(Math.log10(raw));
  for (const m of [1, 2, 2.5, 5, 10]) {
    if (m * magnitude >= raw - 1e-12) return m * magnitude;
  }
  return 10 * magnitude;
}

/** Decimals needed to write `step` exactly: 10 -> 0, 2.5 -> 1, 0.05 -> 2. */
function decimalsFor(step: number): number {
  for (let digits = 0; digits < 6; digits++) {
    const scaled = step * 10 ** digits;
    if (Math.abs(Math.round(scaled) - scaled) < 1e-6) return digits;
  }
  return 6;
}

/** Axis label in the benchmark's unit: "60%", "150", "1,400", "+0.10". */
export function axisLabel(unit: BenchmarkUnit, value: number, step: number): string {
  const v = Math.abs(value) < step * 1e-6 ? 0 : value;
  if (unit === "percent") return `${(v * 100).toFixed(decimalsFor(step * 100))}%`;
  if (unit === "rating") return COUNT.format(Math.round(v));
  if (unit === "index") return v.toFixed(decimalsFor(step));
  const digits = Math.max(1, decimalsFor(step));
  const fixed = Math.abs(v).toFixed(digits);
  return v > 0 ? `+${fixed}` : v < 0 ? `\u2212${fixed}` : fixed;
}

/** Score axis padded to whole steps around the data. Percentages stay within 0-100%. */
export function scoreAxis(unit: BenchmarkUnit, values: readonly number[], maxTicks = 5): Axis {
  const finite = values.filter((v) => Number.isFinite(v));
  if (finite.length === 0) return { min: 0, max: 1, ticks: [] };
  let low = Math.min(...finite);
  let high = Math.max(...finite);
  if (high - low < 1e-9) {
    const pad = unit === "percent" ? 0.05 : Math.max(Math.abs(high) * 0.05, 1e-3);
    low -= pad;
    high += pad;
  }
  const step = niceStep(high - low, maxTicks);
  let min = Math.floor(low / step + 1e-9) * step;
  let max = Math.ceil(high / step - 1e-9) * step;
  if (unit === "percent") {
    min = Math.max(0, min);
    max = Math.min(1, max);
  }
  if (max <= min) max = min + step;
  const ticks: AxisTick[] = [];
  for (let i = 0; min + i * step <= max + step * 1e-6; i++) {
    const value = Number((min + i * step).toPrecision(12));
    ticks.push({ value, label: axisLabel(unit, value, step) });
  }
  return { min, max, ticks };
}

function monthStartDay(year: number, month: number): number {
  return Math.round(Date.UTC(year, month, 1) / DAY_MS);
}

/**
 * Date axis padded by a few percent, with ticks on month starts every 1, 2,
 * 3, 6 or 12 months (at most `maxTicks`). January ticks read as the year;
 * the first tick of a short axis also carries its year.
 */
export function dateAxis(minDay: number, maxDay: number, maxTicks = 6): Axis {
  const span = Math.max(maxDay - minDay, 30);
  const pad = Math.max(Math.round(span * 0.04), 7);
  const min = Math.min(minDay, maxDay) - pad;
  const max = Math.max(minDay, maxDay) + pad;
  const start = new Date(min * DAY_MS);
  const end = new Date(max * DAY_MS);
  const months = (end.getUTCFullYear() - start.getUTCFullYear()) * 12 + end.getUTCMonth() - start.getUTCMonth();
  const every = [1, 2, 3, 6, 12].find((m) => months / m <= maxTicks) ?? 12 * Math.ceil(months / (12 * maxTicks));
  // Ticks sit on multiples of the interval: Jan, Apr, Jul, Oct for 3 months; every Nth January past a year.
  const onTick = (year: number, month: number) => (every <= 12 ? month % every === 0 : month === 0 && year % (every / 12) === 0);
  const ticks: AxisTick[] = [];
  let year = start.getUTCFullYear();
  let month = start.getUTCMonth();
  for (let guard = 0; guard < 1200; guard++) {
    const day = monthStartDay(year, month);
    if (day > max) break;
    if (day >= min && onTick(year, month)) {
      const label = month === 0 ? String(year) : ticks.length === 0 ? `${MONTHS[month]} ${year}` : MONTHS[month];
      ticks.push({ value: day, label });
    }
    month += 1;
    if (month > 11) {
      month = 0;
      year += 1;
    }
  }
  return { min, max, ticks };
}

function fraction(value: number, min: number, max: number): number {
  if (!(max > min)) return 0.5;
  return Math.min(1, Math.max(0, (value - min) / (max - min)));
}

export function timelineGeometry(points: readonly TimelinePoint[], unit: BenchmarkUnit): TimelineGeometry {
  const days = points.map((p) => dayNumber(p.releaseDate));
  const x = days.length > 0 ? dateAxis(Math.min(...days), Math.max(...days)) : dateAxis(0, 30);
  const y = scoreAxis(
    unit,
    points.map((p) => p.score),
  );
  const positions = points.map((p, i) => ({ x: fraction(days[i], x.min, x.max), y: 1 - fraction(p.score, y.min, y.max) }));

  // Running best by release date: a step line that only moves up.
  const order = points.map((_, i) => i).sort((a, b) => days[a] - days[b] || points[b].score - points[a].score);
  const frontier: Array<{ x: number; y: number }> = [];
  let best = Number.NEGATIVE_INFINITY;
  for (const i of order) {
    if (points[i].score <= best) continue;
    best = points[i].score;
    if (frontier.length > 0) frontier.push({ x: positions[i].x, y: frontier[frontier.length - 1].y });
    frontier.push(positions[i]);
  }
  // Hold the last record until the newest release in the data.
  if (frontier.length > 0 && days.length > 0) {
    frontier.push({ x: fraction(Math.max(...days), x.min, x.max), y: frontier[frontier.length - 1].y });
  }
  return { x, y, positions, frontier };
}
