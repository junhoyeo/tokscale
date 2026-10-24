import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import * as fs from "node:fs/promises";
import { createRequire } from "node:module";
import type { SQL } from "drizzle-orm";

await vi.hoisted(async () => {
  const { AsyncLocalStorage } = await import("node:async_hooks");
  Object.assign(globalThis, { AsyncLocalStorage });
});
const state = vi.hoisted(() => ({ hidden: false, deleted: false, large: false, revision: 0, revisionAvailable: true, revisionError: false, queries: [] as string[],
  pause: null as null | { match: string; entered: () => void; resume: Promise<void> },
}));
vi.mock("@/lib/groups/cache", () => ({ revalidateUserGroupLeaderboards: vi.fn(async () => {}) }));
vi.mock("@/lib/auth/requestSession", () => ({ getSessionFromRequest: async () => ({ id: "target", username: "target" }) }));
vi.mock("@/lib/auth/session", () => ({ clearSession: async () => {} }));
vi.mock("@/lib/auth/personalTokens", () => ({ authenticatePersonalToken: vi.fn() }));
vi.mock("@/lib/db", async () => {
  const schema = await import("@/lib/db/schema");
  const { PgDialect } = await import("drizzle-orm/pg-core");
  const dialect = new PgDialect();
  const usernames = () => state.hidden || state.deleted ? ["visible"] : ["target", "visible"];
  const execute = async (query: SQL) => {
    const text = dialect.sqlToQuery(query).sql;
    state.queries.push(text);
    if (text.includes("SELECT revision::text")) {
      if (state.revisionError) throw new Error('relation "explore_cache_revision" does not exist');
      return state.revisionAvailable ? [{ revision: String(state.revision) }] : [];
    }
    if (text.includes("UPDATE explore_cache_revision")) {
      if (!state.revisionAvailable) return [];
      state.revision++;
      return [{ revision: String(state.revision) }];
    }
    if (state.large) {
      if (text.includes("AS first_seen")) return Array.from({ length: 12000 }, (_, i) => ({ client: "codex", model: `synthetic-model-${i}`, first_seen: "2026-10-01", tokens: 500 }));
      if (text.includes("s.user_id::text AS user_id")) return Array.from({ length: 60000 }, (_, i) => ({
        user_id: String(i % 1000), client: "codex", model: `synthetic-model-${Math.floor(i / 5)}`, current: true,
        tokens: 100, cost: 1, input: 100, output: 0, cache_read: 0, cache_write: 0, reasoning: 0,
      }));
      if (text.includes("u.id::text AS user_id")) return Array.from({ length: 1000 }, (_, i) => ({
        user_id: String(i), username: `synthetic-person-${i}`, display_name: `Person ${i}`, avatar_url: null,
        tokens: 6000, cost: 60, active_days: 1, last_submitted_at: "2026-10-19T00:00:00.000Z",
      }));
    }
    if (text.includes("AS first_date")) return [{ first_date: "2026-10-01", latest_date: "2026-10-19" }];
    if (text.includes("AS first_seen")) return [{ client: "codex", model: "gpt-5.5", first_seen: "2026-10-01", tokens: usernames().length * 100 }];
    if (text.includes("SELECT DISTINCT")) {
      const rows = usernames().flatMap(user_id => [
        { user_id, model: "gpt-5.5" }, { user_id, model: "openai/gpt-5.5" }, { user_id, model: "unknown" },
      ]);
      if (state.pause?.match === "counts") {
        const pause = state.pause;
        state.pause = null;
        pause.entered();
        await pause.resume;
      }
      return rows;
    }
    if (text.includes("s.user_id::text AS user_id")) return usernames().map(user_id => ({ user_id, client: "codex", model: "gpt-5.5", current: true, tokens: 100, cost: 1, input: 100, output: 0, cache_read: 0, cache_write: 0, reasoning: 0 }));
    if (text.includes("u.id::text AS user_id")) {
      const rows = usernames().map(username => ({ user_id: username, username, display_name: null, avatar_url: null, tokens: 100, cost: 1, active_days: 1, last_submitted_at: "2026-10-19T00:00:00.000Z" }));
      if (state.pause?.match === "humans") {
        const pause = state.pause;
        state.pause = null;
        pause.entered();
        await pause.resume;
      }
      return rows;
    }
    if (text.includes("WITH cells AS MATERIALIZED")) return [{
      trend: [{ date: "2026-10-19", activeUsers: usernames().length, tokens: usernames().length * 100 }],
      cells: usernames().map(username => ({ username, displayName: null, avatarUrl: null, client: "codex", tokens: 100, cost: 1, input: 100, output: 0, cacheRead: 0, cacheWrite: 0, reasoning: 0 })),
      user_days: usernames().map(username => ({ username, days: 1 })),
    }];
    throw new Error(`Unexpected SQL: ${text}`);
  };
  const remove = () => ({ where: () => ({ returning: async () => { state.deleted = true; return [{ id: "target" }]; } }) });
  const tx = {
    execute,
    update: () => ({ set: ({ leaderboardHidden }: { leaderboardHidden: boolean }) => ({ where: () => ({ returning: async () => {
      state.hidden = leaderboardHidden;
      return [{ id: "target", username: "target" }];
    } }) }) }),
    insert: () => ({ values: async () => {} }),
    delete: remove,
  };
  return { ...schema, db: { execute, delete: remove, transaction: async (cb: (transaction: typeof tx) => Promise<unknown>) => cb(tx) } };
});

