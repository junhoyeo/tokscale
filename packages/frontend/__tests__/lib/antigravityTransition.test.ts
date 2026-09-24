import { describe, expect, it } from "vitest";
import { planAntigravityTransition } from "../../src/lib/db/antigravityTransition";
import { SUPPORTED_VERSIONED_PARSERS } from "../../src/lib/db/parserHighWater";

function day(
  date: string,
  client: string,
  tokens: number,
  costIsComplete?: boolean
) {
  return {
    date,
    clients: [
      {
        client,
        modelId: "gemini-3-flash",
        tokens: {
          input: tokens,
          output: 0,
          cacheRead: 0,
          cacheWrite: 0,
          reasoning: 0,
        },
        cost: tokens / 100,
        messages: 2,
      },
    ],
    ...(costIsComplete === undefined ? {} : { totals: { costIsComplete } }),
  };
}

const GEN = SUPPORTED_VERSIONED_PARSERS["antigravity-cli"];

function baseArgs() {
  return {
    submittedClients: new Set(["antigravity-cli", "antigravity-extension"]),
    incomingVersions: {
      "antigravity-cli": GEN,
      "antigravity-extension": GEN,
    } as Record<string, number>,
    persistedVersions: undefined,
    parserStates: undefined,
    fullHistory: true,
    isBackfill: false,
    contributions: [
      day("2026-09-20", "antigravity-cli", 1000, false),
      day("2026-09-21", "antigravity-extension", 500),
    ],
    existingDays: [],
  };
}

describe("planAntigravityTransition first admission", () => {
  it("admits a partial family with incomplete pricing when nothing is credited yet", () => {
    const plan = planAntigravityTransition(baseArgs());
    expect(plan.mode).toBe("replace");
    expect(plan.layouts?.["antigravity-cli"]?.["2026-09-20"]).toBeDefined();
    expect(plan.layouts?.["antigravity-extension"]?.["2026-09-21"]).toBeDefined();
    // Missing surface starts empty rather than blocking the family.
    expect(plan.layouts?.["antigravity"]).toEqual({});
  });

  it("freezes first admission on an explicitly unsupported parser generation", () => {
    const plan = planAntigravityTransition({
      ...baseArgs(),
      incomingVersions: { "antigravity-cli": GEN + 1, "antigravity-extension": GEN },
    });
    expect(plan.mode).toBe("freeze");
  });

  it("freezes first admission for backfills and partial scans", () => {
    expect(
      planAntigravityTransition({ ...baseArgs(), isBackfill: true }).mode
    ).toBe("freeze");
    expect(
      planAntigravityTransition({ ...baseArgs(), fullHistory: false }).mode
    ).toBe("freeze");
  });

  it("freezes a partial family once the device has credited usage", () => {
    const plan = planAntigravityTransition({
      ...baseArgs(),
      existingDays: [
        {
          date: "2026-09-19",
          sourceBreakdown: {
            "antigravity-cli": {
              tokens: 2000,
              input: 2000,
              output: 0,
              cacheRead: 0,
              cacheWrite: 0,
              reasoning: 0,
              messages: 4,
              cost: 20,
              models: {},
            },
          },
        },
      ],
    });
    // Old behavior preserved: partial trio + prior coverage stays frozen.
    expect(plan.mode).toBe("freeze");
  });

  it("returns status-quo when the family is untouched", () => {
    const plan = planAntigravityTransition({
      ...baseArgs(),
      submittedClients: new Set(["claude"]),
      incomingVersions: {},
      contributions: [day("2026-09-20", "claude", 100)],
    });
    expect(plan.mode).toBe("status-quo");
  });
});
