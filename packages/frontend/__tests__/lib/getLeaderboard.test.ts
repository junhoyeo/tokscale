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
  return {
    results,
    queries,
    sql,
    reset: () => {
      results.length = 0;
      queries.length = 0;
    },
  };
});

vi.mock("next/cache", () => ({ unstable_cache: (fn: () => unknown) => fn }));
vi.mock("@/lib/db", () => ({
  db: {
    execute: vi.fn(() => Promise.resolve(state.results.shift() ?? [])),
    select: vi.fn(),
  },
  users: {
    id: "users.id",
    username: "users.username",
    displayName: "users.displayName",
    avatarUrl: "users.avatarUrl",
    leaderboardHidden: "users.leaderboardHidden",
  },
}));
vi.mock("@/lib/db/usernameLookup", () => ({
  USERNAME_LOOKUP_LIMIT: 2,
  getSingleUsernameMatch: (rows: unknown[]) => rows[0] ?? null,
  normalizeUsernameCacheKey: (name: string) => name.toLowerCase(),
  usernameEqualsIgnoreCase: (name: string) =>
    state.sql`LOWER(users.username) = LOWER(${name})`,
}));
vi.mock("drizzle-orm", () => ({ sql: state.sql }));

let getLeaderboardData: (typeof import("../../src/lib/leaderboard/getLeaderboard"))["getLeaderboardData"];
let getUserRank: (typeof import("../../src/lib/leaderboard/getLeaderboard"))["getUserRank"];

function text(value: unknown): string {
  if (!value || typeof value !== "object") return String(value ?? "");
  const query = value as { strings?: string[]; values?: unknown[] };
  if (!query.strings) return "";
  return query.strings.reduce(
    (out, part, index) =>
      `${out}${part}${index < query.values!.length ? text(query.values![index]) : ""}`,
    "",
  );
}
function allSql() {
  return state.queries.map(text).join("\n");
}
function finalSql() {
  return text(state.queries.at(-1));
}
function row(
  users: unknown,
  stats = { totalUsers: 0, totalTokens: 0, totalCost: 0, uniqueUsers: 0 },
) {
  return [{ users, ...stats }];
}

beforeAll(
  async () =>
    ({ getLeaderboardData, getUserRank } =
      await import("../../src/lib/leaderboard/getLeaderboard")),
);
beforeEach(() => state.reset());

describe("period leaderboard aggregate query", () => {
  it("uses deterministic ROW_NUMBER ordering before pagination", async () => {
    state.results.push(
      row(
        [
          {
            rank: 2,
            userId: "bravo",
            username: "bravo",
            displayName: null,
            avatarUrl: null,
            totalTokens: 10,
            totalCost: 2,
          },
        ],
        { totalUsers: 3, totalTokens: 30, totalCost: 3, uniqueUsers: 3 },
      ),
    );
    const data = await getLeaderboardData("week", 2, 1, "tokens");
    const query = allSql();
    expect(query).toContain(
      "ROW_NUMBER() OVER (ORDER BY total_tokens DESC, total_cost DESC, LOWER(username) ASC, user_id ASC)",
    );
    expect(query).toContain("LIMIT 1 OFFSET 1");
    expect(data.users).toMatchObject([{ username: "bravo", rank: 2 }]);
    expect(finalSql().match(/FROM daily_breakdown/g)).toHaveLength(1);
  });

  it("keeps hidden users in totals while excluding them before rank and page", async () => {
    state.results.push(
      row([], {
        totalUsers: 1,
        totalTokens: 300,
        totalCost: 30,
        uniqueUsers: 2,
      }),
    );
    const data = await getLeaderboardData("week", 1, 50);
    expect(data.stats).toEqual({
      totalTokens: 300,
      totalCost: 30,
      uniqueUsers: 2,
    });
    expect(allSql()).toContain("WHERE leaderboard_hidden = false");
  });

  it("scopes client and model directives in the same JSON client entry", async () => {
    state.results.push(row([]));
    await getLeaderboardData(
      "week",
      1,
      50,
      "tokens",
      "client:codex model:gpt-5",
    );
    const query = allSql();
    expect(query).toContain("jsonb_each");
    expect(query).toContain("client.key");
    expect(query).toContain("model.key");
    expect(query).toContain("model.value->>'tokens'");
  });

  it("uses literal matching for percent, underscore, and the escape character", async () => {
    state.results.push(row([]));
    await getLeaderboardData("week", 1, 50, "tokens", "a%_!b");
    const query = allSql();
    expect(query).toContain("ESCAPE '!'");
    expect(query).toContain("%a!%!_!!b%");
  });

  it("returns a period user rank from the fully ranked result", async () => {
    state.results.push(
      row([
        {
          rank: 2,
          userId: "a",
          username: "alice",
          displayName: null,
          avatarUrl: null,
          totalTokens: 10,
          totalCost: 1,
        },
      ]),
    );
    await expect(getUserRank("alice", "week")).resolves.toMatchObject({
      username: "alice",
      rank: 2,
    });
    expect(allSql()).toContain("LOWER(username) = LOWER(alice)");
  });

  it("rejects corrupt JSON instead of silently returning an empty page", async () => {
    state.results.push(row("{"));
    await expect(getLeaderboardData("week")).rejects.toThrow(
      "malformed users JSON",
    );
  });
});
