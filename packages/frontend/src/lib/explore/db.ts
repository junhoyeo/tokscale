/**
 * Database rollups for the explore surfaces.
 *
 * Reads daily_breakdown.source_breakdown (the JSON the submit route writes:
 * client -> { tokens, cost, ..., models: { rawModelId -> {...} } }) and folds it
 * into facts that ./rollup.ts aggregates. Accounts with
 * users.leaderboard_hidden = true are excluded in every query.
 *
 * Public loaders cache bounded partitions (revalidate 600 s). Shared cache
 * dependencies are resolved outside cache callbacks. The compute* functions
 * are exported uncached for scripts and measurements.
 */
import { createHash } from "node:crypto";
import { getExploreCacheGeneration } from "./revision";
import { partitionedCache } from "./partitionedCache";
import { foldModelMixes, type ModelMixRollup } from "@/lib/benchmarks/usage";
import { sql, type SQL } from "drizzle-orm";
import { db } from "@/lib/db";
import { isIdentifiedModelKey, modelKey } from "./modelKey";
import {
  buildModelCatalog,
  buildModelDetail,
  buildPeriodRollup,
  type CatalogEntry,
  type ModelCatalog,
  type ModelUserClientCell,
  type PeriodRollup,
  type UsageFact,
  type UserTotals,
} from "./rollup";
import type { ExplorePeriod, ModelDetail } from "./types";
import { maxAnchorDate, queryStart, resolveWindow, toIsoDate, type ExploreWindow } from "./window";

export { EXPLORE_REVALIDATE_SECONDS } from "./partitionedCache";
const CACHE_VERSION = "v6-partitions";

// Only concurrent work is shared in memory; completed results live exclusively
// in Next's tagged cache, so a privacy invalidation cannot miss a second TTL.
interface PendingLoad { readers: number; settled: boolean; result?: Promise<unknown> }
const pending = new Map<string, PendingLoad>();
async function shareColdLoad<T>(key: string, read: (computeOnce: (compute: () => Promise<T>) => Promise<T>) => Promise<T>): Promise<T> {
  const entry = pending.get(key) ?? { readers: 0, settled: true };
  entry.readers++;
  pending.set(key, entry);
  const release = () => {
    if (entry.readers === 0 && entry.settled && pending.get(key) === entry) pending.delete(key);
  };
  try {
    // Each reader still enters Next's cache to record its route dependencies.
    // A stale reader can finish BEFORE its background refresh; retain that
    // computation until it settles so subsequent requests share the SQL too.
    return await read((compute) => {
      if (!entry.result) {
        entry.settled = false;
        entry.result = compute().finally(() => { entry.settled = true; release(); });
      }
      return entry.result as Promise<T>;
    });
  } finally {
    entry.readers--;
    release();
  }
}

/** Visible (not leaderboard-hidden) day rows. */
const VISIBLE_DAYS = sql`
  daily_breakdown d
  INNER JOIN submissions s ON s.id = d.submission_id
  INNER JOIN users u ON u.id = s.user_id
`;

/**
 * Model cells of one client entry. Rows written before per-model maps existed
 * carry a single legacy `modelId` (or nothing): treat the client entry itself
 * as that one model, so its tokens still count (as "unknown" when unnamed).
 */
const MODEL_CELLS = sql`
  jsonb_each(
    CASE
      WHEN jsonb_typeof(c.value->'models') = 'object' AND c.value->'models' <> '{}'::jsonb
        THEN c.value->'models'
      ELSE jsonb_build_object(COALESCE(NULLIF(c.value->>'modelId', ''), 'unknown'), c.value)
    END
  ) AS m(key, value)
`;

const CELL_TOKENS = sql`COALESCE((m.value->>'tokens')::float8, 0)`;

function num(value: unknown): number {
  const n = typeof value === "number" ? value : Number(value);
  return Number.isFinite(n) ? n : 0;
}

