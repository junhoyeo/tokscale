import { describe, expect, it } from "vitest";
import { buildModelCatalog, buildPeriodRollup, type UsageFact } from "@/lib/explore/rollup";
import { resolveWindow } from "@/lib/explore/window";
import { describeCoverage } from "@/components/explore/format";

describe("identified-model coverage", () => {
  it("includes scalar-only legacy tokens in the denominator", () => {
    const window = resolveWindow("30d", { firstDate: "2026-01-01", latestDate: "2026-10-19" }, "2026-10-19");
    const facts: UsageFact[] = [{ userId: "identified", client: "codex", model: "gpt-5.5", current: true,
      tokens: 100, cost: 1, input: 100, output: 0, cacheRead: 0, cacheWrite: 0, reasoning: 0 }];
    const users = [100, 900].map((tokens, i) => ({ userId: i ? "legacy" : "identified", username: `person-${i}`,
      displayName: null, avatarUrl: null, tokens, cost: 1, activeDays: 1, lastSubmittedAt: null }));
    const catalog = buildModelCatalog([{ client: "codex", model: "gpt-5.5", firstSeen: "2026-10-19", tokens: 100 }]);
    const { coverage } = buildPeriodRollup({ window, facts, users, catalog });
    expect(coverage.accounts).toBe(2);
    expect(coverage.modelIdentifiedShare).toBe(0.1);
    expect(describeCoverage(coverage).identified).toContain("10%");
  });
});
