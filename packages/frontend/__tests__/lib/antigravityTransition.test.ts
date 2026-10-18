import { describe, expect, it } from "vitest";
import { antigravityPriorCoverage } from "../../src/lib/db/antigravityStateCoverage";
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

  it("replaces when historical gemini-default, unknown, or thinking model IDs are re-attributed to concrete models", () => {
    const args = baseArgs();
    args.existingDays = [
      {
        date: "2026-07-18",
        sourceBreakdown: {
          "antigravity-cli": {
            tokens: 69_226_019,
            input: 60_000_000,
            output: 1_000_000,
            cacheRead: 8_000_000,
            cacheWrite: 0,
            reasoning: 226_019,
            messages: 500,
            cost: 12,
            models: {
              "gemini-default": {
                tokens: 69_226_019,
                input: 60_000_000,
                output: 1_000_000,
                cacheRead: 8_000_000,
                cacheWrite: 0,
                reasoning: 226_019,
                messages: 500,
                cost: 12,
              },
            },
          },
        },
      },
      {
        date: "2026-08-27",
        sourceBreakdown: {
          "antigravity-cli": {
            tokens: 10_000,
            input: 8_000,
            output: 2_000,
            cacheRead: 0,
            cacheWrite: 0,
            reasoning: 0,
            messages: 4,
            cost: 0.1,
            models: {
              unknown: {
                tokens: 3_000,
                input: 3_000,
                output: 0,
                cacheRead: 0,
                cacheWrite: 0,
                reasoning: 0,
                messages: 2,
                cost: 0,
              },
              "claude-opus-4-6-thinking": {
                tokens: 7_000,
                input: 5_000,
                output: 2_000,
                cacheRead: 0,
                cacheWrite: 0,
                reasoning: 0,
                messages: 2,
                cost: 0.1,
              },
            },
          },
        },
      },
    ];
    args.contributions = [
      {
        date: "2026-07-18",
        clients: [
          {
            client: "antigravity-cli",
            modelId: "gemini-3.5-flash-extra-low",
            tokens: {
              input: 25_000_000,
              output: 400_000,
              cacheRead: 4_856_468,
              cacheWrite: 0,
              reasoning: 100_000,
            },
            cost: 5,
            messages: 220,
          },
          {
            client: "antigravity-cli",
            modelId: "gemini-3.5-flash-medium",
            tokens: {
              input: 35_000_000,
              output: 600_000,
              cacheRead: 3_143_532,
              cacheWrite: 0,
              reasoning: 126_019,
            },
            cost: 7,
            messages: 280,
          },
        ],
      },
      {
        date: "2026-08-27",
        clients: [
          {
            client: "antigravity-cli",
            modelId: "gemini-3.7-flash",
            tokens: {
              input: 3_000,
              output: 0,
              cacheRead: 0,
              cacheWrite: 0,
              reasoning: 0,
            },
            cost: 0.01,
            messages: 2,
          },
          {
            client: "antigravity-cli",
            modelId: "claude-opus-4-6",
            tokens: {
              input: 5_000,
              output: 2_000,
              cacheRead: 0,
              cacheWrite: 0,
              reasoning: 0,
            },
            cost: 0.1,
            messages: 2,
          },
        ],
      },
      {
        date: "2026-10-18",
        clients: [
          {
            client: "antigravity-cli",
            modelId: "gemini-4-argon-trial",
            tokens: {
              input: 173_302,
              output: 1_857,
              cacheRead: 1_172_876,
              cacheWrite: 0,
              reasoning: 1_077,
            },
            cost: 0.4932,
            messages: 10,
          },
        ],
      },
    ];

    const plan = planAntigravityTransition(args);
    expect(plan.mode).toBe("replace");
    expect(
      plan.layouts?.["antigravity-cli"]?.["2026-10-18"]?.models[
        "gemini-4-argon-trial"
      ]?.tokens
    ).toBe(1_349_112);
  });

  it("rebuilds schema v1 parserStates from credited existingDays instead of freezing on uncredited cellwise-max inflation", () => {
    const args = baseArgs();
    const creditedCell = {
      tokens: 100_000,
      input: 100_000,
      output: 0,
      cacheRead: 0,
      cacheWrite: 0,
      reasoning: 0,
      messages: 5,
      cost: 1,
      models: {
        "gemini-default": {
          tokens: 100_000,
          input: 100_000,
          output: 0,
          cacheRead: 0,
          cacheWrite: 0,
          reasoning: 0,
          messages: 5,
          cost: 1,
        },
      },
    };
    args.existingDays = [
      {
        date: "2026-08-07",
        sourceBreakdown: { "antigravity-cli": creditedCell },
      },
    ];
    // Schema v1 (stateVersion omitted) retained the old session-start day in
    // `days` alongside the re-dated day (`advanceDays`), inflating its sum to
    // 200,000 even though only 100,000 was ever credited in `existingDays`.
    args.parserStates = {
      "antigravity-cli": {
        version: 1,
        baselineEstablished: true,
        aggregate: {
          tokens: 100_000,
          input: 100_000,
          output: 0,
          cacheRead: 0,
          cacheWrite: 0,
          reasoning: 0,
          messages: 5,
          inputIncludingCacheRead: 100_000,
        },
        days: {
          "2026-08-07": creditedCell,
          "2026-08-08": creditedCell,
        },
      },
    };
    args.contributions = [
      {
        date: "2026-08-08",
        clients: [
          {
            client: "antigravity-cli",
            modelId: "gemini-3.5-flash-medium",
            tokens: {
              input: 120_000,
              output: 0,
              cacheRead: 0,
              cacheWrite: 0,
              reasoning: 0,
            },
            cost: 1.2,
            messages: 6,
          },
        ],
      },
    ];

    const plan = planAntigravityTransition(args);
    expect(plan.mode).toBe("replace");
  });

  it("deduplicates legacy ledgers across surfaces even when a model ID was canonicalized between them", () => {
    const args = baseArgs();
    const makeCell = (modelId: string) => ({
      tokens: 100_000,
      input: 100_000,
      output: 0,
      cacheRead: 0,
      cacheWrite: 0,
      reasoning: 0,
      messages: 5,
      cost: 1,
      models: {
        [modelId]: {
          tokens: 100_000,
          input: 100_000,
          output: 0,
          cacheRead: 0,
          cacheWrite: 0,
          reasoning: 0,
          messages: 5,
          cost: 1,
        },
      },
    });
    args.existingDays = [
      {
        date: "2026-08-07",
        sourceBreakdown: {
          "antigravity-cli": makeCell("claude-opus-4-6"),
        },
      },
    ];
    args.parserStates = {
      antigravity: {
        stateVersion: 2,
        version: 1,
        baselineEstablished: true,
        aggregate: {
          tokens: 100_000,
          input: 100_000,
          output: 0,
          cacheRead: 0,
          cacheWrite: 0,
          reasoning: 0,
          messages: 5,
          inputIncludingCacheRead: 100_000,
        },
        days: { "2026-08-07": makeCell("claude-opus-4-6-thinking") },
        observedDays: { "2026-08-07": makeCell("claude-opus-4-6-thinking") },
      },
      "antigravity-cli": {
        stateVersion: 2,
        version: 1,
        baselineEstablished: true,
        aggregate: {
          tokens: 100_000,
          input: 100_000,
          output: 0,
          cacheRead: 0,
          cacheWrite: 0,
          reasoning: 0,
          messages: 5,
          inputIncludingCacheRead: 100_000,
        },
        days: { "2026-08-07": makeCell("claude-opus-4-6") },
        observedDays: { "2026-08-07": makeCell("claude-opus-4-6") },
      },
    };
    args.contributions = [
      {
        date: "2026-08-07",
        clients: [
          {
            client: "antigravity-cli",
            modelId: "claude-opus-4-6",
            tokens: {
              input: 100_000,
              output: 0,
              cacheRead: 0,
              cacheWrite: 0,
              reasoning: 0,
            },
            cost: 1,
            messages: 5,
          },
        ],
      },
    ];

    const plan = planAntigravityTransition(args);
    expect(plan.mode).toBe("replace");
  });

  it("keeps distinct canonical models across legacy ledgers separate even when their seven-field totals match", () => {
    const args = baseArgs();
    const makeCell = (modelId: string) => ({
      tokens: 100_000,
      input: 100_000,
      output: 0,
      cacheRead: 0,
      cacheWrite: 0,
      reasoning: 0,
      messages: 5,
      cost: 1,
      models: {
        [modelId]: {
          tokens: 100_000,
          input: 100_000,
          output: 0,
          cacheRead: 0,
          cacheWrite: 0,
          reasoning: 0,
          messages: 5,
          cost: 1,
        },
      },
    });
    args.existingDays = [
      {
        date: "2026-08-07",
        sourceBreakdown: {
          "antigravity-cli": makeCell("gemini-3-pro"),
        },
      },
    ];
    args.parserStates = {
      antigravity: {
        stateVersion: 2,
        version: 1,
        baselineEstablished: true,
        aggregate: {
          tokens: 100_000,
          input: 100_000,
          output: 0,
          cacheRead: 0,
          cacheWrite: 0,
          reasoning: 0,
          messages: 5,
          inputIncludingCacheRead: 100_000,
        },
        days: { "2026-08-07": makeCell("claude-sonnet-4-6") },
        observedDays: { "2026-08-07": makeCell("claude-sonnet-4-6") },
      },
      "antigravity-cli": {
        stateVersion: 2,
        version: 1,
        baselineEstablished: true,
        aggregate: {
          tokens: 100_000,
          input: 100_000,
          output: 0,
          cacheRead: 0,
          cacheWrite: 0,
          reasoning: 0,
          messages: 5,
          inputIncludingCacheRead: 100_000,
        },
        days: { "2026-08-07": makeCell("gemini-3-pro") },
        observedDays: { "2026-08-07": makeCell("gemini-3-pro") },
      },
    };
    args.contributions = [
      {
        date: "2026-08-07",
        clients: [
          {
            client: "antigravity-cli",
            modelId: "gemini-3-pro",
            tokens: {
              input: 150_000,
              output: 0,
              cacheRead: 0,
              cacheWrite: 0,
              reasoning: 0,
            },
            cost: 1.5,
            messages: 7,
          },
        ],
      },
    ];

    // 100,000 (claude-sonnet-4-6) + 100,000 (gemini-3-pro) = 200,000 distinct
    // credited tokens; a 150,000 snapshot does not cover the lifetime and freezes.
    expect(planAntigravityTransition(args).mode).toBe("freeze");
  });

  it("does not collapse two tier models within the same ledger when their seven-field vectors match", () => {
    const args = baseArgs();
    const tierModel = {
      tokens: 50_000,
      input: 50_000,
      output: 0,
      cacheRead: 0,
      cacheWrite: 0,
      reasoning: 0,
      messages: 3,
      cost: 0.5,
    };
    const twoTierCell = {
      tokens: 100_000,
      input: 100_000,
      output: 0,
      cacheRead: 0,
      cacheWrite: 0,
      reasoning: 0,
      messages: 6,
      cost: 1,
      models: {
        "gemini-3.5-flash-extra-low": tierModel,
        "gemini-3.5-flash-medium": tierModel,
      },
    };
    args.parserStates = {
      "antigravity-cli": {
        stateVersion: 2,
        version: 1,
        baselineEstablished: true,
        aggregate: {
          tokens: 100_000,
          input: 100_000,
          output: 0,
          cacheRead: 0,
          cacheWrite: 0,
          reasoning: 0,
          messages: 6,
          inputIncludingCacheRead: 100_000,
        },
        days: { "2026-08-07": twoTierCell },
        observedDays: { "2026-08-07": twoTierCell },
      },
    };
    args.contributions = [
      {
        date: "2026-08-07",
        clients: [
          {
            client: "antigravity-cli",
            modelId: "gemini-3.5-flash-medium",
            tokens: {
              input: 60_000,
              output: 0,
              cacheRead: 0,
              cacheWrite: 0,
              reasoning: 0,
            },
            cost: 0.6,
            messages: 6,
          },
        ],
      },
    ];

    // Both 50,000 tier models belong to the same ledger (total 100,000), so a
    // 60,000 snapshot is below the credited total and must freeze.
    expect(planAntigravityTransition(args).mode).toBe("freeze");
  });
});

