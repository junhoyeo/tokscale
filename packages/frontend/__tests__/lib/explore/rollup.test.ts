import { describe, expect, it } from "vitest";
import {
  buildModelCatalog,
  buildModelDetail,
  buildPeriodRollup,
  indexableModelKeys,
  pageRows,
  rankRows,
  tokenMixShares,
  type UsageFact,
  type UserTotals,
} from "@/lib/explore/rollup";
import { resolveWindow } from "@/lib/explore/window";

const window7d = resolveWindow("7d", { firstDate: "2026-01-01", latestDate: "2026-10-19" }, "2026-10-19");

function fact(userId: string, client: string, model: string, tokens: number, current = true): UsageFact {
  return {
    userId,
    client,
    model,
    current,
    tokens,
    cost: tokens / 1e6,
    input: tokens * 0.1,
    output: tokens * 0.05,
    cacheRead: tokens * 0.8,
    cacheWrite: tokens * 0.05,
    reasoning: 0,
  };
}

function user(userId: string, tokens: number, lastSubmittedAt: string | null = "2026-10-19T08:00:00.000Z"): UserTotals {
  return { userId, username: `user-${userId}`, displayName: null, avatarUrl: null, tokens, cost: tokens / 1e6, activeDays: 3, lastSubmittedAt };
}

const catalog = buildModelCatalog([
  { client: "claude", model: "claude-opus-4-5-20251101", firstSeen: "2026-01-02", tokens: 10 },
  { client: "claude", model: "claude-opus-4.5", firstSeen: "2026-03-01", tokens: 30 },
  { client: "codex", model: "gpt-5.6-sol", firstSeen: "2026-10-14", tokens: 50 },
  { client: "kilocode", model: "openai/gpt-5.6-sol", firstSeen: "2026-10-15", tokens: 5 },
  { client: "claude", model: "unknown", firstSeen: "2026-01-01", tokens: 99 },
]);

describe("buildModelCatalog", () => {
  it("folds raw ids into canonical keys with the earliest first-seen date", () => {
    expect(catalog.modelFirstSeen["claude-opus-4-5"]).toBe("2026-01-02");
    expect(catalog.aliases["claude-opus-4-5"]).toEqual(["claude-opus-4.5", "claude-opus-4-5-20251101"]);
    expect(catalog.aliases["gpt-5.6-sol"]).toEqual(["gpt-5.6-sol", "openai/gpt-5.6-sol"]);
    expect(catalog.agentFirstSeen.kilo).toBe("2026-10-15");
  });
});

describe("buildPeriodRollup", () => {
  const facts: UsageFact[] = [
    fact("a", "claude", "claude-opus-4-5-20251101", 600),
    fact("a", "codex", "gpt-5.6-sol", 300),
    fact("a", "claude", "unknown", 100),
    fact("b", "claude", "claude-opus-4.5", 200),
    fact("c", "kilocode", "openai/gpt-5.6-sol", 50),
    // previous window only
    fact("d", "claude", "claude-opus-4-5", 999, false),
    fact("a", "claude", "claude-opus-4-5", 1, false),
  ];
  const users = [user("a", 1000), user("b", 200, "2026-10-19T09:30:00.000Z"), user("c", 50), user("z", 0, "2026-10-20T00:00:00.000Z")];
  const rollup = buildPeriodRollup({ window: window7d, facts, users, catalog });

  it("fills coverage from visible active accounts only", () => {
    expect(rollup.coverage).toEqual({
      period: "7d",
      start: "2026-10-13",
      end: "2026-10-19",
      accounts: 3,
      modelIdentifiedShare: 1150 / 1250,
      lastDataAt: "2026-10-19T09:30:00.000Z",
    });
  });

  it("counts distinct active users per canonical model and keeps the previous window", () => {
    const opus = rollup.models.find((m) => m.key === "claude-opus-4-5")!;
    expect(opus).toMatchObject({ rank: 1, activeUsers: 2, previousActiveUsers: 2, tokens: 800, name: "Claude Opus 4.5", lab: "Anthropic", isNew: false });
    expect(opus.adoption).toBeCloseTo(2 / 3);
    expect(opus.topAgents).toEqual([expect.objectContaining({ key: "claude", share: 1 })]);

    const sol = rollup.models.find((m) => m.key === "gpt-5.6-sol")!;
    expect(sol).toMatchObject({ activeUsers: 2, previousActiveUsers: 0, tokens: 350, isNew: true });
    expect(sol.topAgents.map((a) => a.key)).toEqual(["codex", "kilo"]);
  });

  it("never gives unidentified models a row", () => {
    expect(rollup.models.map((m) => m.key)).not.toContain("unknown");
  });

  it("ranks agents and keeps unidentified tokens in agent totals", () => {
    const claude = rollup.agents.find((a) => a.key === "claude")!;
    expect(claude).toMatchObject({ activeUsers: 2, tokens: 900, previousActiveUsers: 2, name: "Claude Code" });
    expect(claude.topModels).toEqual([expect.objectContaining({ key: "claude-opus-4-5", share: 800 / 900 })]);
    expect(rollup.agents.find((a) => a.key === "kilo")).toBeDefined();
    expect(rollup.agents.find((a) => a.key === "kilocode")).toBeUndefined();
  });

  it("ranks humans by tokens with their top models and agents", () => {
    expect(rollup.humans.map((h) => [h.rank, h.username])).toEqual([
      [1, "user-a"],
      [2, "user-b"],
      [3, "user-c"],
    ]);
    expect(rollup.humans[0].topModels.map((m) => m.key)).toEqual(["claude-opus-4-5", "gpt-5.6-sol"]);
    expect(rollup.humans[0].topAgents.map((a) => a.key)).toEqual(["claude", "codex"]);
  });

  it("uses null previous counts and no isNew flags for 'all'", () => {
    const all = resolveWindow("all", { firstDate: "2026-01-01", latestDate: "2026-10-19" }, "2026-10-19");
    const r = buildPeriodRollup({ window: all, facts: facts.filter((f) => f.current), users, catalog });
    expect(r.models.every((m) => m.previousActiveUsers === null && !m.isNew)).toBe(true);
  });

  it("returns empty but valid output without data", () => {
    const r = buildPeriodRollup({ window: window7d, facts: [], users: [], catalog });
    expect(r.coverage).toMatchObject({ accounts: 0, modelIdentifiedShare: 0, lastDataAt: null });
    expect(r.models).toEqual([]);
  });
});