function parseJsonArray<T>(value: unknown): T[] {
  const parsed = typeof value === "string" ? JSON.parse(value) : value;
  return Array.isArray(parsed) ? (parsed as T[]) : [];
}

interface Bounds {
  firstDate: string | null;
  latestDate: string | null;
}

export async function computeBounds(now: Date = new Date()): Promise<Bounds> {
  const rows = await db.execute<{ first_date: string | null; latest_date: string | null }>(sql`
    SELECT to_char(MIN(d.date), 'YYYY-MM-DD') AS first_date, to_char(MAX(d.date), 'YYYY-MM-DD') AS latest_date
    FROM ${VISIBLE_DAYS}
    WHERE u.leaderboard_hidden = false AND d.tokens > 0 AND d.date <= ${maxAnchorDate(now)}
  `);
  return { firstDate: rows[0]?.first_date ?? null, latestDate: rows[0]?.latest_date ?? null };
}

export async function computeWindow(period: ExplorePeriod, now: Date = new Date()): Promise<ExploreWindow> {
  const bounds = await computeBounds(now);
  return resolveWindow(period, bounds, toIsoDate(now.getTime()));
}

export async function computeCatalogEntries(): Promise<CatalogEntry[]> {
  const rows = await db.execute<Record<string, unknown>>(sql`
    SELECT c.key AS client, m.key AS model, to_char(MIN(d.date), 'YYYY-MM-DD') AS first_seen, SUM(${CELL_TOKENS}) AS tokens
    FROM ${VISIBLE_DAYS}
    CROSS JOIN LATERAL jsonb_each(d.source_breakdown) AS c(key, value)
    CROSS JOIN LATERAL ${MODEL_CELLS}
    WHERE u.leaderboard_hidden = false
      AND jsonb_typeof(d.source_breakdown) = 'object'
      AND jsonb_typeof(c.value) = 'object'
      AND ${CELL_TOKENS} > 0
    GROUP BY 1, 2
  `);
  return rows.map((r) => ({ client: String(r.client), model: String(r.model), firstSeen: String(r.first_seen), tokens: num(r.tokens) }));
}

export async function computeCatalog(): Promise<ModelCatalog> {
  return buildModelCatalog(await computeCatalogEntries());
}

export async function computeFacts(window: ExploreWindow): Promise<UsageFact[]> {
  const rows = await db.execute<Record<string, unknown>>(sql`
    SELECT s.user_id::text AS user_id, c.key AS client, m.key AS model, (d.date >= ${window.start}) AS current,
      SUM(${CELL_TOKENS}) AS tokens,
      SUM(COALESCE((m.value->>'cost')::float8, 0)) AS cost,
      SUM(COALESCE((m.value->>'input')::float8, 0)) AS input,
      SUM(COALESCE((m.value->>'output')::float8, 0)) AS output,
      SUM(COALESCE((m.value->>'cacheRead')::float8, 0)) AS cache_read,
      SUM(COALESCE((m.value->>'cacheWrite')::float8, 0)) AS cache_write,
      SUM(COALESCE((m.value->>'reasoning')::float8, 0)) AS reasoning
    FROM ${VISIBLE_DAYS}
    CROSS JOIN LATERAL jsonb_each(d.source_breakdown) AS c(key, value)
    CROSS JOIN LATERAL ${MODEL_CELLS}
    WHERE u.leaderboard_hidden = false
      AND d.date >= ${queryStart(window)} AND d.date <= ${window.end}
      AND jsonb_typeof(d.source_breakdown) = 'object'
      AND jsonb_typeof(c.value) = 'object'
      AND ${CELL_TOKENS} > 0
    GROUP BY 1, 2, 3, 4
  `);
  return rows.map((r) => ({
    userId: String(r.user_id),
    client: String(r.client),
    model: String(r.model),
    current: r.current === true || r.current === "t",
    tokens: num(r.tokens),
    cost: num(r.cost),
    input: num(r.input),
    output: num(r.output),
    cacheRead: num(r.cache_read),
    cacheWrite: num(r.cache_write),
    reasoning: num(r.reasoning),
  }));
}