describe("antigravityPriorCoverage flash tier canonicalization", () => {
  const cell = (modelId: string) => ({
    tokens: 100_000,
    input: 100_000,
    output: 0,
    cacheRead: 0,
    cacheWrite: 0,
    reasoning: 0,
    messages: 5,
    cost: 1,
    models: {
      [modelId]: {
        tokens: 100_000,
        input: 100_000,
        output: 0,
        cacheRead: 0,
        cacheWrite: 0,
        reasoning: 0,
        messages: 5,
        cost: 1,
      },
    },
  });
  const ledger = (modelId: string) => ({
    stateVersion: 2,
    version: 1,
    baselineEstablished: true,
    aggregate: {
      tokens: 100_000,
      input: 100_000,
      output: 0,
      cacheRead: 0,
      cacheWrite: 0,
      reasoning: 0,
      messages: 5,
      inputIncludingCacheRead: 100_000,
    },
    days: { "2026-08-07": cell(modelId) },
    observedDays: { "2026-08-07": cell(modelId) },
  });

  it("folds a resolved flash tier with its legacy gemini-default cell across surfaces", () => {
    const prior = antigravityPriorCoverage([], {
      antigravity: ledger("gemini-default"),
      "antigravity-cli": ledger("gemini-3.5-flash-medium"),
    });
    expect(prior.unverifiable).toBe(false);
    expect(prior.total.tokens).toBe(100_000);
  });

  it("keeps a non-tier gemini-3.5-flash variant apart from a flash tier", () => {
    const prior = antigravityPriorCoverage([], {
      antigravity: ledger("gemini-3.5-flash-high"),
      "antigravity-cli": ledger("gemini-3.5-flash-lite"),
    });
    expect(prior.unverifiable).toBe(false);
    expect(prior.total.tokens).toBe(200_000);
  });
});
