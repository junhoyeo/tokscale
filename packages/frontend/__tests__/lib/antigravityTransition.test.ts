import { describe, expect, it } from "vitest";
import { planAntigravityTransition } from "../../src/lib/db/antigravityTransition";
import { SUPPORTED_VERSIONED_PARSERS } from "../../src/lib/db/parserHighWater";

const GEN = SUPPORTED_VERSIONED_PARSERS["antigravity-cli"];

function day(
  date: string,
  client: string,
  tokens: number,
  cost: number,
  costIsComplete?: boolean
) {
  return {
    date,
    clients: [
      {
        client,
        modelId: "gemini-3-pro",
        tokens: {
          input: tokens,
          output: 0,
          cacheRead: 0,
          cacheWrite: 0,
          reasoning: 0,
        },
        cost,
        messages: 2,
      },
    ],
    ...(costIsComplete === undefined ? {} : { totals: { costIsComplete } }),
  };
}

function fullTrioVersions() {
  return Object.fromEntries(
    (["antigravity", "antigravity-cli", "antigravity-extension"] as const).map(
      (client) => [client, SUPPORTED_VERSIONED_PARSERS[client]]
    )
  );
}

function baseArgs(): Parameters<typeof planAntigravityTransition>[0] {
  return {
    submittedClients: new Set<string>(["antigravity-cli", "antigravity-extension"]),
    incomingVersions: fullTrioVersions(),
    persistedVersions: undefined,
    parserStates: undefined,
    fullHistory: true,
    isBackfill: false,
    contributions: [],
    existingDays: [],
  };
}

describe("planAntigravityTransition incomplete-pricing admission", () => {
  it("admits cost-incomplete usage when the full family declares supported generations", () => {
    const args = baseArgs();
    args.contributions = [
      // An unpriced Gemini model on a costIsComplete: false day. This is the
      // shape that used to freeze every later full-history submit.
      day("2026-09-20", "antigravity-cli", 1_000_000_000, 0, false),
      day("2026-09-21", "antigravity-extension", 2_100_000_000, 21, false),
    ];
    const plan = planAntigravityTransition(args);

    expect(plan.mode).toBe("replace");
    // All three pins, exactly the supported generations — the identity the
    // route persists.
    expect(plan.parserVersions).toEqual({
      antigravity: GEN,
      "antigravity-cli": GEN,
      "antigravity-extension": GEN,
    });
    expect(plan.layouts?.["antigravity-cli"]?.["2026-09-20"]).toBeDefined();
    expect(plan.layouts?.["antigravity-extension"]?.["2026-09-21"]).toBeDefined();
    // The absent desktop source starts empty rather than blocking the family.
    expect(plan.layouts?.["antigravity"]).toEqual({});
    expect(plan.warning).toContain("partial");
  });

  it("still freezes an unsupported incoming generation before any admission", () => {
    const args = baseArgs();
    args.contributions = [day("2026-09-20", "antigravity-cli", 100, 1, false)];
    const future = { ...args.incomingVersions, "antigravity-cli": GEN + 1 };
    const plan = planAntigravityTransition({ ...args, incomingVersions: future });
    expect(plan.mode).toBe("freeze");
    // Future declarations are pinned; tokens are not admitted.
    expect(plan.parserVersions).toEqual({ "antigravity-cli": GEN + 1 });
  });

  it("still freezes admission when a stored family generation is newer than supported", () => {
    const args = baseArgs();
    args.persistedVersions = { "antigravity-extension": GEN + 1 };
    const plan = planAntigravityTransition(args);
    expect(plan.mode).toBe("freeze");
    expect(plan.parserVersions).toBeUndefined();
  });

  it("still freezes a member that omits its generation", () => {
    const args = baseArgs();
    const partial = { ...args.incomingVersions } as Record<string, number>;
    delete partial["antigravity"];
    const plan = planAntigravityTransition({ ...args, incomingVersions: partial });
    expect(plan.mode).toBe("freeze");
  });

  it("still freezes backfills, partial history, and coverage deficits", () => {
    const args = baseArgs();
    args.contributions = [day("2026-09-20", "antigravity-cli", 100, 1, false)];
    expect(
      planAntigravityTransition({ ...args, isBackfill: true }).mode
    ).toBe("freeze");
    expect(
      planAntigravityTransition({ ...args, fullHistory: false }).mode
    ).toBe("freeze");

    const credited = {
      ...args,
      existingDays: [
        {
          date: "2026-09-19",
          sourceBreakdown: {
            "antigravity-cli": {
              tokens: 5000,
              input: 5000,
              output: 0,
              cacheRead: 0,
              cacheWrite: 0,
              reasoning: 0,
              messages: 10,
              cost: 5,
              models: {},
            },
          },
        },
      ],
    };
    // The incomplete snapshot does not cover the credited lifetime.
    expect(planAntigravityTransition(credited).mode).toBe("freeze");
  });

  it("keeps complete pricing on the exact-replacement path", () => {
    const args = baseArgs();
    args.contributions = [
      day("2026-09-20", "antigravity-cli", 100, 1),
      day("2026-09-21", "antigravity-extension", 200, 2),
    ];
    const plan = planAntigravityTransition(args);
    expect(plan.mode).toBe("replace");
    expect(plan.warning).not.toContain("partial");
  });
});