export async function computeUserTotals(window: ExploreWindow): Promise<UserTotals[]> {
  const rows = await db.execute<Record<string, unknown>>(sql`
    SELECT u.id::text AS user_id, u.username, u.display_name, u.avatar_url,
      SUM(d.tokens)::float8 AS tokens,
      SUM(d.cost)::float8 AS cost,
      (COUNT(DISTINCT d.date) FILTER (WHERE d.tokens > 0))::int AS active_days,
      to_char(MAX(s.updated_at) AT TIME ZONE 'UTC', 'YYYY-MM-DD"T"HH24:MI:SS.MS"Z"') AS last_submitted_at
    FROM ${VISIBLE_DAYS}
    WHERE u.leaderboard_hidden = false AND d.date >= ${window.start} AND d.date <= ${window.end}
    GROUP BY u.id, u.username, u.display_name, u.avatar_url
  `);
  return rows.map((r) => ({
    userId: String(r.user_id),
    username: String(r.username),
    displayName: (r.display_name as string | null) ?? null,
    avatarUrl: (r.avatar_url as string | null) ?? null,
    tokens: num(r.tokens),
    cost: num(r.cost),
    activeDays: num(r.active_days),
    lastSubmittedAt: (r.last_submitted_at as string | null) ?? null,
  }));
}

interface PeriodData {
  rollup: PeriodRollup;
  mixes: ModelMixRollup;
}

async function computePeriodData(period: ExplorePeriod, catalog: ModelCatalog, now: Date = new Date()): Promise<PeriodData> {
  const window = await computeWindow(period, now);
  const facts = await computeFacts(window);
  const users = await computeUserTotals(window);
  return {
    rollup: buildPeriodRollup({ window, facts, users, catalog }),
    mixes: foldModelMixes(window, facts),
  };
}

export async function computePeriodRollup(period: ExplorePeriod, catalog?: ModelCatalog, now: Date = new Date()): Promise<PeriodRollup> {
  // This uncached entry point does not call any cached dependencies either.
  return (await computePeriodData(period, catalog ?? await computeCatalog(), now)).rollup;
}

type DetailQueryResult = {
  trend: unknown;
  cells: unknown;
  user_days: unknown;
};

function inList(values: string[]): SQL {
  return sql.join(
    values.map((v) => sql`${v}`),
    sql`, `,
  );
}

/**
 * The catalog covers all time, so a known model can have no usage in a
 * shorter period. Such a period has no detail: the model page then falls
 * back to all time instead of showing an empty window.
 */
function hasPeriodUsage(rollup: PeriodRollup, key: string): boolean {
  return rollup.models.some((m) => m.key === key);
}

