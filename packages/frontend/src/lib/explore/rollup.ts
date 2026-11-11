/**
 * Pure aggregation for the explore rollups. The database layer (./db.ts) turns
 * daily_breakdown.source_breakdown into flat facts; everything here is plain
 * arithmetic so it can be unit-tested without Postgres.
 */
import { agentKey, agentRef, modelRef } from "./modelMeta";
import { isIdentifiedModelKey, modelKey } from "./modelKey";
import type {
  AgentRef,
  AgentUsageRow,
  Coverage,
  ExploreSort,
  HumanRow,
  ModelDetail,
  ModelRef,
  ModelUsageRow,
  TokenMix,
} from "./types";
import { adoption, enumerateDates, identifiedShare, isNewInWindow, type ExploreWindow } from "./window";

export interface TokenSums extends TokenMix {
  tokens: number;
  cost: number;
}

/** One (user, client, raw model) cell summed over the current or previous window. */
export interface UsageFact extends TokenSums {
  userId: string;
  client: string;
  model: string;
  current: boolean;
}

/** Scalar per-user totals for the current window (daily_breakdown.tokens / cost). */
export interface UserTotals {
  userId: string;
  username: string;
  displayName: string | null;
  avatarUrl: string | null;
  tokens: number;
  cost: number;
  activeDays: number;
  /** ISO timestamp of the user's newest submission. */
  lastSubmittedAt: string | null;
}

/** All-time first appearance of one (client, raw model) pair among visible accounts. */
export interface CatalogEntry {
  client: string;
  model: string;
  firstSeen: string;
  tokens: number;
}

export interface ModelCatalog {
  /** Canonical key -> earliest date with usage. */
  modelFirstSeen: Record<string, string>;
  /** Agent key -> earliest date with usage. */
  agentFirstSeen: Record<string, string>;
  /** Canonical key -> raw ids that map to it, most-used first (all time). */
  aliases: Record<string, string[]>;
}

export interface PeriodRollup {
  coverage: Coverage;
  /** Ranked by active users. */
  models: ModelUsageRow[];
  /** Ranked by active users. */
  agents: AgentUsageRow[];
  /** Ranked by tokens. */
  humans: HumanRow[];
}

export const TOP_N = 3;

export function emptySums(): TokenSums {
  return { tokens: 0, cost: 0, input: 0, output: 0, cacheRead: 0, cacheWrite: 0, reasoning: 0 };
}

export function addSums(target: TokenSums, source: TokenSums): void {
  target.tokens += source.tokens;
  target.cost += source.cost;
  target.input += source.input;
  target.output += source.output;
  target.cacheRead += source.cacheRead;
  target.cacheWrite += source.cacheWrite;
  target.reasoning += source.reasoning;
}

/** Token mix as shares of the sum of the five buckets (0..1 each). */
export function tokenMixShares(sums: TokenMix): TokenMix {
  const total = sums.input + sums.output + sums.cacheRead + sums.cacheWrite + sums.reasoning;
  if (total <= 0) return { input: 0, output: 0, cacheRead: 0, cacheWrite: 0, reasoning: 0 };
  return {
    input: sums.input / total,
    output: sums.output / total,
    cacheRead: sums.cacheRead / total,
    cacheWrite: sums.cacheWrite / total,
    reasoning: sums.reasoning / total,
  };
}

function mapGet<K, V>(map: Map<K, V>, key: K, create: () => V): V {
  let value = map.get(key);
  if (value === undefined) {
    value = create();
    map.set(key, value);
  }
  return value;
}

/** Top entries of a token map as (key, share of `total`) pairs. */
export function topShares(tokensByKey: Map<string, number>, total: number, n: number = TOP_N): Array<{ key: string; share: number }> {
  return [...tokensByKey.entries()]
    .filter(([, tokens]) => tokens > 0)
    .sort((a, b) => b[1] - a[1] || (a[0] < b[0] ? -1 : a[0] > b[0] ? 1 : 0))
    .slice(0, n)
    .map(([key, tokens]) => ({ key, share: total > 0 ? tokens / total : 0 }));
}