// Exercise the installed Next cache, not a mock that simply calls callbacks.
const require = createRequire(import.meta.url);
const { IncrementalCache } = require("next/dist/server/lib/incremental-cache/index.js");
const { workAsyncStorage } = require("next/dist/server/app-render/work-async-storage.external.js");
const { workUnitAsyncStorage } = require("next/dist/server/app-render/work-unit-async-storage.external.js");
const cache = new IncrementalCache({
  fs, dev: false, flushToDisk: false, minimalMode: false,
  serverDistDir: "/tmp/tokscale-analytics-test/server", requestHeaders: {}, maxMemoryCacheSize: 50 * 1024 * 1024,
  getPrerenderManifest: () => ({ version: 4, routes: {}, dynamicRoutes: {}, notFoundRoutes: [], preview: { previewModeId: "test" } }),
});
interface Store {
  page: string;
  route: string;
  incrementalCache: typeof cache;
  isStaticGeneration: boolean;
  pendingRevalidates?: Record<string, Promise<unknown>>;
  pendingRevalidatedTags?: Array<{ tag: string; profile?: string | { expire?: number } }>;
  cacheLifeProfiles: Record<string, { stale: number; revalidate: number; expire: number }>;
}
const dataQueries = () => state.queries.filter(q => !q.includes("explore_cache_revision"));
const background: Promise<unknown>[] = [];
async function render<T>(fn: () => Promise<T>, action = false, drain = true): Promise<T> {
  const store: Store = { page: action ? "/api/settings/route" : "/models/[key]/page", route: "/models/[key]",
    incrementalCache: cache, isStaticGeneration: false, cacheLifeProfiles: { max: { stale: 300, revalidate: 900, expire: 31536000 } } };
  const unit = { type: "request", phase: action ? "action" : "render", url: { pathname: "/models/gpt-5.5", search: "" },
    implicitTags: { tags: ["_N_T_/layout", "_N_T_/models/layout", "_N_T_/models/[key]/page", "_N_T_/models/gpt-5.5"], expirationsByCacheKind: new Map() } };
  return workAsyncStorage.run(store, () => workUnitAsyncStorage.run(unit, async () => {
    const result = await fn();
    // Leaf writes can append entries while a parent is revalidating.
    if (drain) {
      let pendingCount = -1;
      while (pendingCount !== Object.keys(store.pendingRevalidates ?? {}).length) {
        pendingCount = Object.keys(store.pendingRevalidates ?? {}).length;
        await Promise.all(Object.values(store.pendingRevalidates ?? {}));
      }
    } else {
      background.push(...Object.values(store.pendingRevalidates ?? {}));
    }
    for (const item of store.pendingRevalidatedTags ?? []) {
      await cache.revalidateTag(item.tag, typeof item.profile === "string" ? store.cacheLifeProfiles[item.profile] : item.profile);
    }
    return result;
  }));
}

