import { beforeAll, beforeEach, describe, expect, it, vi } from "vitest";

const state = vi.hoisted(() => {
  const results: Array<unknown> = [];
  const queries: Array<{ strings: string[]; values: unknown[] }> = [];
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
    queries,
    sql,
    select,
    reset: () => {
      results.length = 0;
      queries.length = 0;
      select.mockClear();
    },
  };
});
vi.mock("next/cache", () => ({ unstable_cache: (fn: () => unknown) => fn }));
vi.mock("@/lib/db", () => ({
  db: {
    execute: vi.fn(() => Promise.resolve(state.results.shift() ?? [])),
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
    expect(query()).toContain("unnest(s.sources_used)");
    expect(query()).toContain("unnest(s.models_used)");
  });

  it("keeps global headline totals unfiltered by directives and includes hidden users", async () => {
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
    expect(query()).toContain("FROM (\n    SELECT s.user_id");
    expect(query()).toContain("AS stats");
    expect(query()).toContain("WHERE leaderboard_hidden = false");
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
    expect(query()).toContain("COUNT(*) FROM (");
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