/** Sort by the requested metric and renumber ranks from 1. Ties: the other metric, then key. */
export function rankRows<T extends { activeUsers: number; tokens: number; rank: number; key: string }>(rows: T[], sort: ExploreSort = "users"): T[] {
  const sorted = [...rows].sort((a, b) => {
    const primary = sort === "tokens" ? b.tokens - a.tokens : b.activeUsers - a.activeUsers;
    if (primary !== 0) return primary;
    const secondary = sort === "tokens" ? b.activeUsers - a.activeUsers : b.tokens - a.tokens;
    if (secondary !== 0) return secondary;
    return a.key < b.key ? -1 : a.key > b.key ? 1 : 0;
  });
  return sorted.map((row, i) => ({ ...row, rank: i + 1 }));
}

export function pageRows<T>(rows: T[], limit: number | undefined, offset: number | undefined): T[] {
  const start = Math.max(0, Math.floor(offset ?? 0));
  const size = Math.max(0, Math.floor(limit ?? 50));
  return rows.slice(start, start + size);
}

/** Fold catalog cells into canonical keys. Unidentified keys keep no aliases. */
export function buildModelCatalog(entries: CatalogEntry[]): ModelCatalog {
  const modelFirstSeen: Record<string, string> = Object.create(null);
  const agentFirstSeen: Record<string, string> = Object.create(null);
  const aliasTokens = new Map<string, Map<string, number>>();
  for (const entry of entries) {
    if (entry.tokens <= 0) continue;
    const key = modelKey(entry.model);
    const agent = agentKey(entry.client);
    if (!modelFirstSeen[key] || entry.firstSeen < modelFirstSeen[key]) modelFirstSeen[key] = entry.firstSeen;
    if (!agentFirstSeen[agent] || entry.firstSeen < agentFirstSeen[agent]) agentFirstSeen[agent] = entry.firstSeen;
    const raw = mapGet(aliasTokens, key, () => new Map<string, number>());
    raw.set(entry.model, (raw.get(entry.model) ?? 0) + entry.tokens);
  }
  const aliases: Record<string, string[]> = Object.create(null);
  for (const [key, raw] of aliasTokens) {
    aliases[key] = [...raw.entries()].sort((a, b) => b[1] - a[1] || (a[0] < b[0] ? -1 : 1)).map(([id]) => id);
  }
  return { modelFirstSeen, agentFirstSeen, aliases };
}

interface EntityAccumulator {
  users: Set<string>;
  previousUsers: Set<string>;
  sums: TokenSums;
  /** Tokens by the other dimension (agent for a model, model for an agent). */
  split: Map<string, number>;
}

function newAccumulator(): EntityAccumulator {
  return { users: new Set(), previousUsers: new Set(), sums: emptySums(), split: new Map() };
}

export interface BuildRollupInput {
  window: ExploreWindow;
  facts: UsageFact[];
  users: UserTotals[];
  catalog: ModelCatalog;
  /** Cap on human rows kept (the rest are only counted in coverage). */
  humansLimit?: number;
}

