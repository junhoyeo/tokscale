import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";

vi.mock("@/lib/explore/db", () => {
  throw new Error("the database layer must not load without DATABASE_URL");
});

describe("@/lib/explore without DATABASE_URL", () => {
  const saved = process.env.DATABASE_URL;
  beforeEach(() => {
    delete process.env.DATABASE_URL;
  });
  afterEach(() => {
    if (saved === undefined) delete process.env.DATABASE_URL;
    else process.env.DATABASE_URL = saved;
  });

  it("returns fixture rollups with the same contract", async () => {
    const explore = await import("@/lib/explore");
    const models = await explore.getModelLeaderboard({ period: "7d", sort: "tokens", limit: 3 });
    expect(models.rows).toHaveLength(3);
    expect(models.rows.map((r) => r.rank)).toEqual([1, 2, 3]);
    expect(models.rows[0].tokens).toBeGreaterThanOrEqual(models.rows[1].tokens);
    expect(models.coverage.period).toBe("7d");

    const agents = await explore.getAgentLeaderboard({ offset: 1, limit: 2 });
    expect(agents.rows.map((r) => r.rank)).toEqual([2, 3]);

    const humans = await explore.getHumansLeaderboard({ period: "all" });
    expect(humans.rows.length).toBeGreaterThan(0);

    const snapshot = await explore.getCommunitySnapshot("30d");
    expect(snapshot.models.length).toBeLessThanOrEqual(5);

    const keys = await explore.listModelKeys();
    expect(keys).toContain("claude-opus-5");
    expect(await explore.getModelDetail("claude-opus-5", "7d")).not.toBeNull();
    expect(await explore.getModelDetail("does-not-exist")).toBeNull();
  });
  it.each(["production", undefined])("does not invent usage in %s without a database", async (mode) => {
    vi.stubEnv("NODE_ENV", mode);
    try {
      const explore = await import("@/lib/explore");
      const loaders = [
        () => explore.getCommunitySnapshot(), () => explore.getModelLeaderboard(),
        () => explore.getAgentLeaderboard(), () => explore.getHumansLeaderboard(),
        () => explore.getModelDetail("claude-opus-5"), () => explore.listModelKeys(),
      ];
      for (const load of loaders) await expect(load()).rejects.toThrow("DATABASE_URL is required");
    } finally {
      vi.unstubAllEnvs();
    }
  });

});
