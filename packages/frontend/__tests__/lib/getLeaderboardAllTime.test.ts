import { beforeAll, beforeEach, describe, expect, it, vi } from "vitest";

import { cteBody } from "../support/sqlCte";

const state = vi.hoisted(() => {
  function renderSql(value: unknown): string {
    if (!value || typeof value !== "object") return String(value ?? "");
    const q = value as { strings?: string[]; values?: unknown[] };
    return q.strings
      ? q.strings.reduce(
          (s, p, i) =>
            `${s}${p}${i < q.values!.length ? renderSql(q.values![i]) : ""}`,
          "",
        )
      : "";
  }

  const results: Array<unknown> = [];
  const scopedResults: Array<unknown> = [];
  const queries: Array<{ strings: string[]; values: unknown[] }> = [];
  const cacheCalls: Array<{ keyParts: string[]; options: unknown }> = [];
  const cacheStore = new Map<string, unknown>();
  const sql = Object.assign(
    vi.fn((strings: TemplateStringsArray, ...values: unknown[]) => {
      const query = { strings: Array.from(strings), values, as: () => ({}) };
      queries.push(query);
      return query;
    }),
    {
      join: (items: unknown[], separator: unknown) => ({
        strings: [],
        values: [items, separator],
      }),
    },
  );
  const execute = vi.fn((statement: unknown) => {
    const rendered = renderSql(statement);
    if (rendered.includes("FROM daily_breakdown d") && !rendered.includes("WITH aggregated AS")) {
      return Promise.resolve(scopedResults.shift() ?? []);
    }
    return Promise.resolve(results.shift() ?? []);
  });
  const unstableCache = vi.fn((fn: () => unknown, keyParts: string[], options: unknown) => {
    cacheCalls.push({ keyParts, options });
    return async () => {
      const key = JSON.stringify(keyParts);
      if (!cacheStore.has(key)) {
        cacheStore.set(key, await fn());
      }
      return cacheStore.get(key);
    };
  });
  const select = vi.fn(() => {
    const builder = {
      from: () => builder,
      where: () => builder,
      limit: () =>
        Promise.resolve([
          {
            id: "a",
            username: "alice",
            displayName: null,
            avatarUrl: null,
            leaderboardHidden: false,
          },
        ]),
    };
    return builder;
  });
  return {
    results,
    scopedResults,
    queries,
    cacheCalls,
    execute,
    sql,
    select,
    unstableCache,
    reset: () => {
      results.length = 0;
      scopedResults.length = 0;
      queries.length = 0;
      cacheCalls.length = 0;
      cacheStore.clear();
      execute.mockClear();
      select.mockClear();
      unstableCache.mockClear();
    },
  };
});
vi.mock("next/cache", () => ({ unstable_cache: state.unstableCache }));
vi.mock("@/lib/db", () => ({
  db: {
    execute: state.execute,
    select: state.select,
  },
  users: {
    id: "id",
    username: "username",
    displayName: "displayName",
    avatarUrl: "avatarUrl",
    leaderboardHidden: "leaderboardHidden",
  },
}));
vi.mock("@/lib/db/usernameLookup", () => ({
  USERNAME_LOOKUP_LIMIT: 2,
  getSingleUsernameMatch: (rows: unknown[]) => rows[0] ?? null,
  normalizeUsernameCacheKey: (v: string) => v.toLowerCase(),
  usernameEqualsIgnoreCase: (v: string) =>
    state.sql`LOWER(username) = LOWER(${v})`,
}));
vi.mock("drizzle-orm", () => ({ sql: state.sql }));
let getLeaderboardData: (typeof import("../../src/lib/leaderboard/getLeaderboard"))["getLeaderboardData"];
let getUserRank: (typeof import("../../src/lib/leaderboard/getLeaderboard"))["getUserRank"];
function text(value: unknown): string {
  if (!value || typeof value !== "object") return String(value ?? "");
  const q = value as { strings?: string[]; values?: unknown[] };
  return q.strings
    ? q.strings.reduce(
        (s, p, i) =>
          `${s}${p}${i < q.values!.length ? text(q.values![i]) : ""}`,
        "",
      )
    : "";
}
function query() {
  return state.queries.map(text).join("\n");
}
function finalQuery() {
  return text(state.queries.at(-1));
}
function occurrences(value: string, needle: string) {
  return value.split(needle).length - 1;
}
function executedSql() {
  return state.execute.mock.calls.map(([statement]) => text(statement)).join("\n");
}
function scopedScanCount() {
  return state.execute.mock.calls.filter(([statement]) =>
    text(statement).includes("FROM daily_breakdown d"),
  ).length;
}
function leaderboardRow(totalTokens = 0, totalCost = 0) {
  return [
    {
      users: [],
      totalUsers: 0,
      totalTokens,
      totalCost,
      uniqueUsers: 0,
    },
  ];
}
beforeAll(
  async () =>
    ({ getLeaderboardData, getUserRank } =
      await import("../../src/lib/leaderboard/getLeaderboard")),
);
beforeEach(() => state.reset());