export function buildPeriodRollup({ window, facts, users, catalog, humansLimit = 1000 }: BuildRollupInput): PeriodRollup {
  const models = new Map<string, EntityAccumulator>();
  const agents = new Map<string, EntityAccumulator>();
  const userModels = new Map<string, Map<string, number>>();
  const userAgents = new Map<string, Map<string, number>>();
  const keyCache = new Map<string, string>();
  // Scalar totals also include legacy days with no source_breakdown. Those
  // tokens are unidentified, not absent from the coverage denominator.
  const totalTokens = users.reduce((sum, user) => sum + Math.max(0, user.tokens), 0);
  let identifiedTokens = 0;

  for (const fact of facts) {
    if (fact.tokens <= 0) continue;
    let key = keyCache.get(fact.model);
    if (key === undefined) {
      key = modelKey(fact.model);
      keyCache.set(fact.model, key);
    }
    const identified = isIdentifiedModelKey(key);
    const agent = agentKey(fact.client);
    const agentAcc = mapGet(agents, agent, newAccumulator);
    const modelAcc = identified ? mapGet(models, key, newAccumulator) : null;

    if (!fact.current) {
      agentAcc.previousUsers.add(fact.userId);
      modelAcc?.previousUsers.add(fact.userId);
      continue;
    }

    agentAcc.users.add(fact.userId);
    addSums(agentAcc.sums, fact);
    mapGet(userAgents, fact.userId, () => new Map()).set(agent, (userAgents.get(fact.userId)?.get(agent) ?? 0) + fact.tokens);

    if (modelAcc) {
      identifiedTokens += fact.tokens;
      modelAcc.users.add(fact.userId);
      addSums(modelAcc.sums, fact);
      modelAcc.split.set(agent, (modelAcc.split.get(agent) ?? 0) + fact.tokens);
      agentAcc.split.set(key, (agentAcc.split.get(key) ?? 0) + fact.tokens);
      mapGet(userModels, fact.userId, () => new Map()).set(key, (userModels.get(fact.userId)?.get(key) ?? 0) + fact.tokens);
    }
  }

  const activeUsers = users.filter((u) => u.tokens > 0);
  const accounts = activeUsers.length;
  let lastDataAt: string | null = null;
  for (const user of activeUsers) {
    if (user.lastSubmittedAt && (lastDataAt === null || user.lastSubmittedAt > lastDataAt)) lastDataAt = user.lastSubmittedAt;
  }

  const coverage: Coverage = {
    period: window.period,
    start: window.start,
    end: window.end,
    accounts,
    modelIdentifiedShare: identifiedShare(identifiedTokens, totalTokens),
    lastDataAt,
  };
  const hasPrevious = window.previous !== null;

  const modelRows: ModelUsageRow[] = [];
  for (const [key, acc] of models) {
    if (acc.users.size === 0) continue;
    modelRows.push({
      ...modelRef(key),
      rank: 0,
      activeUsers: acc.users.size,
      previousActiveUsers: hasPrevious ? acc.previousUsers.size : null,
      adoption: adoption(acc.users.size, accounts),
      tokens: acc.sums.tokens,
      estimatedCost: acc.sums.cost,
      isNew: isNewInWindow(window, catalog.modelFirstSeen[key] ?? null),
      topAgents: topShares(acc.split, acc.sums.tokens).map(({ key: agent, share }) => ({ ...agentRef(agent), share })),
    });
  }

  const agentRows: AgentUsageRow[] = [];
  for (const [key, acc] of agents) {
    if (acc.users.size === 0) continue;
    agentRows.push({
      ...agentRef(key),
      rank: 0,
      activeUsers: acc.users.size,
      previousActiveUsers: hasPrevious ? acc.previousUsers.size : null,
      adoption: adoption(acc.users.size, accounts),
      tokens: acc.sums.tokens,
      estimatedCost: acc.sums.cost,
      isNew: isNewInWindow(window, catalog.agentFirstSeen[key] ?? null),
      topModels: topShares(acc.split, acc.sums.tokens).map(({ key: model, share }) => ({ ...modelRef(model), share })),
    });
  }

  const humans: HumanRow[] = [...activeUsers]
    .sort((a, b) => b.tokens - a.tokens || b.cost - a.cost || (a.username.toLowerCase() < b.username.toLowerCase() ? -1 : 1))
    .slice(0, humansLimit)
    .map((user, i) => {
      const byModel = userModels.get(user.userId) ?? new Map<string, number>();
      const byAgent = userAgents.get(user.userId) ?? new Map<string, number>();
      return {
        rank: i + 1,
        username: user.username,
        displayName: user.displayName,
        avatarUrl: user.avatarUrl,
        tokens: user.tokens,
        estimatedCost: user.cost,
        activeDays: user.activeDays,
        topModels: topShares(byModel, 1).map(({ key }) => modelRef(key)),
        topAgents: topShares(byAgent, 1).map(({ key }) => agentRef(key)),
      };
    });

  return { coverage, models: rankRows(modelRows, "users"), agents: rankRows(agentRows, "users"), humans };
}

/** One (user, client) cell of a single model's usage in the current window. */
export interface ModelUserClientCell extends TokenSums {
  username: string;
  displayName: string | null;
  avatarUrl: string | null;
  client: string;
}

