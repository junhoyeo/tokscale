/**
 * Explore data entry point. Pages import only from here.
 *
 * With DATABASE_URL set, every function reads cached rollups of
 * daily_breakdown (see ./db.ts, revalidated every 10 minutes). Development
 * and tests without a database can use fixtures. Production fails clearly
 * when the database is not configured; it never publishes invented activity.
 */
import { fixtureAgents, fixtureHumans, fixtureModelDetail, fixtureModels, fixtureSnapshot, fixtureCoverage } from "./fixtures";
import { isIdentifiedModelKey, modelKey } from "./modelKey";
import { pageRows, rankRows, indexableModelKeys } from "./rollup";
import type {
  AgentUsageRow,
  CommunitySnapshot,
  Coverage,
  HumanRow,
  LeaderboardQuery,
  ModelDetail,
  ModelUsageRow,
  Ranked,
  ExplorePeriod,
} from "./types";

export * from "./types";

/** Models need this many distinct all-time accounts before they get an indexable page. */
export const MIN_INDEXABLE_MODEL_USERS = 5;
const SNAPSHOT_SIZE = 5;

function hasDatabase(): boolean {
  if (process.env.DATABASE_URL) return true;
  if (process.env.NODE_ENV === "development" || process.env.NODE_ENV === "test") return false;
  throw new Error("DATABASE_URL is required for public analytics outside development and tests");
}

function loadDb() {
  return import("./db");
}

export async function getModelLeaderboard(q: LeaderboardQuery = {}): Promise<Ranked<ModelUsageRow>> {
  const period = q.period ?? "30d";
  if (!hasDatabase()) {
    return { rows: pageRows(rankRows(fixtureModels(period), q.sort), q.limit, q.offset), coverage: fixtureCoverage(period) };
  }
  const rollup = await (await loadDb()).getPeriodRollup(period);
  return { rows: pageRows(rankRows(rollup.models, q.sort), q.limit, q.offset), coverage: rollup.coverage };
}

export async function getAgentLeaderboard(q: LeaderboardQuery = {}): Promise<Ranked<AgentUsageRow>> {
  const period = q.period ?? "30d";
  if (!hasDatabase()) {
    return { rows: pageRows(rankRows(fixtureAgents(period), q.sort), q.limit, q.offset), coverage: fixtureCoverage(period) };
  }
  const rollup = await (await loadDb()).getPeriodRollup(period);
  return { rows: pageRows(rankRows(rollup.agents, q.sort), q.limit, q.offset), coverage: rollup.coverage };
}

/** Humans are always ranked by tokens in the period; `sort` is ignored. */
export async function getHumansLeaderboard(q: LeaderboardQuery = {}): Promise<Ranked<HumanRow>> {
  const period = q.period ?? "30d";
  if (!hasDatabase()) {
    return { rows: pageRows(fixtureHumans(period), q.limit, q.offset), coverage: fixtureCoverage(period) };
  }
  const rollup = await (await loadDb()).getPeriodRollup(period);
  return { rows: pageRows(rollup.humans, q.limit, q.offset), coverage: rollup.coverage };
}

/**
 * The profile the home page features: `username`'s row, with their real rank in
 * `period`. The period rollup keeps the top 1,000 humans by tokens. When the
 * account is not among them, which includes having no public usage in the period,
 * the first-ranked human takes its place so the section is not empty.
 */
export async function getFeaturedHuman(
  username: string,
  period: ExplorePeriod = "30d",
): Promise<{ human: HumanRow | null; coverage: Coverage }> {
  const { humans, coverage } = hasDatabase()
    ? await (await loadDb()).getPeriodRollup(period)
    : { humans: fixtureHumans(period), coverage: fixtureCoverage(period) };
  const wanted = username.toLowerCase();
  return { human: humans.find((h) => h.username.toLowerCase() === wanted) ?? humans[0] ?? null, coverage };
}

/**
 * Detail for one model. `key` may be any spelling of the model id; the result's
 * `model.key` is canonical, so a page can redirect when they differ. Returns
 * null for unknown or unidentified models, and for a known model without
 * public usage in `period` (the model page then falls back to all time).
 */
export async function getModelDetail(key: string, period: ExplorePeriod = "30d"): Promise<ModelDetail | null> {
  if (!hasDatabase()) return fixtureModelDetail(key, period);
  return (await loadDb()).getModelDetailCached(key, period);
}

/**
 * A model's leaderboard row in `period`: its name, lab, active users and tokens.
 * It comes from the period rollup the leaderboards already cache, so unlike
 * getModelDetail it never scans usage. Null where getModelDetail is null.
 */
export async function getModelSummary(key: string, period: ExplorePeriod = "30d"): Promise<ModelUsageRow | null> {
  if (!hasDatabase()) return fixtureModels(period).find((m) => m.key === key) ?? null;
  const canonical = modelKey(key);
  if (!isIdentifiedModelKey(canonical)) return null;
  const rollup = await (await loadDb()).getPeriodRollup(period);
  return rollup.models.find((m) => m.key === canonical) ?? null;
}

export async function getCommunitySnapshot(period: ExplorePeriod = "30d"): Promise<CommunitySnapshot> {
  if (!hasDatabase()) return fixtureSnapshot(period);
  const rollup = await (await loadDb()).getPeriodRollup(period);
  return {
    humans: rollup.humans.slice(0, SNAPSHOT_SIZE),
    models: rollup.models.slice(0, SNAPSHOT_SIZE),
    agents: rollup.agents.slice(0, SNAPSHOT_SIZE),
    coverage: rollup.coverage,
  };
}

/** Canonical keys of models with enough data for an indexable page. */
export async function listModelKeys(): Promise<string[]> {
  if (!hasDatabase()) return fixtureModels("all").map((m) => m.key);
  const rollup = await (await loadDb()).getPeriodRollup("all");
  return indexableModelKeys(rollup, MIN_INDEXABLE_MODEL_USERS);
}