export async function computeModelDetail(
  rawKey: string,
  period: ExplorePeriod,
  deps?: { catalog?: ModelCatalog; rollup?: PeriodRollup; now?: Date },
): Promise<ModelDetail | null> {
  const key = modelKey(rawKey);
  if (!isIdentifiedModelKey(key)) return null;
  const catalog = deps?.catalog ?? (await getModelCatalog());
  const aliases = catalog.aliases[key];
  if (!aliases || aliases.length === 0) return null;
  const rollup = deps?.rollup ?? (await getPeriodRollup(period));
  if (!hasPeriodUsage(rollup, key)) return null;
  // Same window as the rollup the summary comes from, even if that rollup is cached.
  const window = resolveWindow(period, { firstDate: rollup.coverage.start, latestDate: rollup.coverage.end }, rollup.coverage.end);

  const rows = await db.execute<DetailQueryResult>(sql`
    WITH cells AS MATERIALIZED (
      SELECT s.user_id, d.date, c.key AS client,
        ${CELL_TOKENS} AS tokens,
        COALESCE((m.value->>'cost')::float8, 0) AS cost,
        COALESCE((m.value->>'input')::float8, 0) AS input,
        COALESCE((m.value->>'output')::float8, 0) AS output,
        COALESCE((m.value->>'cacheRead')::float8, 0) AS cache_read,
        COALESCE((m.value->>'cacheWrite')::float8, 0) AS cache_write,
        COALESCE((m.value->>'reasoning')::float8, 0) AS reasoning
      FROM ${VISIBLE_DAYS}
      CROSS JOIN LATERAL jsonb_each(d.source_breakdown) AS c(key, value)
      CROSS JOIN LATERAL ${MODEL_CELLS}
      WHERE u.leaderboard_hidden = false
        AND d.date >= ${window.start} AND d.date <= ${window.end}
        AND jsonb_typeof(d.source_breakdown) = 'object'
        AND jsonb_typeof(c.value) = 'object'
        AND m.key IN (${inList(aliases)})
        AND ${CELL_TOKENS} > 0
    ),
    trend AS (
      SELECT to_char(date, 'YYYY-MM-DD') AS date, COUNT(DISTINCT user_id)::int AS "activeUsers", SUM(tokens) AS tokens
      FROM cells GROUP BY date
    ),
    user_client AS (
      SELECT u.username, u.display_name AS "displayName", u.avatar_url AS "avatarUrl", cells.client,
        SUM(tokens) AS tokens, SUM(cost) AS cost, SUM(input) AS input, SUM(output) AS output,
        SUM(cache_read) AS "cacheRead", SUM(cache_write) AS "cacheWrite", SUM(reasoning) AS reasoning
      FROM cells INNER JOIN users u ON u.id = cells.user_id
      GROUP BY u.id, u.username, u.display_name, u.avatar_url, cells.client
    ),
    user_days AS (
      SELECT u.username, COUNT(DISTINCT cells.date)::int AS days
      FROM cells INNER JOIN users u ON u.id = cells.user_id
      GROUP BY u.id, u.username
    )
    SELECT
      COALESCE((SELECT json_agg(trend) FROM trend), '[]'::json) AS trend,
      COALESCE((SELECT json_agg(user_client) FROM user_client), '[]'::json) AS cells,
      COALESCE((SELECT json_agg(user_days) FROM user_days), '[]'::json) AS user_days
  `);
  const result = rows[0];
  const trend = parseJsonArray<{ date: string; activeUsers: number; tokens: number }>(result?.trend).map((p) => ({
    date: p.date,
    activeUsers: num(p.activeUsers),
    tokens: num(p.tokens),
  }));
  const cells = parseJsonArray<ModelUserClientCell>(result?.cells).map((c) => ({
    username: String(c.username),
    displayName: c.displayName ?? null,
    avatarUrl: c.avatarUrl ?? null,
    client: String(c.client),
    tokens: num(c.tokens),
    cost: num(c.cost),
    input: num(c.input),
    output: num(c.output),
    cacheRead: num(c.cacheRead),
    cacheWrite: num(c.cacheWrite),
    reasoning: num(c.reasoning),
  }));
  const userDays: Record<string, number> = Object.create(null);
  for (const row of parseJsonArray<{ username: string; days: number }>(result?.user_days)) {
    userDays[String(row.username)] = num(row.days);
  }
  return buildModelDetail({ key, window, rollup, catalog, trend, cells, userDays });
}

async function getCatalogForGeneration(generation: string): Promise<ModelCatalog> {
  return shareColdLoad(`${generation}:catalog`, (once) =>
    partitionedCache(() => once(computeCatalog), ["explore", CACHE_VERSION, generation, "catalog"]));
}

export async function getModelCatalog(): Promise<ModelCatalog> {
  return getCatalogForGeneration(await getExploreCacheGeneration());
}