const elapsedNow = performance.now.bind(performance);
let cacheTime = Date.now();
afterEach(() => vi.restoreAllMocks());
beforeEach(async () => {
  // Millisecond-granularity expiration must be after prior writes. Advance
  // the cache clock deterministically, without sleeping or changing TTLs.
  vi.spyOn(Date, "now").mockImplementation(() => ++cacheTime);
  vi.spyOn(performance, "now").mockImplementation(() => cacheTime - performance.timeOrigin);
  state.hidden = false;
  state.deleted = false;
  state.revision = 0;
  state.revisionAvailable = true;
  state.revisionError = false;
  state.large = false;
  state.pause = null;
  state.queries = [];
  background.length = 0;
  await cache.revalidateTag("explore", { expire: 0 });
});

function pauseQuery(match: string) {
  let entered!: () => void;
  let resume!: () => void;
  const started = new Promise<void>(resolve => { entered = resolve; });
  const released = new Promise<void>(resolve => { resume = resolve; });
  state.pause = { match, entered, resume: released };
  return { started, resume };
}

describe("actual Next analytics cache", () => {
  it.each(["row", "table"])("fails closed instead of serving warm analytics when the revision %s is missing", async (missing) => {
    const { getPeriodRollup, getModelDetailCached, getModelUsageCounts } = await import("@/lib/explore/db");
    await render(() => getPeriodRollup("30d"));
    state.revisionAvailable = missing !== "row";
    state.revisionError = missing === "table";
    state.queries = [];
    await expect(render(() => getPeriodRollup("30d"))).rejects.toThrow();
    await expect(render(() => getModelDetailCached("gpt-5.5", "30d"))).rejects.toThrow();
    await expect(render(getModelUsageCounts)).rejects.toThrow();
    expect(dataQueries()).toEqual([]);
  });

  it("cannot resurrect old keys when their cache writes complete after a privacy mutation", async () => {
    const { getModelUsageCounts } = await import("@/lib/explore/db");
    const { applyModerationAction } = await import("@/lib/moderation/actions");
    let publish!: () => void;
    let entered!: () => void;
    const held = new Promise<void>(resolve => { publish = resolve; });
    const started = new Promise<void>(resolve => { entered = resolve; });
    const actualSet = cache.set.bind(cache);
    vi.spyOn(cache, "set").mockImplementation(async (...args: unknown[]) => {
      if (state.revision === 0) { entered(); await held; }
      return actualSet(...args);
    });
    const before = render(getModelUsageCounts);
    await started;
    await render(() => applyModerationAction({ target: { id: "target", username: "target", leaderboardHidden: false }, actorUserId: "admin", actorUsername: "admin", action: "hide", reason: "test" }), true);
    expect((await render(getModelUsageCounts)).counts["gpt-5.5"]).toBe(1);
    publish();
    expect((await before).counts["gpt-5.5"]).toBe(2);
    expect((await render(getModelUsageCounts)).counts["gpt-5.5"]).toBe(1);
  });

  it.each(["counts", "humans"])("fences late pre-hide %s writes out of the new privacy generation", async (kind) => {
    const { getModelUsageCounts, getPeriodRollup } = await import("@/lib/explore/db");
    const { applyModerationAction } = await import("@/lib/moderation/actions");
    const load = async () => kind === "counts"
      ? (await getModelUsageCounts()).counts["gpt-5.5"]
      : (await getPeriodRollup("30d")).humans.length;
    const barrier = pauseQuery(kind);
    const before = render(load);
    await barrier.started;
    await render(() => applyModerationAction({ target: { id: "target", username: "target", leaderboardHidden: false }, actorUserId: "admin", actorUsername: "admin", action: "hide", reason: "test" }), true);
    // The new read must finish independently, before the old SQL is released.
    expect(await render(load)).toBe(1);
    barrier.resume();
    expect(await before).toBe(2);
    expect(await render(load)).toBe(1);
  });

  it("does not reuse a detail built with stale dependencies after their SWR refresh", async () => {
    const { getModelDetailCached, getPeriodRollup } = await import("@/lib/explore/db");
    state.hidden = true;
    const first = await render(() => getModelDetailCached("gpt-5.5", "30d"));
    expect(first?.summary.activeUsers).toBe(1);
    state.hidden = false; // Represents normal new data, without privacy invalidation.
    cacheTime += 601000;
    await render(() => getModelDetailCached("gpt-5.5", "30d"));
    const detail = await render(() => getModelDetailCached("gpt-5.5", "30d"));
    const rollup = await render(() => getPeriodRollup("30d"));
    expect(detail?.summary.activeUsers).toBe(2);
    expect(detail?.coverage.accounts).toBe(2);
    expect(detail?.humans).toHaveLength(2);
    expect(detail?.coverage).toEqual(rollup.coverage);
  });

  it("caches a production-sized all-time result in bounded parts without truncating APIs", async () => {
    state.large = true;
    const { getPeriodRollup } = await import("@/lib/explore/db");
    const { getModelLeaderboard, listModelKeys } = await import("@/lib/explore");
    const writes: number[] = [];
    const actualSet = cache.set.bind(cache);
    vi.spyOn(cache, "set").mockImplementation(async (...args: unknown[]) => {
      writes.push(Buffer.byteLength(JSON.stringify(args[1])));
      return actualSet(...args);
    });
    const warn = vi.spyOn(console, "warn");
    const coldStart = elapsedNow();
    const cold = await render(() => getPeriodRollup("all"));
    const coldMs = elapsedNow() - coldStart;
    const bytes = Buffer.byteLength(JSON.stringify(cold));
    expect(bytes).toBeGreaterThan(3456422);
    expect(cold.models).toHaveLength(12000);
    expect(cold.humans).toHaveLength(1000);
    expect(writes.length).toBeGreaterThan(10);
    expect(Math.max(...writes)).toBeLessThan(350000);
    expect(warn.mock.calls.flat().join(" ")).not.toContain("items over 2MB");
    expect(dataQueries()).toHaveLength(4); // catalog, bounds, facts, user totals
    state.queries = [];
    const warmStart = elapsedNow();
    expect(await render(() => getPeriodRollup("all"))).toEqual(cold);
    const warmMs = elapsedNow() - warmStart;
    vi.stubEnv("DATABASE_URL", "postgresql://tokscale:tokscale@127.0.0.1:5480/tokscale");
    try {
      const page = await render(() => getModelLeaderboard({ period: "all", offset: 11900, limit: 100 }));
      expect(page.rows).toHaveLength(100);
      expect(page.rows[0].rank).toBe(11901);
      expect(await render(listModelKeys)).toHaveLength(12000);
    } finally { vi.unstubAllEnvs(); }
    expect(dataQueries()).toEqual([]);
    expect(state.queries.filter(q => q.includes("SELECT revision::text"))).toHaveLength(3); // one per warm loader
    console.log("synthetic all-time cache", { bytes, maxEntryBytes: Math.max(...writes), entries: writes.length, coldMs, warmMs, coldAggregationQueries: 4, warmAggregationQueries: 0, revisionReadsPerLoader: 1 });
  }, 20000);

  it("refreshes ordinary submissions by manifest TTL without revalidating immutable leaves", async () => {
    const { partitionedCache } = await import("@/lib/explore/partitionedCache");
    let revision = 1;
    const load = vi.fn(async () => ({ revision, text: "data".repeat(100000) }));
    const key = ["explore", "swr-test"];
    const error = vi.spyOn(console, "error");
    expect((await render(() => partitionedCache(load, key))).revision).toBe(1);
    revision = 2;
    cacheTime += 601000;
    // Ordinary changes may return one stale result while its manifest refreshes.
    expect((await render(() => partitionedCache(load, key))).revision).toBe(1);
    expect((await render(() => partitionedCache(load, key))).revision).toBe(2);
    expect(load).toHaveBeenCalledTimes(2);
    expect(error).not.toHaveBeenCalled();
  });

  it("repairs an independently evicted part without mixing generations", async () => {
    const { partitionedCache } = await import("@/lib/explore/partitionedCache");
    let source = { text: "한글".repeat(500000), revision: 1 };
    const load = vi.fn(async () => source);
    const key = ["explore", "eviction-test"];
    await render(() => partitionedCache(load, key));
    expect(load).toHaveBeenCalledTimes(1);
    source = { text: "new-data".repeat(500000), revision: 2 };
    const actualGet = cache.get.bind(cache);
    let dropped = false;
    const get = vi.spyOn(cache, "get").mockImplementation(async (...args: unknown[]) => {
      const entry = await actualGet(...args);
      // A leaf's stored JSON body is a base64 string. Simulate one eviction;
      // the production cache itself still performs every other operation.
      if (!dropped && entry?.value?.data?.body?.startsWith('"')) { dropped = true; return null; }
      return entry;
    });
    expect(await render(() => partitionedCache(load, key))).toEqual(source);
    expect(dropped).toBe(true);
    expect(load).toHaveBeenCalledTimes(2);
    get.mockRestore();
  });

  it("shares a pending SWR refresh across requests that already returned stale data", async () => {
    const { getModelUsageCounts } = await import("@/lib/explore/db");
    await render(getModelUsageCounts);
    cacheTime += 601000;
    const barrier = pauseQuery("counts");
    expect((await render(getModelUsageCounts, false, false)).counts["gpt-5.5"]).toBe(2);
    await barrier.started;
    await render(getModelUsageCounts, false, false);
    await render(getModelUsageCounts, false, false);
    expect(dataQueries().filter(q => q.includes("SELECT DISTINCT"))).toHaveLength(2);
    barrier.resume();
    await Promise.all(background);
    expect((await render(getModelUsageCounts)).counts["gpt-5.5"]).toBe(2);
  });

  it("keeps a durable recovery pointer after both original and repaired leaves are evicted", async () => {
    const { partitionedCache } = await import("@/lib/explore/partitionedCache");
    let revision = 1;
    const load = vi.fn(async () => ({ revision, text: `revision-${revision}`.repeat(50000) }));
    const writtenParts = new Set<string>();
    const evicted = new Set<string>();
    const actualSet = cache.set.bind(cache);
    const actualGet = cache.get.bind(cache);
    vi.spyOn(cache, "set").mockImplementation(async (...args: unknown[]) => {
      const value = args[1] as { data?: { body?: string } };
      if (value?.data?.body?.startsWith('"')) writtenParts.add(String(args[0]));
      return actualSet(...args);
    });
    vi.spyOn(cache, "get").mockImplementation(async (...args: unknown[]) =>
      evicted.has(String(args[0])) ? null : actualGet(...args));
    const read = () => render(() => partitionedCache(load, ["explore", "repeated-eviction-test"]));
    expect((await read()).revision).toBe(1);
    for (const key of writtenParts) evicted.add(key);
    revision = 2;
    expect((await read()).revision).toBe(2);
    for (const key of writtenParts) evicted.add(key);
    revision = 3;
    expect((await read()).revision).toBe(3);
    expect((await read()).revision).toBe(3);
    expect((await read()).revision).toBe(3);
    expect(load).toHaveBeenCalledTimes(3);
  });

  it("reuses warm shared dependencies and rejects negative keys before period work", async () => {
    const { getModelCatalog, getPeriodRollup, getModelDetailCached } = await import("@/lib/explore/db");
    await render(() => getModelCatalog());
    await render(() => getPeriodRollup("7d"));
    state.queries = [];
    expect(await render(() => getModelDetailCached("gpt-5.5", "7d"))).not.toBeNull();
    expect(dataQueries()).toHaveLength(1);
    expect(dataQueries()[0]).toContain("WITH cells AS MATERIALIZED");
    state.queries = [];
    for (const key of ["missing-a", "missing-b"]) {
      expect(await render(() => getModelDetailCached(key, "30d"))).toBeNull();
      expect(await render(() => getModelDetailCached(key, "all"))).toBeNull();
    }
    expect(dataQueries()).toEqual([]);
  });

  it("shares period scans with pricing mixes and concurrent cold rollup loads", async () => {
    const { getPeriodRollup, getPeriodModelMixes } = await import("@/lib/explore/db");
    await render(() => Promise.all([getPeriodRollup("30d"), getPeriodModelMixes("30d")]));
    expect(dataQueries().filter(q => q.includes("AS first_seen"))).toHaveLength(1);
    expect(dataQueries().filter(q => q.includes("s.user_id::text AS user_id"))).toHaveLength(1);
    state.queries = [];
    const mixes = await render(() => getPeriodModelMixes("30d"));
    expect(mixes.community.input).toBe(200);
    expect(dataQueries()).toEqual([]);
  });

  it("loads benchmark counts without all-time catalogs or detailed period scans", async () => {
    const { getModelUsageCounts } = await import("@/lib/explore/db");
    const first = await render(() => getModelUsageCounts());
    expect(first.counts).toEqual({ "gpt-5.5": 2 });
    expect(dataQueries()).toHaveLength(2);
    expect(dataQueries()[1]).toContain("SELECT DISTINCT");
    expect(dataQueries().join(" ")).not.toContain("AS first_seen");
    await render(() => getModelUsageCounts());
    expect(dataQueries()).toHaveLength(2);
  });

  it.each(["hide", "show", "account", "submissions"])("immediately removes stale analytics after %s", async (action) => {
    const { getModelDetailCached, getPeriodModelMixes, getModelUsageCounts } = await import("@/lib/explore/db");
    const { applyModerationAction } = await import("@/lib/moderation/actions");
    state.hidden = action === "show";
    const load = () => Promise.all([getModelDetailCached("gpt-5.5", "30d"), getPeriodModelMixes("30d"), getModelUsageCounts()]);
    const first = await render(load);
    expect(first[0]?.humans.some(h => h.username === "target")).toBe(action !== "show");
    const initial = dataQueries().length;
    await render(load);
    expect(dataQueries()).toHaveLength(initial);
    await render(async () => {
      if (action === "hide" || action === "show") {
        await applyModerationAction({ target: { id: "target", username: "target", leaderboardHidden: state.hidden }, actorUserId: "admin", actorUsername: "admin", action: action === "show" ? "unhide" : "hide", reason: "test" });
      } else {
        const { DELETE } = action === "account" ? await import("@/app/api/settings/account/route") : await import("@/app/api/settings/submitted-data/route");
        expect((await DELETE(new Request("http://localhost/api/settings", { method: "DELETE" }))).status).toBe(200);
      }
    }, true);
    const second = await render(load);
    const expected = action === "show" ? 2 : 1;
    expect(second[0]?.humans.some(h => h.username === "target")).toBe(action === "show");
    expect(second[0]?.coverage.accounts).toBe(expected);
    expect(second[1].community.input).toBe(expected * 100);
    expect(second[2].counts["gpt-5.5"]).toBe(expected);
    expect(dataQueries().length).toBeGreaterThan(initial);
  });
});
