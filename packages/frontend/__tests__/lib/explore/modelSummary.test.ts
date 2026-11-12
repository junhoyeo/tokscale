import { afterEach, describe, expect, it, vi } from "vitest";

const getPeriodRollup = vi.fn();
const getModelDetailCached = vi.fn();

vi.mock("@/lib/explore/db", () => ({ getPeriodRollup, getModelDetailCached }));

afterEach(() => {
  vi.unstubAllEnvs();
  vi.clearAllMocks();
});

describe("getModelSummary", () => {
  it("returns the model's row from the cached period rollup, without the detail scan", async () => {
    vi.stubEnv("DATABASE_URL", "postgresql://example.invalid/tokscale");
    const row = { key: "claude-opus-5", name: "Claude Opus 5", activeUsers: 42, tokens: 1000 };
    getPeriodRollup.mockResolvedValue({ models: [{ key: "gpt-5", name: "GPT-5" }, row] });
    const { getModelSummary } = await import("@/lib/explore");

    expect(await getModelSummary("claude-opus-5", "7d")).toBe(row);
    expect(await getModelSummary("no-such-model", "7d")).toBeNull();
    expect(getPeriodRollup.mock.calls).toEqual([["7d"], ["7d"]]);
    expect(getModelDetailCached).not.toHaveBeenCalled();
  });

  it("does not look up a model that was never identified", async () => {
    vi.stubEnv("DATABASE_URL", "postgresql://example.invalid/tokscale");
    const { getModelSummary } = await import("@/lib/explore");

    expect(await getModelSummary("unknown", "30d")).toBeNull();
    expect(getPeriodRollup).not.toHaveBeenCalled();
  });
});