async function getPeriodData(period: ExplorePeriod, generation?: string): Promise<PeriodData> {
  const version = generation ?? await getExploreCacheGeneration();
  // Never put this lookup inside the cached callback: Next bypasses nested
  // unstable_cache reads, even when a catalog entry is already warm.
  return shareColdLoad(`${version}:period:${period}`, async (once) => {
    const catalog = await getCatalogForGeneration(version);
    return partitionedCache(
      () => once(() => computePeriodData(period, catalog)),
      ["explore", CACHE_VERSION, version, "period", period],
    );
  });
}

export async function getPeriodRollup(period: ExplorePeriod): Promise<PeriodRollup> {
  return (await getPeriodData(period)).rollup;
}

/** Pricing uses the same facts as leaderboards; do not scan them a second time. */
export async function getPeriodModelMixes(period: ExplorePeriod): Promise<ModelMixRollup> {
  return (await getPeriodData(period)).mixes;
}

export async function getModelDetailCached(rawKey: string, period: ExplorePeriod): Promise<ModelDetail | null> {
  const key = modelKey(rawKey);
  if (!isIdentifiedModelKey(key)) return null;
  const generation = await getExploreCacheGeneration();
  const catalog = await getCatalogForGeneration(generation);
  if (!catalog.aliases[key]?.length) return null;
  const { rollup } = await getPeriodData(period, generation);
  // No usage in this period: skip the detail scan and its cache entry.
  if (!hasPeriodUsage(rollup, key)) return null;
  // A dependency can return its previous snapshot during ordinary SWR. Bind
  // the composed detail to that exact snapshot, not just the period name, so
  // a later fresh rollup cannot hit a detail built with the older summary.
  const dependencies = createHash("sha256").update(JSON.stringify({ catalog, rollup })).digest("hex");
  return shareColdLoad(`${generation}:model:${key}:${period}:${dependencies}`, (once) => partitionedCache(
    () => once(() => computeModelDetail(key, period, { catalog, rollup })),
    ["explore", CACHE_VERSION, generation, "model", key, period, dependencies],
  ));
}

/**
 * Benchmark results only need recent model/account pairs, not detailed people,
 * agents, previous-period facts, or an all-time catalog. Canonicalize before
 * counting so two raw aliases used by one account still count as one user.
 */
export interface ModelUsageCounts {
  start: string;
  end: string;
  counts: Record<string, number>;
}

export async function computeModelUsageCounts(period: ExplorePeriod = "30d"): Promise<ModelUsageCounts> {
  const window = await computeWindow(period);
  const rows = await db.execute<{ user_id: string; model: string }>(sql`
    SELECT DISTINCT s.user_id::text AS user_id, m.key AS model
    FROM ${VISIBLE_DAYS}
    CROSS JOIN LATERAL jsonb_each(d.source_breakdown) AS c(key, value)
    CROSS JOIN LATERAL ${MODEL_CELLS}
    WHERE u.leaderboard_hidden = false
      AND d.date >= ${window.start} AND d.date <= ${window.end}
      AND jsonb_typeof(d.source_breakdown) = 'object'
      AND jsonb_typeof(c.value) = 'object'
      AND ${CELL_TOKENS} > 0
  `);
  const accounts = new Map<string, Set<string>>();
  for (const row of rows) {
    const key = modelKey(row.model);
    if (!isIdentifiedModelKey(key)) continue;
    if (!accounts.has(key)) accounts.set(key, new Set());
    accounts.get(key)!.add(row.user_id);
  }
  return {
    start: window.start,
    end: window.end,
    counts: Object.fromEntries([...accounts].map(([key, users]) => [key, users.size])),
  };
}

export async function getModelUsageCounts(): Promise<ModelUsageCounts> {
  const generation = await getExploreCacheGeneration();
  return shareColdLoad(`${generation}:model-usage-counts:30d`, (once) => partitionedCache(
    () => once(() => computeModelUsageCounts("30d")),
    ["explore", CACHE_VERSION, generation, "model-usage-counts", "30d"],
  ));
}