describe("all-time leaderboard aggregate query", () => {
  it("uses competition rank and source/model directives", async () => {
    state.results.push([
      {
        users: [],
        totalUsers: 0,
        totalTokens: 100,
        totalCost: 10,
        uniqueUsers: 2,
      },
    ]);
    await getLeaderboardData(
      "all",
      1,
      50,
      "tokens",
      "client:codex model:gpt-5",
    );
    expect(query()).toContain("RANK() OVER (ORDER BY total_tokens DESC)");
    expect(query()).toContain("jsonb_each(COALESCE(d.source_breakdown");
    expect(query()).toContain("jsonb_each(COALESCE(client.value->'models'");
    expect(query()).toContain("LOWER(client.key) LIKE %codex%");
    expect(query()).toContain("LOWER(model.key) LIKE %gpt-5%");
    expect(finalQuery()).toContain("jsonb_to_recordset");
  });

  it("keeps global headline totals unfiltered by directives and excludes hidden users", async () => {
    state.results.push([
      {
        users: [],
        totalUsers: 1,
        totalTokens: 1000,
        totalCost: 100,
        uniqueUsers: 3,
      },
    ]);
    const data = await getLeaderboardData(
      "all",
      1,
      50,
      "tokens",
      "client:codex",
    );
    expect(data.stats).toEqual({
      totalTokens: 1000,
      totalCost: 100,
      uniqueUsers: 3,
    });
    expect(query()).toContain("stat_rows AS (");
    expect(query()).toContain("stats AS (");
    const stats = cteBody(finalQuery(), "stats", "rankable");
    expect(stats).toContain("FROM stat_rows");
    expect(stats).toContain("WHERE leaderboard_hidden = false");
    expect(cteBody(finalQuery(), "rankable", "filtered")).toContain(
      "WHERE leaderboard_hidden = false",
    );
    expect(occurrences(executedSql(), "jsonb_each(COALESCE(d.source_breakdown")).toBe(1);
    expect(finalQuery()).toContain("jsonb_to_recordset");
    expect(occurrences(finalQuery(), "stats AS (")).toBe(1);
    expect(occurrences(finalQuery(), "FROM stat_rows")).toBe(1);
  });

  it("caches scoped all-time aggregates by canonical directives, not text or pagination", async () => {
    state.scopedResults.push(
      [{ userId: "user-a", totalTokens: "100", totalCost: "1" }],
      [{ userId: "user-b", totalTokens: "200", totalCost: "2" }],
      [{ userId: "user-c", totalTokens: "300", totalCost: "3" }],
      [{ userId: "user-d", totalTokens: "400", totalCost: "4" }],
    );
    state.results.push(
      leaderboardRow(100, 1),
      leaderboardRow(100, 1),
      leaderboardRow(100, 1),
      leaderboardRow(100, 1),
      leaderboardRow(100, 1),
      leaderboardRow(100, 1),
      leaderboardRow(200, 2),
      leaderboardRow(300, 3),
      leaderboardRow(400, 4),
    );

    await getLeaderboardData("all", 1, 50, "tokens", "client:codex model:gpt-5");
    await getLeaderboardData("all", 1, 50, "tokens", "client:codex model:gpt-5");
    await getLeaderboardData("all", 1, 50, "tokens", "model:GPT-5 client:CODEX");
    await getLeaderboardData("all", 1, 50, "tokens", "client:codex model:gpt-5 alice");
    await getLeaderboardData("all", 2, 50, "tokens", "client:codex model:gpt-5");
    await getLeaderboardData("all", 1, 50, "cost", "client:codex model:gpt-5");
    await getLeaderboardData("all", 1, 50, "tokens", "client:codex client:codex model:gpt-5");

    expect(scopedScanCount()).toBe(1);

    await getLeaderboardData("all", 1, 50, "tokens", "client:claude model:gpt-5");

    expect(scopedScanCount()).toBe(2);

    await getLeaderboardData("all", 1, 50, "tokens", "client:a:b model:c");
    await getLeaderboardData("all", 1, 50, "tokens", "client:a model:b:c");

    expect(scopedScanCount()).toBe(4);
    const scopedCacheCalls = state.cacheCalls.filter(({ keyParts }) =>
      keyParts[0]?.startsWith("leaderboard:all:scoped:"),
    );
    expect(new Set(scopedCacheCalls.map(({ keyParts }) => keyParts[0]))).toEqual(
      new Set([
        'leaderboard:all:scoped:[["codex"],["gpt-5"]]',
        'leaderboard:all:scoped:[["claude"],["gpt-5"]]',
        'leaderboard:all:scoped:[["a:b"],["c"]]',
        'leaderboard:all:scoped:[["a"],["b:c"]]',
      ]),
    );
    expect(scopedCacheCalls[0]?.options).toEqual({
      tags: ["leaderboard", "leaderboard:all"],
      revalidate: 300,
    });
  });

  it("caches an oversized scoped aggregate sentinel and preserves results through the direct path", async () => {
    const oversizedRows = Array.from({ length: 15_000 }, (_, index) => ({
      userId: `00000000-0000-0000-0000-${index.toString().padStart(12, "0")}`,
      totalTokens: "10000000000000000000",
      totalCost: "1000000000000.0000",
    }));
    state.scopedResults.push(oversizedRows);
    state.results.push(leaderboardRow(300, 3), leaderboardRow(300, 3));

    const first = await getLeaderboardData("all", 1, 50, "tokens", "client:codex");
    const second = await getLeaderboardData("all", 1, 50, "tokens", "client:codex alice");

    expect(first.stats.totalTokens).toBe(300);
    expect(second.stats.totalCost).toBe(3);
    expect(scopedScanCount()).toBe(3);
    expect(finalQuery()).toContain("FROM daily_breakdown d");
    expect(finalQuery()).not.toContain("jsonb_to_recordset");
  });

  it("aggregates duplicate submission rows into one ranked user before counting", async () => {
    state.results.push([
      {
        users: [
          {
            rank: 1,
            userId: "alice",
            username: "alice",
            displayName: null,
            avatarUrl: null,
            totalTokens: 300,
            totalCost: 3,
          },
        ],
        totalUsers: 1,
        totalTokens: 300,
        totalCost: 3,
        uniqueUsers: 1,
      },
    ]);
    const data = await getLeaderboardData("all");
    expect(data).toMatchObject({
      users: [{ username: "alice", rank: 1, totalTokens: 300 }],
      pagination: { totalUsers: 1 },
      stats: { uniqueUsers: 1 },
    });
    expect(query()).toContain("SUM(s.total_tokens) AS total_tokens");
    expect(query()).toContain("GROUP BY s.user_id");
    expect(query()).toContain("COUNT(*)::int AS unique_users");
    const final = finalQuery();
    expect(final.indexOf("GROUP BY s.user_id")).toBeLessThan(
      final.indexOf("RANK() OVER (ORDER BY total_tokens DESC)"),
    );
  });

  it("keeps primary-metric ties at the same rank and orders their display deterministically", async () => {
    state.results.push([
      {
        users: [
          {
            rank: 1,
            userId: "alice",
            username: "alice",
            displayName: null,
            avatarUrl: null,
            totalTokens: 300,
            totalCost: 3,
          },
          {
            rank: 1,
            userId: "bob",
            username: "bob",
            displayName: null,
            avatarUrl: null,
            totalTokens: 300,
            totalCost: 2,
          },
        ],
        totalUsers: 2,
        totalTokens: 600,
        totalCost: 5,
        uniqueUsers: 2,
      },
    ]);
    const data = await getLeaderboardData("all");
    expect(data.users.map((user) => [user.username, user.rank])).toEqual([
      ["alice", 1],
      ["bob", 1],
    ]);
    const final = finalQuery();
    expect(final).toContain("RANK() OVER (ORDER BY total_tokens DESC)");
    expect(final).toContain(
      "ORDER BY rank ASC, total_cost DESC, LOWER(username) ASC, user_id ASC",
    );
  });

  it("returns one all-time user rank after aggregating that user's submissions", async () => {
    state.results.push([
      {
        users: [
          {
            rank: 1,
            userId: "alice",
            username: "alice",
            displayName: null,
            avatarUrl: null,
            totalTokens: 300,
            totalCost: 3,
          },
        ],
        totalUsers: 1,
        totalTokens: 850,
        totalCost: 10,
        uniqueUsers: 3,
      },
    ]);
    await expect(getUserRank("alice")).resolves.toMatchObject({
      username: "alice",
      rank: 1,
      totalTokens: 300,
    });
    expect(query()).toContain("SUM(s.total_tokens) AS total_tokens");
  });

  it("does not interpret literal percent or underscore directives as wildcards", async () => {
    state.results.push([
      {
        users: [],
        totalUsers: 0,
        totalTokens: 0,
        totalCost: 0,
        uniqueUsers: 0,
      },
    ]);
    await getLeaderboardData("all", 1, 50, "tokens", "a%_!");
    expect(query()).toContain("%a!%!_!!%");
    expect(query()).toContain("ESCAPE '!'");
  });
});