export interface ModelDetailInput {
  key: string;
  window: ExploreWindow;
  rollup: PeriodRollup;
  catalog: ModelCatalog;
  trend: Array<{ date: string; activeUsers: number; tokens: number }>;
  cells: ModelUserClientCell[];
  /** Distinct days each user used the model in the window. */
  userDays: Record<string, number>;
  humansLimit?: number;
  relatedLimit?: number;
}

export function buildModelDetail(input: ModelDetailInput): ModelDetail {
  const { key, window, rollup, catalog } = input;
  const ref: ModelRef = modelRef(key);
  const row = rollup.models.find((m) => m.key === key) ?? null;

  const mix = emptySums();
  const agentUsers = new Map<string, Set<string>>();
  const agentTokens = new Map<string, number>();
  const humans = new Map<string, { info: ModelUserClientCell; sums: TokenSums; agents: Map<string, number> }>();
  for (const cell of input.cells) {
    if (cell.tokens <= 0) continue;
    addSums(mix, cell);
    const agent = agentKey(cell.client);
    mapGet(agentUsers, agent, () => new Set()).add(cell.username);
    agentTokens.set(agent, (agentTokens.get(agent) ?? 0) + cell.tokens);
    const human = mapGet(humans, cell.username, () => ({ info: cell, sums: emptySums(), agents: new Map<string, number>() }));
    addSums(human.sums, cell);
    human.agents.set(agent, (human.agents.get(agent) ?? 0) + cell.tokens);
  }

  const byDate = new Map(input.trend.map((p) => [p.date, p]));
  const trend = enumerateDates(window.start, window.end).map((date) => ({
    date,
    activeUsers: byDate.get(date)?.activeUsers ?? 0,
    tokens: byDate.get(date)?.tokens ?? 0,
  }));

  const rollupHumans = new Map(rollup.humans.map((h) => [h.username, h]));
  const humanRows: HumanRow[] = [...humans.values()]
    .sort((a, b) => b.sums.tokens - a.sums.tokens || (a.info.username < b.info.username ? -1 : 1))
    .slice(0, input.humansLimit ?? 10)
    .map((h, i) => ({
      rank: i + 1,
      username: h.info.username,
      displayName: h.info.displayName,
      avatarUrl: h.info.avatarUrl,
      tokens: h.sums.tokens,
      estimatedCost: h.sums.cost,
      activeDays: input.userDays[h.info.username] ?? 0,
      topModels: rollupHumans.get(h.info.username)?.topModels ?? [ref],
      topAgents: topShares(h.agents, 1).map(({ key: agent }) => agentRef(agent)),
    }));

  const agents: Array<AgentRef & { activeUsers: number; share: number }> = [...agentTokens.entries()]
    .sort((a, b) => b[1] - a[1] || (a[0] < b[0] ? -1 : 1))
    .map(([agent, tokens]) => ({
      ...agentRef(agent),
      activeUsers: agentUsers.get(agent)?.size ?? 0,
      share: mix.tokens > 0 ? tokens / mix.tokens : 0,
    }));

  const related = ref.labSlug
    ? rollup.models.filter((m) => m.labSlug === ref.labSlug && m.key !== key).slice(0, input.relatedLimit ?? 6)
    : [];

  return {
    model: { ...ref, aliases: (catalog.aliases[key] ?? []).filter((raw) => raw !== key).slice(0, 20) },
    coverage: rollup.coverage,
    summary: {
      rank: row?.rank ?? null,
      activeUsers: row?.activeUsers ?? 0,
      previousActiveUsers: row?.previousActiveUsers ?? (window.previous ? 0 : null),
      adoption: row?.adoption ?? 0,
      tokens: row?.tokens ?? 0,
      estimatedCost: row?.estimatedCost ?? 0,
      tokenMix: tokenMixShares(mix),
      firstSeen: catalog.modelFirstSeen[key] ?? null,
    },
    trend,
    agents,
    humans: humanRows,
    related,
  };
}

/** Keys with at least `minUsers` all-time active users, by active users. */
export function indexableModelKeys(allTime: PeriodRollup, minUsers: number): string[] {
  return allTime.models.filter((m) => m.activeUsers >= minUsers).map((m) => m.key);
}