describe("ranking helpers", () => {
  const rows = [
    { key: "b", rank: 0, activeUsers: 5, tokens: 10 },
    { key: "a", rank: 0, activeUsers: 5, tokens: 10 },
    { key: "c", rank: 0, activeUsers: 1, tokens: 99 },
  ];

  it("sorts by users with stable tie-breaks and renumbers ranks", () => {
    expect(rankRows(rows, "users").map((r) => [r.rank, r.key])).toEqual([
      [1, "a"],
      [2, "b"],
      [3, "c"],
    ]);
    expect(rankRows(rows, "tokens")[0].key).toBe("c");
  });

  it("pages with limit and offset", () => {
    expect(pageRows([1, 2, 3, 4], 2, 1)).toEqual([2, 3]);
    expect(pageRows([1, 2, 3], undefined, undefined)).toEqual([1, 2, 3]);
  });

  it("filters indexable model keys by all-time users", () => {
    const all = resolveWindow("all", { firstDate: "2026-01-01", latestDate: "2026-10-19" }, "2026-10-19");
    const r = buildPeriodRollup({
      window: all,
      facts: [fact("a", "claude", "claude-opus-5", 1), fact("b", "claude", "claude-opus-5", 1), fact("a", "codex", "gpt-5.5", 1)],
      users: [user("a", 2), user("b", 1)],
      catalog,
    });
    expect(indexableModelKeys(r, 2)).toEqual(["claude-opus-5"]);
  });

  it("turns token buckets into shares", () => {
    expect(tokenMixShares({ input: 1, output: 1, cacheRead: 2, cacheWrite: 0, reasoning: 0 })).toEqual({ input: 0.25, output: 0.25, cacheRead: 0.5, cacheWrite: 0, reasoning: 0 });
    expect(tokenMixShares({ input: 0, output: 0, cacheRead: 0, cacheWrite: 0, reasoning: 0 }).input).toBe(0);
  });
});

describe("buildModelDetail", () => {
  it("zero-fills the trend and splits usage by agent and human", () => {
    const rollup = buildPeriodRollup({
      window: window7d,
      facts: [fact("a", "claude", "claude-opus-4-5", 600), fact("b", "kilocode", "claude-opus-4-5", 200), fact("a", "codex", "gpt-5.6-sol", 300)],
      users: [user("a", 900), user("b", 200)],
      catalog,
    });
    const cell = (username: string, client: string, tokens: number) => ({
      username,
      displayName: null,
      avatarUrl: null,
      client,
      tokens,
      cost: tokens / 1e6,
      input: tokens,
      output: 0,
      cacheRead: 0,
      cacheWrite: 0,
      reasoning: 0,
    });
    const detail = buildModelDetail({
      key: "claude-opus-4-5",
      window: window7d,
      rollup,
      catalog,
      trend: [{ date: "2026-10-15", activeUsers: 2, tokens: 800 }],
      cells: [cell("user-a", "claude", 600), cell("user-b", "kilocode", 200)],
      userDays: { "user-a": 1, "user-b": 1 },
    });
    expect(detail.model).toMatchObject({ key: "claude-opus-4-5", name: "Claude Opus 4.5", aliases: ["claude-opus-4.5", "claude-opus-4-5-20251101"] });
    expect(detail.trend).toHaveLength(7);
    expect(detail.trend.find((p) => p.date === "2026-10-15")).toEqual({ date: "2026-10-15", activeUsers: 2, tokens: 800 });
    expect(detail.trend.find((p) => p.date === "2026-10-13")).toEqual({ date: "2026-10-13", activeUsers: 0, tokens: 0 });
    expect(detail.summary).toMatchObject({ rank: 1, activeUsers: 2, tokens: 800, firstSeen: "2026-01-02" });
    expect(detail.summary.tokenMix.input).toBe(1);
    expect(detail.agents.map((a) => [a.key, a.activeUsers, a.share])).toEqual([
      ["claude", 1, 0.75],
      ["kilo", 1, 0.25],
    ]);
    expect(detail.humans.map((h) => h.username)).toEqual(["user-a", "user-b"]);
    expect(detail.related).toEqual([]);
  });
});
