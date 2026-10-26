import { describe, expect, it } from "vitest";
import {
  addClientBreakdownIncrement,
  applyRecountProofToCell,
  foldParserClientSnapshot,
  planParserHighWaterSubmission,
  recountCostUnsafeModels,
  recountProofProvesCost,
  scanScopeCoversFullHistory,
  SUPPORTED_VERSIONED_PARSERS,
  type ClientRecountProof,
  type ParserClientHighWaterState,
  type ParserHighWaterPlan,
  type RecountProofCell,
  type RecountProofEntry,
} from "../../src/lib/db/parserHighWater";
import {
  mergeClientBreakdownsWithRegressionGuard,
  recalculateDayTotals,
  type ClientBreakdownData,
} from "../../src/lib/db/helpers";

function contribution(
  date: string,
  tokens: number,
  modelId = "model-a",
  cost = tokens / 10
) {
  return {
    date,
    clients: [
      {
        client: "copilot",
        modelId,
        tokens: {
          input: tokens,
          output: 0,
          cacheRead: 0,
          cacheWrite: 0,
          reasoning: 0,
        },
        cost,
        messages: tokens > 0 ? 1 : 0,
      },
    ],
  };
}

function bucketContribution(
  date: string,
  tokens: {
    input: number;
    output: number;
    cacheRead: number;
    cacheWrite: number;
    reasoning: number;
  },
  cost: number,
  messages: number,
  modelId = "model-a"
) {
  return {
    date,
    clients: [{ client: "copilot", modelId, tokens, cost, messages }],
  };
}

function snapshot(...rows: ReturnType<typeof contribution>[]) {
  return foldParserClientSnapshot(rows, "copilot");
}

function legacy(...rows: ReturnType<typeof contribution>[]) {
  return snapshot(...rows);
}

function baseline(
  existingLegacyDays: Record<string, ClientBreakdownData>,
  incomingDays: Record<string, ClientBreakdownData>
) {
  return planParserHighWaterSubmission({
    client: "copilot",
    incomingVersion: 2,
    fullHistory: true,
    existingLegacyDays,
    incomingDays,
  });
}

function next(
  state: ParserClientHighWaterState,
  incomingDays: Record<string, ClientBreakdownData>
) {
  return planParserHighWaterSubmission({
    client: "copilot",
    incomingVersion: 2,
    fullHistory: true,
    existingLegacyDays: {},
    incomingDays,
    state,
  });
}

/**
 * Assert a replay credited nothing *because there was nothing to credit*.
 *
 * `increments` alone cannot carry that meaning: a frozen plan returns
 * `{ mode: "freeze", increments: {} }`, so asserting an empty increment map
 * passes just as happily when the state was rejected on read. Those two
 * outcomes could not be further apart -- one is correct idempotency, the other
 * is a user whose high-water stopped accepting anything at all -- so the mode
 * has to be checked alongside it.
 */
function expectCreditedNothing(plan: ParserHighWaterPlan) {
  expect(plan.increments).toEqual({});
  expect(plan.mode).not.toBe("freeze");
}


/** `applyRecountProofToCell` contexts used below. */
const AT_TRANSITION = {
  transition: true,
  costIsComplete: true,
  storedCostIsFloor: false,
  costUnsafeModels: new Set<string>() as ReadonlySet<string>,
} as const;
const AT_TRANSITION_FLOOR = { ...AT_TRANSITION, storedCostIsFloor: true } as const;
describe("non-destructive parser generation high-water", () => {
  it("treats prototype-named model IDs as ordinary untrusted keys", () => {
    const first = baseline(
      {},
      snapshot(contribution("2026-07-01", 100, "__proto__", 10))
    );
    const plan = next(
      first.nextState!,
      snapshot(contribution("2026-07-01", 150, "__proto__", 15))
    );

    const model = plan.increments["2026-07-01"].models["__proto__"];
    expect(Object.getPrototypeOf(plan.increments["2026-07-01"].models)).toBeNull();
    expect(model).toMatchObject({ input: 50, tokens: 50, cost: 5 });
    expect((Object.prototype as { input?: number }).input).toBeUndefined();
  });

  it("preserves legacy rows and records the first v2 full snapshot as a no-add baseline", () => {
    const plan = baseline(
      legacy(contribution("2026-07-01", 100)),
      snapshot(contribution("2026-07-02", 100))
    );

    expect(plan.mode).toBe("baseline-legacy");
    expect(plan.increments).toEqual({});
    expect(plan.nextState?.version).toBe(2);
    expect(plan.nextState?.aggregate.tokens).toBe(100);
  });

  it.each([
    { incoming: 50, expected: 0 },
    { incoming: 100, expected: 0 },
    { incoming: 150, expected: 50 },
  ])(
    "credits only bounded transition growth when deleted legacy usage is replaced by $incoming new tokens",
    ({ incoming, expected }) => {
      const plan = baseline(
        legacy(contribution("2026-06-01", 100, "legacy-model", 10)),
        snapshot(contribution("2026-07-01", incoming, "new-model", 15))
      );
      const credited = Object.values(plan.increments).reduce(
        (sum, day) => sum + day.tokens,
        0
      );

      expect(plan.mode).toBe("baseline-legacy");
      expect(credited).toBe(expected);
      expect(plan.nextState?.aggregate.tokens).toBe(Math.max(100, incoming));
    }
  );

  it("does not mint transition usage from a pure token-bucket move", () => {
    const plan = baseline(
      snapshot(
        bucketContribution(
          "2026-07-01",
          { input: 100, output: 0, cacheRead: 0, cacheWrite: 0, reasoning: 0 },
          10,
          1
        )
      ),
      snapshot(
        bucketContribution(
          "2026-07-02",
          { input: 0, output: 100, cacheRead: 0, cacheWrite: 0, reasoning: 0 },
          10,
          1
        )
      )
    );

    expect(plan.increments).toEqual({});
  });

  it("keeps a zero-token legacy message inside the lifetime high-water", () => {
    const legacyMessage = snapshot(
      bucketContribution(
        "2026-07-01",
        { input: 0, output: 0, cacheRead: 0, cacheWrite: 0, reasoning: 0 },
        0,
        1
      )
    );
    const movedMessage = snapshot(
      bucketContribution(
        "2026-07-02",
        { input: 0, output: 0, cacheRead: 0, cacheWrite: 0, reasoning: 0 },
        0,
        1
      )
    );
    const plan = baseline(legacyMessage, movedMessage);

    expect(plan.mode).toBe("baseline-legacy");
    expect(plan.increments).toEqual({});
    expect(plan.nextState?.aggregate).toMatchObject({ tokens: 0, messages: 1 });
  });

  it.each([
    { legacyDate: "2026-07-02", newDate: "2026-07-01" },
    { legacyDate: "2026-07-01", newDate: "2026-07-02" },
  ])(
    "derives a prior model from scalar legacy rows before allocating growth ($legacyDate -> $newDate)",
    ({ legacyDate, newDate }) => {
      const scalarLegacy = {
        tokens: 100,
        cost: 10,
        input: 100,
        output: 0,
        cacheRead: 0,
        cacheWrite: 0,
        reasoning: 0,
        messages: 1,
        modelId: "model-a",
        models: {},
      } satisfies ClientBreakdownData;
      const plan = baseline(
        { [legacyDate]: scalarLegacy },
        snapshot(
          contribution(legacyDate, 100, "model-a", 10),
          contribution(newDate, 50, "model-b", 5)
        )
      );

      expect(plan.increments[legacyDate]).toBeUndefined();
      expect(plan.increments[newDate].models["model-b"]).toMatchObject({
        input: 50,
        tokens: 50,
        cost: 5,
      });
    }
  );

  it.each([
    { legacyDate: "2026-07-02", newDate: "2026-07-01" },
    { legacyDate: "2026-07-01", newDate: "2026-07-02" },
  ])(
    "credits deferred growth when a truncated first scan is restored ($legacyDate -> $newDate)",
    ({ legacyDate, newDate }) => {
      const scalarLegacy = {
        tokens: 100,
        cost: 10,
        input: 100,
        output: 0,
        cacheRead: 0,
        cacheWrite: 0,
        reasoning: 0,
        messages: 1,
        modelId: "model-a",
        models: {},
      } satisfies ClientBreakdownData;
      const first = baseline(
        { [legacyDate]: scalarLegacy },
        snapshot(
          contribution(legacyDate, 80, "model-a", 8),
          contribution(newDate, 50, "model-b", 5)
        )
      );
      const restored = next(
        first.nextState!,
        snapshot(
          contribution(legacyDate, 100, "model-a", 10),
          contribution(newDate, 50, "model-b", 5)
        )
      );

      expect(first.increments[newDate].tokens).toBe(30);
      expect(
        first.nextState?.days[legacyDate].models["model-a"].input
      ).toBe(100);
      expect(restored.increments[legacyDate]).toBeUndefined();
      expect(restored.increments[newDate].models["model-b"]).toMatchObject({
        input: 20,
        tokens: 20,
        cost: 2,
      });
      expect(restored.nextState?.aggregate.tokens).toBe(150);
    }
  );

  it("preserves scalar usage not represented by a partial nested-model map", () => {
    const nested = snapshot(
      contribution("2026-07-01", 10, "model-a", 1)
    )["2026-07-01"];
    const mixedLegacy = {
      ...nested,
      tokens: 15,
      input: 15,
      cost: 1.5,
      messages: 2,
      modelId: "model-b",
    } satisfies ClientBreakdownData;
    const incoming = snapshot(
      contribution("2026-07-01", 10, "model-a", 1),
      contribution("2026-07-01", 5, "model-b", 0.5)
    );
    const first = baseline({ "2026-07-01": mixedLegacy }, incoming);
    const replay = next(first.nextState!, incoming);

    expect(first.increments).toEqual({});
    expect(first.nextState?.aggregate).toMatchObject({
      tokens: 15,
      input: 15,
      messages: 2,
    });
    expect(first.nextState?.days["2026-07-01"].models["model-b"]).toMatchObject({
      tokens: 5,
      input: 5,
      cost: 0.5,
      messages: 1,
    });
    expectCreditedNothing(replay);
  });

  it("normalizes old nested models that predate the reasoning bucket", () => {
    const oldModel = {
      tokens: 10,
      cost: 1,
      input: 10,
      output: 0,
      cacheRead: 0,
      cacheWrite: 0,
      messages: 1,
    } as ClientBreakdownData["models"][string];
    const oldDay = {
      tokens: 10,
      cost: 1,
      input: 10,
      output: 0,
      cacheRead: 0,
      cacheWrite: 0,
      reasoning: 0,
      messages: 1,
      models: { "model-a": oldModel },
    } satisfies ClientBreakdownData;
    const incoming = snapshot(
      contribution("2026-07-01", 10, "model-a", 1)
    );
    const first = baseline({ "2026-07-01": oldDay }, incoming);
    const replay = next(first.nextState!, incoming);

    expect(first.nextState?.aggregate.reasoning).toBe(0);
    expect(
      first.nextState?.days["2026-07-01"].models["model-a"].reasoning
    ).toBe(0);
    expectCreditedNothing(replay);
  });

  it("is idempotent when the same v2 full snapshot is replayed", () => {
    const first = baseline({}, snapshot(contribution("2026-07-01", 100)));
    const replay = next(
      first.nextState!,
      snapshot(contribution("2026-07-01", 100))
    );

    expectCreditedNothing(replay);
    expect(replay.nextState).toEqual(first.nextState);
  });

  it("does not add a later creation-to-shutdown date move with no aggregate growth", () => {
    const first = baseline(
      legacy(contribution("2026-07-01", 100)),
      snapshot(contribution("2026-07-01", 100))
    );
    const plan = next(
      first.nextState!,
      snapshot(contribution("2026-07-02", 100))
    );

    expect(plan.mode).toBe("incremental");
    expect(plan.increments).toEqual({});
  });

  it("does not spend unrelated new usage to delete or replace locally deleted history", () => {
    const first = baseline(
      legacy(contribution("2026-06-01", 100, "legacy-model", 5)),
      snapshot(contribution("2026-06-01", 100, "legacy-model", 5))
    );
    const plan = next(
      first.nextState!,
      snapshot(contribution("2026-07-02", 100, "different-model", 20))
    );

    // The full local snapshot lost 100 old tokens and gained 100 unrelated
    // tokens. Aggregate growth is zero, so neither model nor the higher cost
    // can be added and the stored legacy row is never touched.
    expect(plan.increments).toEqual({});
    expect(plan.nextState?.aggregate.tokens).toBe(100);
  });

  it("caps mixed deletion plus new work to net cumulative growth", () => {
    const first = baseline(
      legacy(contribution("2026-06-01", 100, "legacy-model", 5)),
      snapshot(contribution("2026-06-01", 100, "legacy-model", 5))
    );
    const plan = next(
      first.nextState!,
      snapshot(contribution("2026-07-02", 200, "different-model", 25))
    );
    const increment = plan.increments["2026-07-02"];

    expect(increment.tokens).toBe(100);
    expect(increment.input).toBe(100);
    expect(increment.cost).toBe(12.5);
    expect(increment.models["different-model"]).toMatchObject({
      tokens: 100,
      input: 100,
      cost: 12.5,
    });
  });

  it("allocates marginal rather than cumulative cost and message metadata", () => {
    const first = baseline(
      {},
      snapshot(contribution("2026-07-01", 100, "model-a", 1))
    );
    const plan = next(
      first.nextState!,
      snapshot(
        bucketContribution(
          "2026-07-01",
          { input: 100, output: 100, cacheRead: 0, cacheWrite: 0, reasoning: 0 },
          11,
          2
        )
      )
    );

    expect(plan.increments["2026-07-01"].models["model-a"]).toMatchObject({
      tokens: 100,
      output: 100,
      cost: 10,
      messages: 1,
    });
  });

  it("does not lower cost context when a snapshot temporarily loses history", () => {
    const first = baseline(
      {},
      snapshot(
        bucketContribution(
          "2026-07-01",
          { input: 100, output: 0, cacheRead: 0, cacheWrite: 0, reasoning: 0 },
          10,
          1
        )
      )
    );
    const truncated = next(
      first.nextState!,
      snapshot(
        bucketContribution(
          "2026-07-01",
          { input: 80, output: 30, cacheRead: 0, cacheWrite: 0, reasoning: 0 },
          2,
          1
        )
      )
    );
    const restored = next(
      truncated.nextState!,
      snapshot(
        bucketContribution(
          "2026-07-01",
          { input: 100, output: 40, cacheRead: 0, cacheWrite: 0, reasoning: 0 },
          13,
          1
        )
      )
    );

    expect(truncated.increments["2026-07-01"].cost).toBe(0);
    expect(truncated.nextState?.days["2026-07-01"].models["model-a"].cost).toBe(10);
    expect(restored.increments["2026-07-01"].cost).toBe(3);
  });

  it("retains a new message even when its token growth is a tiny fraction", () => {
    const first = baseline(
      {},
      snapshot(contribution("2026-07-01", 100, "model-a", 1))
    );
    const plan = next(
      first.nextState!,
      snapshot(
        bucketContribution(
          "2026-07-01",
          { input: 101, output: 0, cacheRead: 0, cacheWrite: 0, reasoning: 0 },
          1.1,
          2
        )
      )
    );

    expect(plan.increments["2026-07-01"].models["model-a"]).toMatchObject({
      input: 1,
      messages: 1,
      cost: 0.1,
    });
  });

  it("allocates growth independently across every token bucket", () => {
    const first = baseline(
      {},
      snapshot(
        bucketContribution(
          "2026-07-01",
          { input: 10, output: 20, cacheRead: 30, cacheWrite: 40, reasoning: 50 },
          1,
          1
        )
      )
    );
    const plan = next(
      first.nextState!,
      snapshot(
        bucketContribution(
          "2026-07-01",
          { input: 11, output: 22, cacheRead: 33, cacheWrite: 44, reasoning: 55 },
          2,
          2
        )
      )
    );

    expect(plan.increments["2026-07-01"].models["model-a"]).toMatchObject({
      input: 1,
      output: 2,
      cacheRead: 3,
      cacheWrite: 4,
      reasoning: 5,
      tokens: 15,
      cost: 1,
      messages: 1,
    });
  });

  it("does not fund a cache composition shift with unrelated bucket growth", () => {
    const first = baseline(
      {},
      snapshot(
        bucketContribution(
          "2026-07-01",
          { input: 20, output: 0, cacheRead: 80, cacheWrite: 0, reasoning: 0 },
          1,
          1
        )
      )
    );
    const plan = next(
      first.nextState!,
      snapshot(
        bucketContribution(
          "2026-07-01",
          { input: 0, output: 0, cacheRead: 90, cacheWrite: 20, reasoning: 0 },
          2,
          2
        )
      )
    );
    const increment = plan.increments["2026-07-01"].models["model-a"];

    expect(increment.cacheRead).toBe(0);
    expect(increment.cacheWrite).toBe(10);
    expect(increment.tokens).toBe(10);
  });

  it("tracks observed inclusive input separately from independent bucket maxima", () => {
    const first = baseline(
      {},
      snapshot(
        bucketContribution(
          "2026-07-01",
          { input: 100, output: 0, cacheRead: 0, cacheWrite: 0, reasoning: 0 },
          1,
          1
        )
      )
    );
    const shifted = next(
      first.nextState!,
      snapshot(
        bucketContribution(
          "2026-07-01",
          { input: 0, output: 0, cacheRead: 100, cacheWrite: 0, reasoning: 0 },
          1,
          1
        )
      )
    );
    const grown = next(
      shifted.nextState!,
      snapshot(
        bucketContribution(
          "2026-07-01",
          { input: 0, output: 0, cacheRead: 150, cacheWrite: 0, reasoning: 0 },
          2,
          2
        )
      )
    );

    const creditedModel =
      shifted.nextState?.days["2026-07-01"].models["model-a"];
    const observedModel =
      shifted.nextState?.observedDays?.["2026-07-01"].models["model-a"];
    expect(creditedModel?.input).toBe(100);
    expect(creditedModel?.cacheRead).toBe(0);
    expect(creditedModel?.inputIncludingCacheRead).toBe(100);
    expect(observedModel?.input).toBe(0);
    expect(observedModel?.cacheRead).toBe(100);
    expect(observedModel?.inputIncludingCacheRead).toBe(100);
    expect(grown.increments["2026-07-01"].models["model-a"].cacheRead).toBe(50);
  });

  it("credits same-day growth after re-attribution through a deterministic residual cell", () => {
    const stored = snapshot(
      bucketContribution(
        "2026-07-01",
        { input: 240, output: 0, cacheRead: 0, cacheWrite: 0, reasoning: 0 },
        24,
        12
      )
    );
    const reattributed = snapshot(
      bucketContribution(
        "2026-07-01",
        { input: 40, output: 0, cacheRead: 0, cacheWrite: 0, reasoning: 0 },
        4,
        2
      ),
      bucketContribution(
        "2026-07-02",
        { input: 100, output: 0, cacheRead: 0, cacheWrite: 0, reasoning: 0 },
        10,
        5
      ),
      bucketContribution(
        "2026-07-03",
        { input: 100, output: 0, cacheRead: 0, cacheWrite: 0, reasoning: 0 },
        10,
        5
      )
    );
    const first = baseline(stored, reattributed);
    const grownSnapshot = snapshot(
      bucketContribution(
        "2026-07-01",
        { input: 60, output: 0, cacheRead: 0, cacheWrite: 0, reasoning: 0 },
        6,
        3
      ),
      bucketContribution(
        "2026-07-02",
        { input: 100, output: 0, cacheRead: 0, cacheWrite: 0, reasoning: 0 },
        10,
        5
      ),
      bucketContribution(
        "2026-07-03",
        { input: 100, output: 0, cacheRead: 0, cacheWrite: 0, reasoning: 0 },
        10,
        5
      )
    );
    const grown = next(first.nextState!, grownSnapshot);
    const replay = next(grown.nextState!, grownSnapshot);

    expect(first.increments).toEqual({});
    expect(grown.increments["2026-07-01"]).toBeUndefined();
    expect(grown.increments["2026-07-03"].models["model-a"]).toMatchObject({
      input: 20,
      tokens: 20,
      cost: 2,
    });
    expect(
      Object.values(grown.increments).reduce(
        (sum, day) => sum + day.tokens,
        0
      )
    ).toBe(20);
    expect(grown.nextState?.aggregate.tokens).toBe(260);
    expect(
      Object.values(grown.nextState?.days ?? {}).reduce(
        (sum, day) => sum + day.tokens,
        0
      )
    ).toBe(260);
    expectCreditedNothing(replay);
  });

  it("recovers suppressed growth while migrating a legacy envelope state", () => {
    const stored = legacy(
      contribution("2026-07-01", 240, "model-a", 24)
    );
    const incoming = snapshot(
      contribution("2026-07-01", 60, "model-a", 6),
      contribution("2026-07-02", 100, "model-a", 10),
      contribution("2026-07-03", 100, "model-a", 10)
    );
    const legacyEnvelopeState: ParserClientHighWaterState = {
      version: 2,
      baselineEstablished: true,
      aggregate: {
        tokens: 260,
        input: 260,
        output: 0,
        cacheRead: 0,
        cacheWrite: 0,
        reasoning: 0,
        messages: 3,
        inputIncludingCacheRead: 260,
      },
      days: snapshot(
        contribution("2026-07-01", 240, "model-a", 24),
        contribution("2026-07-02", 100, "model-a", 10),
        contribution("2026-07-03", 100, "model-a", 10)
      ),
    };
    const migrated = planParserHighWaterSubmission({
      client: "copilot",
      incomingVersion: 2,
      fullHistory: true,
      existingLegacyDays: stored,
      incomingDays: incoming,
      state: legacyEnvelopeState,
    });
    const replay = next(migrated.nextState!, incoming);

    expect(migrated.mode).toBe("incremental");
    expect(migrated.nextState?.stateVersion).toBe(2);
    expect(
      Object.values(migrated.increments).reduce(
        (sum, day) => sum + day.tokens,
        0
      )
    ).toBe(20);
    expect(migrated.nextState?.aggregate.tokens).toBe(260);
    expect(migrated.nextState?.aggregate.messages).toBe(3);
    expectCreditedNothing(replay);
  });

  it("spends provable lifetime growth when bucket budgets are jointly infeasible", () => {
    const first = baseline(
      {},
      snapshot(contribution("2026-07-01", 5, "model-b", 0.5))
    );
    const incoming = snapshot(
      contribution("2026-07-01", 10, "model-a", 1),
      bucketContribution(
        "2026-07-01",
        { input: 0, output: 15, cacheRead: 0, cacheWrite: 0, reasoning: 0 },
        1.5,
        1,
        "model-b"
      )
    );
    const grown = next(first.nextState!, incoming);
    const replay = next(grown.nextState!, incoming);

    expect(grown.increments["2026-07-01"].tokens).toBe(20);
    expect(grown.increments["2026-07-01"].models["model-a"].input).toBe(10);
    expect(grown.increments["2026-07-01"].models["model-b"].output).toBe(10);
    expect(grown.nextState?.aggregate.tokens).toBe(25);
    expectCreditedNothing(replay);
  });

  it("prefers genuine observed growth before deterministic residual capacity", () => {
    const first = baseline(
      legacy(contribution("2026-07-01", 100, "old-model", 5)),
      snapshot(contribution("2026-07-03", 100, "moved-model", 5))
    );
    const incoming = snapshot(
      contribution("2026-07-03", 100, "moved-model", 5),
      contribution("2026-07-02", 50, "actual-new", 25)
    );
    const grown = next(first.nextState!, incoming);

    expect(grown.increments["2026-07-03"]).toBeUndefined();
    expect(grown.increments["2026-07-02"].models["actual-new"]).toMatchObject({
      input: 50,
      tokens: 50,
      cost: 25,
      messages: 1,
    });
  });

  it("derives exclusive-input growth from the inclusive delta after a cache shift", () => {
    const first = baseline(
      {},
      snapshot(
        bucketContribution(
          "2026-07-01",
          { input: 100, output: 0, cacheRead: 0, cacheWrite: 0, reasoning: 0 },
          1,
          1
        )
      )
    );
    const shifted = next(
      first.nextState!,
      snapshot(
        bucketContribution(
          "2026-07-01",
          { input: 0, output: 0, cacheRead: 100, cacheWrite: 0, reasoning: 0 },
          1,
          1
        )
      )
    );
    const grown = next(
      shifted.nextState!,
      snapshot(
        bucketContribution(
          "2026-07-01",
          { input: 50, output: 0, cacheRead: 100, cacheWrite: 0, reasoning: 0 },
          2,
          2
        )
      )
    );

    const increment = grown.increments["2026-07-01"].models["model-a"];
    expect(increment.input).toBe(50);
    expect(increment.cacheRead).toBe(0);
    expect(increment.tokens).toBe(50);
  });

  it("applies a partial lifetime budget to inclusive growth exactly once", () => {
    const first = baseline(
      {},
      snapshot(
        contribution("2026-07-02", 100, "model-a", 1),
        contribution("2026-07-01", 100, "deleted-model", 1)
      )
    );
    const grown = next(
      first.nextState!,
      snapshot(contribution("2026-07-02", 250, "model-a", 3))
    );
    const replay = next(
      grown.nextState!,
      snapshot(contribution("2026-07-02", 250, "model-a", 3))
    );

    expect(grown.increments["2026-07-02"].models["model-a"].input).toBe(50);
    expect(grown.increments["2026-07-02"].tokens).toBe(50);
    expectCreditedNothing(replay);
  });

  it("does not reserve cross-cell inclusive growth for an unsupported cache shift", () => {
    const first = baseline(
      {},
      snapshot(
        contribution("2026-07-01", 100, "model-a", 1),
        contribution("2026-07-01", 100, "model-b", 1)
      )
    );
    const plan = next(
      first.nextState!,
      snapshot(
        contribution("2026-07-01", 150, "model-a", 2),
        bucketContribution(
          "2026-07-01",
          { input: 50, output: 0, cacheRead: 50, cacheWrite: 0, reasoning: 0 },
          1,
          1,
          "model-b"
        )
      )
    );

    expect(plan.increments["2026-07-01"].models["model-a"].input).toBe(50);
    expect(plan.increments["2026-07-01"].models["model-b"]).toBeUndefined();
  });

  it("spends a partial shared inclusive budget deterministically and once", () => {
    const first = baseline(
      {},
      snapshot(
        contribution("2026-07-01", 100, "z-growing", 1),
        contribution("2026-07-01", 100, "a-cache-shift", 1),
        contribution("2026-07-01", 50, "deleted-model", 1)
      )
    );
    const grown = next(
      first.nextState!,
      snapshot(
        bucketContribution(
          "2026-07-01",
          { input: 50, output: 0, cacheRead: 50, cacheWrite: 0, reasoning: 0 },
          1,
          1,
          "a-cache-shift"
        ),
        contribution("2026-07-01", 200, "z-growing", 2)
      )
    );
    const replay = next(
      grown.nextState!,
      snapshot(
        bucketContribution(
          "2026-07-01",
          { input: 50, output: 0, cacheRead: 50, cacheWrite: 0, reasoning: 0 },
          1,
          1,
          "a-cache-shift"
        ),
        contribution("2026-07-01", 200, "z-growing", 2)
      )
    );

    expect(grown.increments["2026-07-01"].models["a-cache-shift"]).toBeUndefined();
    expect(grown.increments["2026-07-01"].models["z-growing"].input).toBe(50);
    expect(grown.increments["2026-07-01"].tokens).toBe(50);
    expectCreditedNothing(replay);
  });

  it("caps cell-supported cache moves by aggregate cache growth", () => {
    const first = baseline(
      {},
      snapshot(
        contribution("2026-07-01", 100, "a-cache-move", 1),
        contribution("2026-07-01", 100, "z-input-growth", 1),
        bucketContribution(
          "2026-07-01",
          { input: 0, output: 0, cacheRead: 50, cacheWrite: 0, reasoning: 0 },
          1,
          1,
          "gone-cache"
        )
      )
    );
    const grown = next(
      first.nextState!,
      snapshot(
        bucketContribution(
          "2026-07-01",
          { input: 100, output: 0, cacheRead: 50, cacheWrite: 0, reasoning: 0 },
          2,
          2,
          "a-cache-move"
        ),
        contribution("2026-07-01", 150, "z-input-growth", 2)
      )
    );
    const replay = next(
      grown.nextState!,
      snapshot(
        bucketContribution(
          "2026-07-01",
          { input: 100, output: 0, cacheRead: 50, cacheWrite: 0, reasoning: 0 },
          2,
          2,
          "a-cache-move"
        ),
        contribution("2026-07-01", 150, "z-input-growth", 2)
      )
    );

    expect(grown.increments["2026-07-01"].models["a-cache-move"]).toBeUndefined();
    expect(grown.increments["2026-07-01"].models["z-input-growth"].input).toBe(50);
    expect(grown.increments["2026-07-01"].tokens).toBe(50);
    expectCreditedNothing(replay);
  });

  it("allows genuine fully-cached inclusive-input growth", () => {
    const first = baseline(
      {},
      snapshot(
        bucketContribution(
          "2026-07-01",
          { input: 0, output: 0, cacheRead: 100, cacheWrite: 0, reasoning: 0 },
          1,
          1
        )
      )
    );
    const plan = next(
      first.nextState!,
      snapshot(
        bucketContribution(
          "2026-07-01",
          { input: 0, output: 0, cacheRead: 150, cacheWrite: 0, reasoning: 0 },
          2,
          2
        )
      )
    );

    expect(plan.increments["2026-07-01"].models["model-a"].cacheRead).toBe(50);
  });

  it("does not treat repricing without token growth as new spend", () => {
    const first = baseline(
      legacy(contribution("2026-07-01", 100, "model-a", 5)),
      snapshot(contribution("2026-07-01", 100, "model-a", 5))
    );
    const repriced = next(
      first.nextState!,
      snapshot(contribution("2026-07-01", 100, "model-a", 50))
    );

    expect(repriced.increments).toEqual({});
  });

  it("allocates a bounded mixed move plus growth to the newest positive cell", () => {
    const first = baseline(
      legacy(contribution("2026-07-01", 100, "model-a", 10)),
      snapshot(contribution("2026-07-01", 100, "model-a", 10))
    );
    const plan = next(
      first.nextState!,
      snapshot(
        contribution("2026-07-02", 100, "model-a", 10),
        contribution("2026-07-03", 50, "model-b", 8)
      )
    );

    expect(plan.increments["2026-07-02"]).toBeUndefined();
    expect(plan.increments["2026-07-03"].tokens).toBe(50);
    expect(plan.increments["2026-07-03"].cost).toBe(8);
  });

  it("does not count a pure model rename as new work", () => {
    const first = baseline(
      legacy(contribution("2026-07-01", 100, "old-name", 10)),
      snapshot(contribution("2026-07-01", 100, "old-name", 10))
    );
    const plan = next(
      first.nextState!,
      snapshot(contribution("2026-07-01", 100, "new-name", 10))
    );

    expect(plan.increments).toEqual({});
  });

  it("quantizes partially authorized marginal cost coherently", () => {
    const first = baseline(
      {},
      snapshot(contribution("2026-07-01", 2, "old-model", 1))
    );
    const plan = next(
      first.nextState!,
      snapshot(contribution("2026-07-02", 3, "model-a", 1))
    );
    const increment = plan.increments["2026-07-02"];
    const stored = addClientBreakdownIncrement(undefined, increment);
    const totals = recalculateDayTotals({ copilot: stored });

    expect(increment.models["model-a"].cost).toBe(0.3333);
    expect(increment.cost).toBe(0.3333);
    expect(stored.models["model-a"].cost).toBe(0.3333);
    expect(stored.cost).toBe(0.3333);
    expect(totals.cost.toFixed(4)).toBe("0.3333");
  });

  it("does not mutate an existing breakdown while merging an increment", () => {
    const existing = snapshot(
      contribution("2026-07-01", 100, "model-a", 1.23456)
    )["2026-07-01"];
    existing.cost = 1.23456;
    existing.models["model-a"].cost = 1.23456;
    const before = structuredClone(existing);
    const increment = snapshot(
      contribution("2026-07-01", 1, "model-a", 0.1)
    )["2026-07-01"];

    addClientBreakdownIncrement(existing, increment);

    expect(existing).toEqual(before);
    expect(existing.models["model-a"].cost).toBe(1.23456);
  });

  it("adds only post-baseline cumulative growth and keeps representations coherent", () => {
    const first = baseline(
      legacy(contribution("2026-07-01", 100, "model-a", 10)),
      snapshot(contribution("2026-07-01", 100, "model-a", 10))
    );
    const plan = next(
      first.nextState!,
      snapshot(contribution("2026-07-01", 100, "model-a", 10), contribution("2026-07-02", 50, "model-b", 7))
    );
    const increment = plan.increments["2026-07-02"];
    const stored = addClientBreakdownIncrement(undefined, increment);
    const totals = recalculateDayTotals({ copilot: stored });

    expect(stored.tokens).toBe(50);
    expect(stored.cost).toBe(7);
    expect(stored.models["model-b"].tokens).toBe(50);
    expect(totals).toMatchObject({ tokens: 50, cost: 7, inputTokens: 50 });
  });

  it("anchors a missing first v2 history snapshot to the larger stored legacy total", () => {
    const first = baseline(
      legacy(contribution("2026-06-01", 100, "model-a", 10)),
      {}
    );
    const restored = next(
      first.nextState!,
      snapshot(contribution("2026-06-01", 100, "model-a", 10))
    );

    expect(first.nextState?.aggregate.tokens).toBe(100);
    expect(restored.increments).toEqual({});
  });

  it("keeps parser identity on partial scans but freezes their unsafe changes after transition", () => {
    const first = baseline({}, snapshot(contribution("2026-07-01", 100)));
    const partial = planParserHighWaterSubmission({
      client: "copilot",
      incomingVersion: 2,
      fullHistory: false,
      existingLegacyDays: {},
      incomingDays: snapshot(contribution("2026-07-02", 100)),
      state: first.nextState,
    });

    expect(partial.mode).toBe("freeze");
    expect(partial.increments).toEqual({});
  });

  it("freezes a partial v2 scan when legacy Copilot history already exists", () => {
    const plan = planParserHighWaterSubmission({
      client: "copilot",
      incomingVersion: 2,
      fullHistory: false,
      existingLegacyDays: legacy(contribution("2026-07-01", 100)),
      incomingDays: snapshot(contribution("2026-07-02", 100)),
    });

    expect(plan.mode).toBe("freeze");
    expect(plan.nextState?.baselineEstablished).toBe(false);

    const oldAfterIdentity = planParserHighWaterSubmission({
      client: "copilot",
      fullHistory: false,
      existingLegacyDays: legacy(contribution("2026-07-01", 100)),
      incomingDays: snapshot(contribution("2026-07-03", 200)),
      state: plan.nextState,
    });
    expect(oldAfterIdentity.mode).toBe("freeze");

    const fullBaseline = planParserHighWaterSubmission({
      client: "copilot",
      incomingVersion: 2,
      fullHistory: true,
      existingLegacyDays: legacy(contribution("2026-07-01", 100)),
      incomingDays: snapshot(contribution("2026-07-01", 100)),
      state: plan.nextState,
    });
    expect(fullBaseline.nextState?.baselineEstablished).toBe(true);
  });

  it("keeps old CLI status quo before transition and freezes it afterward", () => {
    const before = planParserHighWaterSubmission({
      client: "copilot",
      fullHistory: false,
      existingLegacyDays: {},
      incomingDays: snapshot(contribution("2026-07-01", 100)),
    });
    const first = baseline({}, snapshot(contribution("2026-07-01", 100)));
    const after = planParserHighWaterSubmission({
      client: "copilot",
      fullHistory: false,
      existingLegacyDays: {},
      incomingDays: snapshot(contribution("2026-07-02", 100)),
      state: first.nextState,
    });

    expect(before.mode).toBe("status-quo");
    expect(after.mode).toBe("freeze");
  });

  it.each([1, 3, 999])("freezes lower or unsupported Copilot generation %s", (version) => {
    const plan = planParserHighWaterSubmission({
      client: "copilot",
      incomingVersion: version,
      fullHistory: true,
      existingLegacyDays: legacy(contribution("2026-07-01", 100)),
      incomingDays: snapshot(contribution("2026-07-02", 200)),
    });

    expect(plan.mode).toBe("freeze");
    expect(plan.nextState).toBeUndefined();
  });

  it("never credits more than lifetime high-water growth across moves, deletions, and replays", () => {
    const first = baseline(
      legacy(contribution("2026-07-01", 100, "model-a", 10)),
      snapshot(contribution("2026-07-01", 100, "model-a", 10))
    );
    let state = first.nextState!;
    let credited = 0;
    let lifetimePeak = 100;
    let seed = 0x1032;

    for (let index = 0; index < 100; index += 1) {
      seed = (seed * 1664525 + 1013904223) >>> 0;
      const reportedTotal = seed % 260;
      const date = `2026-07-${String((seed % 4) + 1).padStart(2, "0")}`;
      const modelId = seed % 3 === 0 ? "renamed-model" : "model-a";
      const plan = next(
        state,
        reportedTotal === 0
          ? {}
          : snapshot(contribution(date, reportedTotal, modelId, reportedTotal / 7))
      );
      const added = Object.values(plan.increments).reduce(
        (sum, day) => sum + day.tokens,
        0
      );
      credited += added;
      lifetimePeak = Math.max(lifetimePeak, reportedTotal);

      expect(added).toBeGreaterThanOrEqual(0);
      expect(credited).toBeLessThanOrEqual(lifetimePeak - 100);
      state = plan.nextState!;
    }
  });

  it("conserves every provable token and message across cells and buckets", () => {
    const first = baseline(
      {},
      snapshot(
        bucketContribution(
          "2026-07-01",
          { input: 50, output: 0, cacheRead: 0, cacheWrite: 0, reasoning: 0 },
          5,
          2,
          "model-a"
        ),
        bucketContribution(
          "2026-07-02",
          { input: 0, output: 50, cacheRead: 0, cacheWrite: 0, reasoning: 0 },
          5,
          3,
          "model-b"
        )
      )
    );
    let state = first.nextState!;
    let seed = 0x1206;

    for (let index = 0; index < 40; index += 1) {
      const rows: ReturnType<typeof bucketContribution>[] = [];
      for (let cell = 0; cell < 4; cell += 1) {
        seed = (seed * 1664525 + 1013904223) >>> 0;
        const input = seed % 80;
        seed = (seed * 1664525 + 1013904223) >>> 0;
        const output = seed % 80;
        const tokens = input + output;
        rows.push(
          bucketContribution(
            cell < 2 ? "2026-07-01" : "2026-07-02",
            { input, output, cacheRead: 0, cacheWrite: 0, reasoning: 0 },
            tokens / 10,
            Math.floor(tokens / 20),
            cell % 2 === 0 ? "model-a" : "model-b"
          )
        );
      }
      const incoming = snapshot(...rows);
      const incomingTokens = Object.values(incoming).reduce(
        (sum, day) => sum + day.tokens,
        0
      );
      const incomingMessages = Object.values(incoming).reduce(
        (sum, day) => sum + day.messages,
        0
      );
      const previousTokens = state.aggregate.tokens;
      const previousMessages = state.aggregate.messages;
      const plan = next(state, incoming);
      const addedTokens = Object.values(plan.increments).reduce(
        (sum, day) => sum + day.tokens,
        0
      );
      const addedMessages = Object.values(plan.increments).reduce(
        (sum, day) => sum + day.messages,
        0
      );

      expect(addedTokens).toBe(Math.max(0, incomingTokens - previousTokens));
      expect(addedMessages).toBe(
        Math.max(0, incomingMessages - previousMessages)
      );
      expect(plan.nextState?.aggregate.tokens).toBe(
        previousTokens + addedTokens
      );
      expect(plan.nextState?.aggregate.messages).toBe(
        previousMessages + addedMessages
      );
      expect(
        Object.values(plan.nextState?.days ?? {}).reduce(
          (sum, day) => sum + day.tokens,
          0
        )
      ).toBe(plan.nextState?.aggregate.tokens);
      expect(next(plan.nextState!, incoming).increments).toEqual({});
      state = plan.nextState!;
    }
  });

  it("fails closed when an accepted generation marker has lost its high-water", () => {
    const plan = planParserHighWaterSubmission({
      client: "copilot",
      incomingVersion: 2,
      fullHistory: true,
      existingLegacyDays: {},
      incomingDays: snapshot(contribution("2026-07-01", 100)),
      persistedVersion: 2,
    });

    expect(plan.mode).toBe("freeze");
    expect(plan.nextState).toBeUndefined();
  });

  it("fails closed for an unknown allocation-state schema", () => {
    const first = baseline({}, snapshot(contribution("2026-07-01", 100)));
    const futureState = {
      ...first.nextState!,
      stateVersion: 999,
    };
    const plan = next(
      futureState,
      snapshot(contribution("2026-07-01", 150))
    );

    expect(plan.mode).toBe("freeze");
    expect(plan.increments).toEqual({});
    expect(plan.nextState).toBeUndefined();
  });

  it.each([
    { stateVersion: 999, version: 2 },
    { stateVersion: 2, version: 1 },
  ])(
    "does not re-baseline an invalid pending state ($stateVersion/$version)",
    ({ stateVersion, version }) => {
      const pending = planParserHighWaterSubmission({
        client: "copilot",
        incomingVersion: 2,
        fullHistory: false,
        existingLegacyDays: {},
        incomingDays: {},
      }).nextState!;
      const plan = next(
        { ...pending, stateVersion, version },
        snapshot(contribution("2026-07-01", 100))
      );

      expect(plan.mode).toBe("freeze");
      expect(plan.increments).toEqual({});
      expect(plan.nextState).toBeUndefined();
    }
  );

  it("fails closed when a v2 aggregate diverges from its credited cells", () => {
    const first = baseline({}, snapshot(contribution("2026-07-01", 100)));
    const inconsistentState = {
      ...first.nextState!,
      aggregate: {
        ...first.nextState!.aggregate,
        tokens: 120,
        input: 120,
        inputIncludingCacheRead: 120,
      },
    };
    const plan = next(
      inconsistentState,
      snapshot(contribution("2026-07-01", 150))
    );

    expect(plan.mode).toBe("freeze");
    expect(plan.increments).toEqual({});
    expect(plan.nextState).toBeUndefined();
  });

  it("allowlists the exact supported client as well as its generation", () => {
    const plan = planParserHighWaterSubmission({
      client: "codex",
      incomingVersion: 2,
      fullHistory: true,
      existingLegacyDays: {},
      incomingDays: {},
    });

    expect(plan.mode).toBe("freeze");
    expect(plan.nextState).toBeUndefined();
  });

  it("accepts a supported v2 snapshot normally for a brand-new device/client", () => {
    const plan = baseline({}, snapshot(contribution("2026-07-01", 100)));

    expect(plan.mode).toBe("baseline-new");
    expect(plan.nextState?.aggregate.tokens).toBe(100);
  });
});

describe("droid parser high-water", () => {
  function droidContribution(date: string, tokens: number, modelId = "kimi-k3-0") {
    return {
      date,
      clients: [
        {
          client: "droid",
          modelId,
          tokens: {
            input: tokens,
            output: 0,
            cacheRead: 0,
            cacheWrite: 0,
            reasoning: 0,
          },
          cost: 0,
          messages: 1,
        },
      ],
    };
  }

  function droidSnapshot(...rows: ReturnType<typeof droidContribution>[]) {
    return foldParserClientSnapshot(rows, "droid");
  }

  function droidPlan(
    existingLegacyDays: Record<string, ClientBreakdownData>,
    incomingDays: Record<string, ClientBreakdownData>,
    state?: ParserClientHighWaterState
  ) {
    return planParserHighWaterSubmission({
      client: "droid",
      incomingVersion: SUPPORTED_VERSIONED_PARSERS.droid,
      fullHistory: true,
      existingLegacyDays,
      incomingDays,
      state,
    });
  }

  // One 321,000-token session that ran across three days. The old parser put
  // the whole session on one day; the transcript-weighted parser splits it
  // across the days it actually ran. The lifetime total is identical.
  const wholeSessionOnOneDay = droidSnapshot(droidContribution("2026-08-07", 321_000));
  const sessionSplitAcrossDays = droidSnapshot(
    droidContribution("2026-08-07", 21_000),
    droidContribution("2026-08-08", 152_000),
    droidContribution("2026-08-09", 148_000)
  );

  it("is registered at the generation every CLI already declares", () => {
    // The two Droid shapes disagree about which day a token lands on, never
    // about the lifetime total, so no generation has to be frozen out. A
    // registered version above 1 would instead freeze every installed CLI.
    expect(SUPPORTED_VERSIONED_PARSERS.droid).toBe(1);
  });

  it("shows why the day-by-day merge alone inflates a re-attributed session", () => {
    // Without the high-water this is what the submit route stores: the merge
    // guard refuses the decrease on 2026-08-07 and accepts the two days that
    // rose, so the device's stored total gains everything that moved.
    let stored = 0;
    for (const date of Object.keys(sessionSplitAcrossDays)) {
      const merged = mergeClientBreakdownsWithRegressionGuard(
        wholeSessionOnOneDay[date] ? { droid: wholeSessionOnOneDay[date] } : {},
        { droid: sessionSplitAcrossDays[date] },
        new Set(["droid"]),
        undefined,
        true
      );
      stored += merged.merged.droid?.tokens ?? 0;
    }

    expect(stored).toBe(621_000);
  });

  it("moves a re-attributed session onto the days the current parser reports", () => {
    const plan = droidPlan(wholeSessionOnOneDay, sessionSplitAcrossDays);

    expect(plan.mode).toBe("replace");
    expect(plan.increments).toEqual({});
    expect(plan.layoutDays?.["2026-08-07"]?.tokens).toBe(21_000);
    expect(plan.layoutDays?.["2026-08-08"]?.tokens).toBe(152_000);
    expect(plan.layoutDays?.["2026-08-09"]?.tokens).toBe(148_000);
    expect(plan.nextState?.aggregate.tokens).toBe(321_000);
  });

  it("still credits real usage that arrives after the re-attribution", () => {
    const state = droidPlan(wholeSessionOnOneDay, sessionSplitAcrossDays).nextState!;
    const grown = droidSnapshot(
      droidContribution("2026-08-07", 21_000),
      droidContribution("2026-08-08", 152_000),
      droidContribution("2026-08-09", 148_000),
      droidContribution("2026-08-10", 79_000)
    );

    const plan = droidPlan({}, grown, state);

    expect(plan.mode).toBe("replace");
    expect(plan.layoutDays?.["2026-08-10"]?.tokens).toBe(79_000);
    expect(plan.nextState?.aggregate.tokens).toBe(400_000);
  });

  it("does not let a date-filtered rescan advance the high-water", () => {
    const state = droidPlan(wholeSessionOnOneDay, sessionSplitAcrossDays).nextState!;

    const plan = planParserHighWaterSubmission({
      client: "droid",
      incomingVersion: SUPPORTED_VERSIONED_PARSERS.droid,
      fullHistory: false,
      existingLegacyDays: {},
      incomingDays: droidSnapshot(droidContribution("2026-08-10", 500_000)),
      state,
    });

    expect(plan.mode).toBe("freeze");
    expect(plan.increments).toEqual({});
  });
});

describe("devin-cli recount proof", () => {
  function devinContribution(
    date: string,
    tokens: number,
    modelId = "swe-2-high",
    cost = 0,
    messages = 1
  ) {
    return {
      date,
      clients: [
        {
          client: "devin-cli",
          modelId,
          tokens: {
            input: tokens,
            output: 0,
            cacheRead: 0,
            cacheWrite: 0,
            reasoning: 0,
          },
          cost,
          messages,
        },
      ],
    };
  }

  function devinSnapshot(...rows: ReturnType<typeof devinContribution>[]) {
    return foldParserClientSnapshot(rows, "devin-cli");
  }

  function proofCell(tokens: number, cost = 0, messages = 1): RecountProofCell {
    return {
      tokens: { input: tokens, output: 0, cacheRead: 0, cacheWrite: 0, reasoning: 0 },
      cost,
      messages,
    };
  }

  /** `date -> { previous, corrected }` for the single `swe-2-high` model. */
  function proof(
    days: Record<
      string,
      { previous: RecountProofCell; corrected: RecountProofCell; modelId?: string }
    >
  ): ClientRecountProof {
    return Object.fromEntries(
      Object.entries(days).map(([date, entry]) => [
        date,
        { [entry.modelId ?? "swe-2-high"]: { previous: entry.previous, corrected: entry.corrected } },
      ])
    );
  }

  /** What a snapshot submits for one proof day when its cost was safe: the corrected side. */
  function submittedFor(proofDay: Record<string, RecountProofEntry>): ClientBreakdownData {
    return foldParserClientSnapshot(
      [
        {
          date: "2026-10-08",
          clients: Object.entries(proofDay).map(([modelId, entry]) => ({
            client: "devin-cli",
            modelId,
            tokens: { ...entry.corrected.tokens },
            cost: entry.corrected.cost,
            messages: entry.corrected.messages,
          })),
        },
      ],
      "devin-cli"
    )["2026-10-08"];
  }

  function devinPlan(
    existingLegacyDays: Record<string, ClientBreakdownData>,
    incomingDays: Record<string, ClientBreakdownData>,
    options: { state?: ParserClientHighWaterState; recountProof?: ClientRecountProof } = {}
  ) {
    return planParserHighWaterSubmission({
      client: "devin-cli",
      incomingVersion: SUPPORTED_VERSIONED_PARSERS["devin-cli"],
      fullHistory: true,
      existingLegacyDays,
      incomingDays,
      state: options.state,
      recountProof: options.recountProof,
    });
  }

  // The row_id-dedup generation counted each persisted copy of a Devin
  // assistant response; the request_id-dedup generation counts each API
  // request once, so the corrected snapshot lands strictly BELOW the stored
  // cells. The proof says, per day, what both generations count from the
  // same rows.
  const inflatedStored = devinSnapshot(
    devinContribution("2026-10-07", 514_000_000),
    devinContribution("2026-10-08", 1_000_000_000)
  );
  const correctedScan = devinSnapshot(
    devinContribution("2026-10-07", 234_000_000),
    devinContribution("2026-10-08", 455_000_000)
  );
  const fullProof = proof({
    "2026-10-07": { previous: proofCell(514_000_000), corrected: proofCell(234_000_000) },
    "2026-10-08": { previous: proofCell(1_000_000_000), corrected: proofCell(455_000_000) },
  });

  it("is registered at the generation the deduplicating CLI declares", () => {
    // Generation 2 is the request_id recount. A registered version of 1 would
    // accept proofs from a generation that never computes them; a version
    // above 2 would freeze the only CLI that can heal the stored rows.
    expect(SUPPORTED_VERSIONED_PARSERS["devin-cli"]).toBe(2);
  });

  it("shows why the merge guard alone can never heal the inflated cells", () => {
    // Without a proof this is what the submit route stores: the same-day
    // regression guard preserves the higher stored value on every covered
    // day, so the inflation is locked in forever.
    let stored = 0;
    for (const date of Object.keys(correctedScan)) {
      const merged = mergeClientBreakdownsWithRegressionGuard(
        inflatedStored[date] ? { "devin-cli": inflatedStored[date] } : {},
        { "devin-cli": correctedScan[date] },
        new Set(["devin-cli"]),
        undefined,
        true
      );
      stored += merged.merged["devin-cli"]?.tokens ?? 0;
    }

    expect(stored).toBe(1_514_000_000);
  });

  it("does not lower a stored cell on a lower snapshot alone", () => {
    // A lower total is not evidence: a database that failed to open or
    // history deleted locally lowers it just the same. Without a proof the
    // transition is the ordinary preserving baseline.
    const plan = devinPlan(inflatedStored, correctedScan);

    expect(plan.mode).toBe("baseline-legacy");
    expect(plan.recountDays).toBeUndefined();
    expect(plan.increments).toEqual({});
    expect(plan.nextState?.days["2026-10-08"]?.tokens).toBe(1_000_000_000);
    expect(plan.nextState?.aggregate.tokens).toBe(1_514_000_000);
  });

  it("subtracts exactly the proven over-count from covered legacy cells", () => {
    const plan = devinPlan(inflatedStored, correctedScan, { recountProof: fullProof });

    expect(plan.mode).toBe("baseline-legacy");
    expect(plan.recountDays).toEqual(["2026-10-07", "2026-10-08"]);
    expect(plan.rejectedRecountDays).toBeUndefined();
    expect(plan.increments).toEqual({});
    expect(plan.nextState?.days["2026-10-07"]?.tokens).toBe(234_000_000);
    expect(plan.nextState?.days["2026-10-08"]?.tokens).toBe(455_000_000);
    expect(plan.nextState?.days["2026-10-08"]?.models["swe-2-high"].tokens).toBe(
      455_000_000
    );
    expect(plan.nextState?.aggregate.tokens).toBe(689_000_000);
  });

  it("keeps stored usage the proof does not describe", () => {
    // Two local databases fed the stored day (A = 400 with 50 duplicate
    // rows, B = 600). This scan read only A: B contributes to neither side of
    // the proof, which explains only 400 of the stored 1,000. Replacing the
    // cell with the 350 the scan reports would delete B outright, and taking
    // A's 50 off would be repeated by every later submit of the same proof.
    const stored = devinSnapshot(devinContribution("2026-10-08", 1_000));
    const scan = devinSnapshot(devinContribution("2026-10-08", 350));
    const plan = devinPlan(stored, scan, {
      recountProof: proof({
        "2026-10-08": { previous: proofCell(400), corrected: proofCell(350) },
      }),
    });

    expect(plan.mode).toBe("baseline-legacy");
    expect(plan.recountDays).toEqual(["2026-10-08"]);
    expect(plan.nextState?.days["2026-10-08"]?.tokens).toBe(1_000);
    expect(plan.nextState?.aggregate.tokens).toBe(1_000);
  });

  it("does not wear unexplained usage down when the same proof is replayed", () => {
    // Stored 1,000, but the proof's rows account for only 500 of it (100
    // after dedup). Subtracting the 400 on every submit would give 600, 200,
    // then 100, removing usage the proof never described.
    const stored = devinSnapshot(devinContribution("2026-10-08", 1_000));
    const scan = devinSnapshot(devinContribution("2026-10-08", 100));
    const partialProof = proof({
      "2026-10-08": { previous: proofCell(500), corrected: proofCell(100) },
    });
    let state = devinPlan(stored, scan, { recountProof: partialProof }).nextState!;
    expect(state.days["2026-10-08"]?.tokens).toBe(1_000);

    for (let replay = 0; replay < 3; replay += 1) {
      const plan = devinPlan({}, scan, { state, recountProof: partialProof });
      expect(plan.mode).toBe("incremental");
      expect(plan.nextState?.days["2026-10-08"]?.tokens).toBe(1_000);
      state = plan.nextState!;
    }
  });

  it("keeps a day the transition could not correct when the missing source is read again", () => {
    // Continuing from the A-only scan above (ledger kept at 1,000): B becomes
    // readable and carries 60 duplicates of its own. The proof now explains
    // the whole stored cell, but the device is past its transition, so a
    // proof no longer lowers counts and the day stays as stored.
    const stored = devinSnapshot(devinContribution("2026-10-08", 1_000));
    const first = devinPlan(stored, devinSnapshot(devinContribution("2026-10-08", 350)), {
      recountProof: proof({
        "2026-10-08": { previous: proofCell(400), corrected: proofCell(350) },
      }),
    });
    expect(first.nextState?.days["2026-10-08"]?.tokens).toBe(1_000);

    const bothSources = proof({
      "2026-10-08": { previous: proofCell(1_000), corrected: proofCell(890) },
    });
    const second = devinPlan({}, devinSnapshot(devinContribution("2026-10-08", 890)), {
      state: first.nextState,
      recountProof: bothSources,
    });
    expect(second.mode).toBe("incremental");
    expect(second.recountDays).toEqual(["2026-10-08"]);
    expect(second.recountTransition).toBeUndefined();
    expect(second.nextState?.days["2026-10-08"]?.tokens).toBe(1_000);
    expect(second.nextState?.aggregate.tokens).toBe(1_000);

    // A transition that reads both sources corrects the day then.
    const transition = devinPlan(stored, devinSnapshot(devinContribution("2026-10-08", 890)), {
      recountProof: bothSources,
    });
    expect(transition.recountTransition).toBe(true);
    expect(transition.nextState?.days["2026-10-08"]?.tokens).toBe(890);
  });

  it("keeps a corrected day when one of its sources later disappears", () => {
    // Databases A and B fed the stored day: generation 1 counted 200 from
    // each and generation 2 counts 100 from each. The first full snapshot
    // reads both and corrects the day to 200.
    const stored = devinSnapshot(devinContribution("2026-10-08", 400, "swe-2-high", 0, 4));
    const first = devinPlan(
      stored,
      devinSnapshot(devinContribution("2026-10-08", 200, "swe-2-high", 0, 2)),
      {
        recountProof: proof({
          "2026-10-08": { previous: proofCell(400, 0, 4), corrected: proofCell(200, 0, 2) },
        }),
      }
    );
    expect(first.nextState?.days["2026-10-08"]?.tokens).toBe(200);

    // B is then deleted. A's proof still counts A's duplicates under
    // generation 1, and its previous side (200 tokens, 2 messages) equals
    // the corrected day exactly. The day is in generation-2 units, though,
    // and B's 100 must stay.
    const afterDelete = devinPlan(
      {},
      devinSnapshot(devinContribution("2026-10-08", 100, "swe-2-high", 0, 1)),
      {
        state: first.nextState,
        recountProof: proof({
          "2026-10-08": { previous: proofCell(200, 0, 2), corrected: proofCell(100, 0, 1) },
        }),
      }
    );
    expect(afterDelete.recountTransition).toBeUndefined();
    expect(afterDelete.nextState?.days["2026-10-08"]?.tokens).toBe(200);
    expect(afterDelete.nextState?.days["2026-10-08"]?.messages).toBe(2);
  });

  it("never lowers a day first credited by this generation when a source disappears", () => {
    // The day was never stored under generation 1: this generation credited
    // 200 from A and B. A's later proof explains only A.
    const state = devinPlan(
      {},
      devinSnapshot(devinContribution("2026-10-08", 200, "swe-2-high", 0, 2))
    ).nextState!;
    const afterDelete = devinPlan(
      {},
      devinSnapshot(devinContribution("2026-10-08", 100, "swe-2-high", 0, 1)),
      {
        state,
        recountProof: proof({
          "2026-10-08": { previous: proofCell(500, 0, 2), corrected: proofCell(100, 0, 1) },
        }),
      }
    );
    expect(afterDelete.nextState?.days["2026-10-08"]?.tokens).toBe(200);
  });

  it("keeps a legacy day whose remaining source has since outgrown it", () => {
    // The stored 400 came from A (200) and B (200) under generation 1. B is
    // gone by the first generation-2 scan and A has grown to 500. A's proof
    // counts more than the stored day, yet none of B's share is in it, so
    // the day is kept rather than lowered to A's corrected 150. (A day that
    // only grew from one source looks the same and stays inflated too.)
    const stored = devinSnapshot(devinContribution("2026-10-08", 400));
    const plan = devinPlan(stored, devinSnapshot(devinContribution("2026-10-08", 150)), {
      recountProof: proof({
        "2026-10-08": { previous: proofCell(500), corrected: proofCell(150) },
      }),
    });
    expect(plan.recountDays).toEqual(["2026-10-08"]);
    expect(plan.nextState?.days["2026-10-08"]?.tokens).toBe(400);
  });

  it("never lowers a cell below the corrected count and lets growth raise it", () => {
    // Stored 1,000 = 900 real + 100 duplicate rows. Since then 180 new tokens
    // arrived whose rows carry 20 duplicates of their own. The proven
    // over-count is 120, but the stored cell is already below the corrected
    // 1,080, so nothing is subtracted and the growth allocation credits the
    // 80 the stored cell lacks.
    const stored = devinSnapshot(devinContribution("2026-10-08", 1_000));
    const scan = devinSnapshot(devinContribution("2026-10-08", 1_080));
    const plan = devinPlan(stored, scan, {
      recountProof: proof({
        "2026-10-08": { previous: proofCell(1_200), corrected: proofCell(1_080) },
      }),
    });

    expect(plan.mode).toBe("baseline-legacy");
    expect(plan.increments["2026-10-08"]?.tokens).toBe(80);
    expect(plan.nextState?.days["2026-10-08"]?.tokens).toBe(1_080);
  });

  it("is idempotent when the same corrected snapshot is replayed", () => {
    const state = devinPlan(inflatedStored, correctedScan, {
      recountProof: fullProof,
    }).nextState!;

    const replay = devinPlan({}, correctedScan, { state, recountProof: fullProof });

    expect(replay.mode).toBe("incremental");
    expect(replay.increments).toEqual({});
    expect(replay.highWaterDeficit).toBeUndefined();
    expect(replay.nextState?.aggregate.tokens).toBe(689_000_000);
    expect(replay.nextState?.days["2026-10-08"]?.tokens).toBe(455_000_000);
  });

  it("keeps the guard closed for a lower snapshot without a matching proof", () => {
    const state = devinPlan(inflatedStored, correctedScan, {
      recountProof: fullProof,
    }).nextState!;

    // A proof for a smaller snapshot cannot lower the day below what it
    // itself calls corrected, and the ledger is already there.
    const lower = devinPlan(
      {},
      devinSnapshot(devinContribution("2026-10-08", 100_000_000)),
      {
        state,
        recountProof: proof({
          "2026-10-08": {
            previous: proofCell(100_000_000),
            corrected: proofCell(100_000_000),
          },
        }),
      }
    );
    expect(lower.mode).toBe("incremental");
    expect(lower.highWaterDeficit).toBeGreaterThan(0);
    expect(lower.increments["2026-10-08"]).toBeUndefined();
    expect(lower.nextState?.days["2026-10-08"]?.tokens).toBe(455_000_000);

    // Real growth on top of the corrected baseline is still credited.
    const grown = devinPlan(
      {},
      devinSnapshot(
        devinContribution("2026-10-07", 234_000_000),
        devinContribution("2026-10-08", 455_000_000),
        devinContribution("2026-10-09", 11_000_000)
      ),
      { state, recountProof: fullProof }
    );
    expect(grown.mode).toBe("incremental");
    expect(grown.nextState?.aggregate.tokens).toBe(700_000_000);
  });

  it("rejects a proof that does not describe the submitted snapshot", () => {
    // The corrected side must reproduce the submitted cell field by field;
    // otherwise the proof was computed from different rows than the
    // snapshot and proves nothing about it.
    const plan = devinPlan(inflatedStored, correctedScan, {
      recountProof: proof({
        "2026-10-07": { previous: proofCell(514_000_000), corrected: proofCell(234_000_000) },
        "2026-10-08": { previous: proofCell(1_000_000_000), corrected: proofCell(400_000_000) },
      }),
    });

    expect(plan.recountDays).toEqual(["2026-10-07"]);
    expect(plan.rejectedRecountDays).toEqual(["2026-10-08"]);
    expect(plan.nextState?.days["2026-10-07"]?.tokens).toBe(234_000_000);
    expect(plan.nextState?.days["2026-10-08"]?.tokens).toBe(1_000_000_000);
  });

  it("rejects a proof whose previous count is below its corrected count", () => {
    const plan = devinPlan(inflatedStored, correctedScan, {
      recountProof: proof({
        "2026-10-08": { previous: proofCell(455_000_000), corrected: proofCell(455_000_001) },
      }),
    });

    expect(plan.recountDays).toBeUndefined();
    expect(plan.rejectedRecountDays).toEqual(["2026-10-08"]);
  });

  it("rejects a proof naming a model the snapshot does not report, and vice versa", () => {
    const mismatchedModel = devinPlan(inflatedStored, correctedScan, {
      recountProof: proof({
        "2026-10-08": {
          previous: proofCell(1_000_000_000),
          corrected: proofCell(455_000_000),
          modelId: "swe-2-low",
        },
      }),
    });
    expect(mismatchedModel.rejectedRecountDays).toEqual(["2026-10-08"]);

    const partialModels = devinPlan(
      inflatedStored,
      devinSnapshot(
        devinContribution("2026-10-08", 400_000_000),
        devinContribution("2026-10-08", 55_000_000, "swe-2-low")
      ),
      {
        recountProof: proof({
          "2026-10-08": { previous: proofCell(900_000_000), corrected: proofCell(400_000_000) },
        }),
      }
    );
    expect(partialModels.rejectedRecountDays).toEqual(["2026-10-08"]);
  });

  it("ignores a recount proof from a client that does not ship one", () => {
    const plan = planParserHighWaterSubmission({
      client: "copilot",
      incomingVersion: SUPPORTED_VERSIONED_PARSERS.copilot,
      fullHistory: true,
      existingLegacyDays: foldParserClientSnapshot(
        [contribution("2026-10-08", 1_000)],
        "copilot"
      ),
      incomingDays: foldParserClientSnapshot([contribution("2026-10-08", 400)], "copilot"),
      recountProof: {
        "2026-10-08": {
          "model-a": { previous: proofCell(1_000), corrected: proofCell(400) },
        },
      },
    });

    expect(plan.recountDays).toBeUndefined();
    expect(plan.rejectedRecountDays).toBeUndefined();
    expect(plan.nextState?.days["2026-10-08"]?.tokens).toBe(1_000);
  });

  it("corrects cost only down to the corrected cost and keeps the stored tags", () => {
    const stored: ClientBreakdownData = {
      ...devinSnapshot(devinContribution("2026-10-08", 3_000, "swe-2-high", 30))["2026-10-08"],
    };
    stored.provenance = { ...stored.provenance!, costIsComplete: false, origin: "cli" };
    const proofDay = proof({
      "2026-10-08": { previous: proofCell(3_000, 30), corrected: proofCell(1_200, 8) },
    })["2026-10-08"];
    const submitted = submittedFor(proofDay);

    const exact = applyRecountProofToCell(stored, proofDay, submitted, AT_TRANSITION_FLOOR);
    expect(exact.cell.tokens).toBe(1_200);
    expect(exact.cell.cost).toBeCloseTo(8, 4);
    expect(exact.cell.models["swe-2-high"].cost).toBeCloseTo(8, 4);
    expect(exact.removed).toEqual({ tokens: 1_800, cost: 22, messages: 0 });
    // Amounts are restated; where they came from and whether their cost was
    // fully priced is not.
    expect(exact.cell.provenance?.costIsComplete).toBe(false);
    expect(exact.cell.provenance?.origin).toBe("cli");

    // A stored cost already partly corrected stops at the corrected cost
    // rather than taking the full proven difference off again.
    const partlyCorrected = { ...stored, cost: 15, models: { "swe-2-high": { ...stored.models["swe-2-high"], cost: 15 } } };
    expect(applyRecountProofToCell(partlyCorrected, proofDay, submitted, AT_TRANSITION).cell.cost).toBeCloseTo(8, 4);

    // A complete stored cost below the corrected cost is not touched; a floor
    // there is raised to the corrected cost, which prices exactly its rows.
    const belowCorrected = { ...stored, cost: 5, models: { "swe-2-high": { ...stored.models["swe-2-high"], cost: 5 } } };
    expect(applyRecountProofToCell(belowCorrected, proofDay, submitted, AT_TRANSITION).cell.cost).toBeCloseTo(5, 4);
    expect(applyRecountProofToCell(belowCorrected, proofDay, submitted, AT_TRANSITION_FLOOR).cell.cost).toBeCloseTo(8, 4);

    // An incomplete snapshot keeps every stored cost as a floor.
    const incomplete = applyRecountProofToCell(stored, proofDay, submitted, {
      ...AT_TRANSITION_FLOOR,
      costIsComplete: false,
      costUnsafeModels: new Set(["swe-2-high"]),
    });
    expect(incomplete.cell.tokens).toBe(1_200);
    expect(incomplete.cell.cost).toBeCloseTo(30, 4);
  });

  it("treats a stored cost within one stored unit above the previous cost as explained", () => {
    // Stored costs are quantized to 1/10,000 while proof costs are raw sums of
    // the same prices, so they can differ by noise below one stored unit.
    const base = devinSnapshot(devinContribution("2026-10-08", 3_000, "swe-2-high", 30))["2026-10-08"];
    const withCost = (cost: number): ClientBreakdownData => ({
      ...base,
      cost,
      models: { "swe-2-high": { ...base.models["swe-2-high"], cost } },
    });
    const proofDay = proof({
      "2026-10-08": { previous: proofCell(3_000, 30), corrected: proofCell(1_200, 8) },
    })["2026-10-08"];

    expect(applyRecountProofToCell(withCost(30.00005), proofDay, submittedFor(proofDay), AT_TRANSITION).cell.cost).toBeCloseTo(8, 4);
    // Beyond that slack the stored cost holds spend the proof does not explain.
    expect(applyRecountProofToCell(withCost(30.0003), proofDay, submittedFor(proofDay), AT_TRANSITION).cell.cost).toBeCloseTo(30.0003, 4);
  });

  it("proves a cost only when every model holds exactly the corrected counts and cost", () => {
    const proofDay = proof({
      "2026-10-08": { previous: proofCell(200, 0.2, 2), corrected: proofCell(100, 0.1, 1) },
    })["2026-10-08"];
    const cell = (tokens: number, cost: number) =>
      devinSnapshot(devinContribution("2026-10-08", tokens, "swe-2-high", cost, 1))["2026-10-08"];

    expect(recountProofProvesCost(cell(100, 0.1), proofDay, submittedFor(proofDay))).toBe(true);
    // Within one stored cost unit of the raw proof cost.
    expect(recountProofProvesCost(cell(100, 0.10005), proofDay, submittedFor(proofDay))).toBe(true);
    // Another source's cell can hold the same counts at a different cost.
    expect(recountProofProvesCost(cell(100, 0.15), proofDay, submittedFor(proofDay))).toBe(false);
    expect(recountProofProvesCost(cell(200, 0.2), proofDay, submittedFor(proofDay))).toBe(false);
    // A model the proof does not name keeps the cell unproven.
    const twoModels = devinSnapshot(
      devinContribution("2026-10-08", 100, "swe-2-high", 0.1, 1),
      devinContribution("2026-10-08", 50, "swe-1-5", 0.05, 1)
    )["2026-10-08"];
    expect(recountProofProvesCost(twoModels, proofDay, submittedFor(proofDay))).toBe(false);
  });

  it("reports what the transition's correction changed and whether it proves the cost", () => {
    const proofDay = proof({
      "2026-10-08": { previous: proofCell(200, 0.2, 2), corrected: proofCell(100, 0.1, 1) },
    })["2026-10-08"];

    const exact = applyRecountProofToCell(
      devinSnapshot(devinContribution("2026-10-08", 200, "swe-2-high", 0.2, 2))["2026-10-08"],
      proofDay,
      submittedFor(proofDay),
      AT_TRANSITION
    );
    expect(exact.changed).toBe(true);
    expect(exact.costProven).toBe(true);
    expect(exact.cell.tokens).toBe(100);

    // The counts are corrected, but a stored cost the previous cost does not
    // account for is kept, so the proof does not stand behind it.
    const unexplainedCost = applyRecountProofToCell(
      devinSnapshot(devinContribution("2026-10-08", 200, "swe-2-high", 0.5, 2))["2026-10-08"],
      proofDay,
      submittedFor(proofDay),
      AT_TRANSITION
    );
    expect(unexplainedCost.changed).toBe(true);
    expect(unexplainedCost.costProven).toBe(false);
    expect(unexplainedCost.cell.tokens).toBe(100);
    expect(unexplainedCost.cell.cost).toBeCloseTo(0.5, 4);

    // A model the proof does not name is untouched and keeps the cell
    // unproven.
    const partly = applyRecountProofToCell(
      devinSnapshot(
        devinContribution("2026-10-08", 200, "swe-2-high", 0.2, 2),
        devinContribution("2026-10-08", 50, "swe-1-5", 0.05, 1)
      )["2026-10-08"],
      proofDay,
      submittedFor(proofDay),
      AT_TRANSITION
    );
    expect(partly.changed).toBe(true);
    expect(partly.costProven).toBe(false);
    expect(partly.cell.models["swe-2-high"].tokens).toBe(100);
    expect(partly.cell.models["swe-1-5"].tokens).toBe(50);

    // Counts that are already the corrected ones are not a recount: the cell
    // and its cost are returned unchanged.
    const sameCounts = applyRecountProofToCell(
      devinSnapshot(devinContribution("2026-10-08", 100, "swe-2-high", 0.15, 1))["2026-10-08"],
      proofDay,
      submittedFor(proofDay),
      AT_TRANSITION
    );
    expect(sameCounts.changed).toBe(false);
    expect(sameCounts.cell.cost).toBeCloseTo(0.15, 4);
  });

  it("keeps a model's stored cost when the proof's cost is not what the snapshot submitted", () => {
    // Submission zeroed an estimate it could not price safely after the proof
    // was built, so the proof's $0.10 rests on that estimate. The counts are
    // corrected; the stored $0.15 is kept.
    const proofDay = proof({
      "2026-10-08": { previous: proofCell(200, 0.2, 2), corrected: proofCell(100, 0.1, 1) },
    })["2026-10-08"];
    const zeroed = devinSnapshot(devinContribution("2026-10-08", 100, "swe-2-high", 0, 1))["2026-10-08"];
    const stored = devinSnapshot(devinContribution("2026-10-08", 200, "swe-2-high", 0.15, 2))["2026-10-08"];

    const correction = applyRecountProofToCell(stored, proofDay, zeroed, AT_TRANSITION);
    // The same on the incomplete day the submission actually marked.
    expect(
      applyRecountProofToCell(stored, proofDay, zeroed, {
        ...AT_TRANSITION,
        costIsComplete: false,
        costUnsafeModels: new Set(["swe-2-high"]),
      }).cell.cost
    ).toBeCloseTo(0.15, 4);
    expect(correction.changed).toBe(true);
    expect(correction.costProven).toBe(false);
    expect(correction.cell.tokens).toBe(100);
    expect(correction.cell.messages).toBe(1);
    expect(correction.cell.cost).toBeCloseTo(0.15, 4);
    expect(correction.removed).toEqual({ tokens: 100, cost: 0, messages: 1 });

    // Nor does such a proof show a cost to be complete.
    const atCorrected = devinSnapshot(devinContribution("2026-10-08", 100, "swe-2-high", 0.1, 1))["2026-10-08"];
    expect(recountProofProvesCost(atCorrected, proofDay, zeroed)).toBe(false);
    expect(recountProofProvesCost(atCorrected, proofDay, submittedFor(proofDay))).toBe(true);
  });

  it("changes the cost of a model the snapshot did not submit only when every day naming it is priced", () => {
    // Every stored row on the 8th is a duplicate of a survivor dated another
    // day, so the snapshot submits nothing for the 8th and the corrected side
    // is empty. The old CLI priced the rows at $0.15.
    const proofDay = proof({
      "2026-10-08": { previous: proofCell(100, 0.2, 1), corrected: proofCell(0, 0, 0) },
    })["2026-10-08"];
    const stored = devinSnapshot(devinContribution("2026-10-08", 100, "swe-2-high", 0.15, 1))["2026-10-08"];

    // A day the proof names the model on could not be priced, so the
    // survivors' price is unknown: the counts go, the cost stays.
    const partial = applyRecountProofToCell(stored, proofDay, undefined, {
      ...AT_TRANSITION,
      costUnsafeModels: new Set(["swe-2-high"]),
    });
    expect(partial.cell.tokens).toBe(0);
    expect(partial.cell.cost).toBeCloseTo(0.15, 4);
    expect(partial.costProven).toBe(false);

    // With every such day priced, the duplicates' cost goes with them.
    const complete = applyRecountProofToCell(stored, proofDay, undefined, AT_TRANSITION);
    expect(complete.cell.tokens).toBe(0);
    expect(complete.cell.cost).toBe(0);
  });

  it("marks the models a proof names on an unpriced day, or all of them once a day was rejected", () => {
    const proofOf = {
      "2026-10-08": {
        "swe-2-high": { previous: proofCell(200, 0.2), corrected: proofCell(100, 0.1) },
      },
      "2026-10-09": {
        "swe-1-5": { previous: proofCell(50, 0.05), corrected: proofCell(50, 0.05) },
        "swe-2-high": { previous: proofCell(80, 0.08), corrected: proofCell(80, 0.08) },
      },
    };
    const accepted = ["2026-10-08", "2026-10-09"];

    expect(recountCostUnsafeModels(proofOf, { accepted, rejected: [] }, new Set())).toEqual(new Set());
    expect(
      recountCostUnsafeModels(proofOf, { accepted, rejected: [] }, new Set(["2026-10-08"]))
    ).toEqual(new Set(["swe-2-high"]));
    expect(
      recountCostUnsafeModels(proofOf, { accepted, rejected: [] }, new Set(["2026-10-09"]))
    ).toEqual(new Set(["swe-1-5", "swe-2-high"]));
    expect(
      recountCostUnsafeModels(proofOf, { accepted: ["2026-10-08"], rejected: ["2026-10-09"] }, new Set())
    ).toEqual(new Set(["swe-1-5", "swe-2-high"]));
  });

  it("holds back a model without survivors while any proof day is unpriced", () => {
    // swe-1-5's rows on the 8th are all duplicates; its survivor could be the
    // 9th's swe-2-high (a copy that lost its generation model), and the 9th is
    // unpriced. swe-1-5 never appears on the 9th, yet it waits too.
    const proofOf = {
      "2026-10-08": {
        "swe-1-5": { previous: proofCell(100, 0.15), corrected: proofCell(0, 0, 0) },
      },
      "2026-10-09": {
        "swe-2-high": { previous: proofCell(100, 0.15), corrected: proofCell(100, 0.15) },
      },
    };
    const recount = { accepted: ["2026-10-08", "2026-10-09"], rejected: [] };

    expect(recountCostUnsafeModels(proofOf, recount, new Set(["2026-10-09"]))).toEqual(
      new Set(["swe-1-5", "swe-2-high"])
    );
    // With every day priced, nothing waits.
    expect(recountCostUnsafeModels(proofOf, recount, new Set())).toEqual(new Set());
  });

  it("reports the unsafe models in the plan and keeps their stored cost", () => {
    // The 8th's survivors are priced, but swe-2-high also appears on the
    // unpriced 9th, so its cost moves on neither day.
    const stored = devinSnapshot(devinContribution("2026-10-08", 200, "swe-2-high", 0.2, 1));
    const plan = planParserHighWaterSubmission({
      client: "devin-cli",
      incomingVersion: SUPPORTED_VERSIONED_PARSERS["devin-cli"],
      fullHistory: true,
      existingLegacyDays: stored,
      incomingDays: devinSnapshot(
        devinContribution("2026-10-08", 100, "swe-2-high", 0.1, 1),
        devinContribution("2026-10-09", 50, "swe-2-high", 0, 1)
      ),
      recountProof: proof({
        "2026-10-08": { previous: proofCell(200, 0.2), corrected: proofCell(100, 0.1) },
        "2026-10-09": { previous: proofCell(50, 0.05), corrected: proofCell(50, 0.05) },
      }),
      costIncompleteDates: new Set(["2026-10-09"]),
    });

    expect(plan.recountCostUnsafeModels).toEqual(["swe-2-high"]);
    expect(plan.nextState?.days["2026-10-08"]?.tokens).toBe(100);
    expect(plan.nextState?.days["2026-10-08"]?.cost).toBeCloseTo(0.2, 4);
  });

  it("keeps generation-1 submissions compatible before the transition", () => {
    const plan = planParserHighWaterSubmission({
      client: "devin-cli",
      incomingVersion: 1,
      fullHistory: true,
      existingLegacyDays: inflatedStored,
      incomingDays: inflatedStored,
    });

    // The receiver deploys before a generation-2 CLI is released. Preserve the
    // existing merge behavior until this device starts its transition rather
    // than silently dropping Devin usage from every installed CLI.
    expect(plan.mode).toBe("status-quo");
    expect(plan.nextState).toBeUndefined();
  });

  it("freezes generation 1 after the generation-2 transition starts", () => {
    const state = devinPlan(inflatedStored, correctedScan, {
      recountProof: fullProof,
    }).nextState!;
    const plan = planParserHighWaterSubmission({
      client: "devin-cli",
      incomingVersion: 1,
      fullHistory: true,
      existingLegacyDays: correctedScan,
      incomingDays: inflatedStored,
      state,
      persistedVersion: 2,
    });

    expect(plan.mode).toBe("freeze");
    expect(plan.nextState).toBeUndefined();
  });

  it("does not let a partial snapshot correct anything", () => {
    const partial = planParserHighWaterSubmission({
      client: "devin-cli",
      incomingVersion: SUPPORTED_VERSIONED_PARSERS["devin-cli"],
      fullHistory: false,
      existingLegacyDays: inflatedStored,
      incomingDays: devinSnapshot(devinContribution("2026-10-08", 455_000_000)),
      recountProof: fullProof,
    });

    expect(partial.mode).toBe("freeze");
    expect(partial.recountDays).toBeUndefined();
    expect(partial.nextState?.baselineEstablished).toBe(false);

    // The pending state must not close the transition: a later full-history
    // snapshot at the registered generation still applies its proof.
    const followed = devinPlan(inflatedStored, correctedScan, {
      state: partial.nextState,
      recountProof: fullProof,
    });
    expect(followed.mode).toBe("baseline-legacy");
    expect(followed.nextState?.aggregate.tokens).toBe(689_000_000);
  });

  it("keeps the transition pending on an empty client snapshot", () => {
    const empty = devinPlan(inflatedStored, {});

    // A default all-client submit declares every default parser even when this
    // client produced no rows. A missing, unreadable, or removed Devin database
    // must not baseline the inflated legacy rows as this generation's ledger.
    expect(empty.mode).toBe("freeze");
    expect(empty.nextState?.baselineEstablished).toBe(false);

    const followed = devinPlan(inflatedStored, correctedScan, {
      state: empty.nextState,
      recountProof: fullProof,
    });
    expect(followed.mode).toBe("baseline-legacy");
    expect(followed.nextState?.aggregate.tokens).toBe(689_000_000);
  });

  it("baselines normally when no inflated history is stored", () => {
    const plan = devinPlan({}, correctedScan, { recountProof: fullProof });

    expect(plan.mode).toBe("baseline-new");
    expect(plan.recountDays).toEqual(["2026-10-07", "2026-10-08"]);
    expect(plan.nextState?.aggregate.tokens).toBe(689_000_000);
  });

  it("keeps a first empty snapshot pending even with nothing stored", () => {
    // A failed first read on a fresh device must not consume the transition:
    // baselining generation 2 on nothing would freeze the generation-1
    // submits this device may still make before it upgrades.
    const empty = devinPlan({}, {});

    expect(empty.mode).toBe("freeze");
    expect(empty.nextState?.baselineEstablished).toBe(false);

    const followed = devinPlan({}, correctedScan, {
      state: empty.nextState,
      recountProof: fullProof,
    });
    expect(followed.mode).toBe("baseline-new");
    expect(followed.nextState?.aggregate.tokens).toBe(689_000_000);
  });

  it("rejects a proof whose corrected cost sits below the submitted cost", () => {
    const stored = devinSnapshot(devinContribution("2026-10-08", 1_000, "swe-2-high", 20));
    const scan = devinSnapshot(devinContribution("2026-10-08", 455, "swe-2-high", 9));
    const plan = devinPlan(stored, scan, {
      recountProof: proof({
        "2026-10-08": { previous: proofCell(1_000, 20), corrected: proofCell(455, 0) },
      }),
    });

    // Tokens match the snapshot, but the proof would floor cost under what
    // this very scan prices the rows at.
    expect(plan.rejectedRecountDays).toEqual(["2026-10-08"]);
    expect(plan.recountDays).toBeUndefined();
    expect(plan.nextState?.days["2026-10-08"]?.cost).toBe(20);
  });

  it("accepts a proof whose corrected cost exceeds a submission that zeroed unpriced rows, keeping the stored cost", () => {
    const stored = devinSnapshot(devinContribution("2026-10-08", 1_000, "swe-2-high", 20));
    // Submission zeroes cost it cannot price after the proof was built, so
    // the snapshot may carry LESS cost than the proof's corrected side. The
    // proof still corrects the counts, but its $9 rests on that unsafe cost,
    // so the stored $20 stays, as the route keeps it for the stored cell.
    const scan = devinSnapshot(devinContribution("2026-10-08", 455, "swe-2-high", 0));
    const plan = devinPlan(stored, scan, {
      recountProof: proof({
        "2026-10-08": { previous: proofCell(1_000, 20), corrected: proofCell(455, 9) },
      }),
    });

    expect(plan.recountDays).toEqual(["2026-10-08"]);
    expect(plan.nextState?.days["2026-10-08"]?.tokens).toBe(455);
    expect(plan.nextState?.days["2026-10-08"]?.cost).toBe(20);
  });

  it("applies an all-duplicate proof day the snapshot does not date", () => {
    // Every generation-1 row on the 6th was a request_id duplicate of a row
    // dated elsewhere, so generation 2 reports nothing for the day. The proof
    // may describe that absence exactly as an empty corrected side.
    const stored = devinSnapshot(
      devinContribution("2026-10-06", 88_000_000),
      devinContribution("2026-10-08", 1_000_000_000)
    );
    const scan = devinSnapshot(devinContribution("2026-10-08", 455_000_000));
    const plan = devinPlan(stored, scan, {
      recountProof: proof({
        "2026-10-06": { previous: proofCell(88_000_000), corrected: proofCell(0, 0, 0) },
        "2026-10-08": { previous: proofCell(1_000_000_000), corrected: proofCell(455_000_000) },
      }),
    });

    expect(plan.recountDays).toEqual(["2026-10-06", "2026-10-08"]);
    expect(plan.nextState?.aggregate.tokens).toBe(455_000_000);

    // A non-empty corrected side for an undated day is still a proof for
    // rows the snapshot does not contain, and is refused.
    const mismatched = devinPlan(stored, scan, {
      recountProof: proof({
        "2026-10-06": { previous: proofCell(88_000_000), corrected: proofCell(10) },
      }),
    });
    expect(mismatched.rejectedRecountDays).toEqual(["2026-10-06"]);
  });
});

describe("per-client full-history scope", () => {
  it("freezes only clients omitted from a new completeness declaration", () => {
    const scope = {
      // Legacy receivers fail closed for an incomplete client.
      fullHistory: false,
      fullHistoryClients: ["copilot", "droid"],
    };
    expect(scanScopeCoversFullHistory(scope, ["amp"])).toBe(false);
    expect(scanScopeCoversFullHistory(scope, ["copilot"])).toBe(true);
    expect(scanScopeCoversFullHistory(scope, ["copilot", "droid"])).toBe(true);
  });

  it("keeps the legacy global full-history meaning when the field is absent", () => {
    expect(scanScopeCoversFullHistory({ fullHistory: true }, ["amp"])).toBe(true);
    expect(scanScopeCoversFullHistory({ fullHistory: false }, ["amp"])).toBe(false);
  });
});

describe("amp parser high-water", () => {
  function contribution(date: string, tokens: number) {
    return {
      date,
      clients: [{
        client: "amp",
        modelId: "claude-sonnet-4-0",
        tokens: { input: tokens, output: 0, cacheRead: 0, cacheWrite: 0, reasoning: 0 },
        cost: 0,
        messages: 1,
      }],
    };
  }

  it("replaces a full re-attributed snapshot without changing lifetime tokens", () => {
    // v1 dated this message on the thread-created day; v2 uses its server
    // usage timestamp. A normal per-day guard would retain 100 on Apr 4 and
    // add another 100 on Apr 9.
    const oldDays = foldParserClientSnapshot([contribution("2026-04-04", 100)], "amp");
    const newDays = foldParserClientSnapshot([contribution("2026-04-09", 100)], "amp");
    const plan = planParserHighWaterSubmission({
      client: "amp",
      incomingVersion: SUPPORTED_VERSIONED_PARSERS.amp,
      fullHistory: true,
      existingLegacyDays: oldDays,
      incomingDays: newDays,
    });

    expect(SUPPORTED_VERSIONED_PARSERS.amp).toBe(2);
    expect(plan.mode).toBe("replace");
    expect(plan.increments).toEqual({});
    expect(plan.layoutDays?.["2026-04-04"]).toBeUndefined();
    expect(plan.layoutDays?.["2026-04-09"]?.tokens).toBe(100);
    expect(plan.nextState?.aggregate.tokens).toBe(100);
  });

  it("freezes an aggregate-covering v2 snapshot when Amp cache coverage is incomplete", () => {
    const oldDays = foldParserClientSnapshot([contribution("2026-04-04", 100)], "amp");
    const partialDays = foldParserClientSnapshot([contribution("2026-04-09", 150)], "amp");
    const plan = planParserHighWaterSubmission({
      client: "amp",
      incomingVersion: SUPPORTED_VERSIONED_PARSERS.amp,
      fullHistory: false,
      existingLegacyDays: oldDays,
      incomingDays: partialDays,
    });

    expect(plan.mode).toBe("freeze");
    expect(plan.increments).toEqual({});
    expect(plan.layoutDays).toBeUndefined();
  });

  it("accepts v1 before transition and freezes it after a v2 baseline", () => {
    const oldDays = foldParserClientSnapshot([contribution("2026-04-04", 100)], "amp");
    const v1 = planParserHighWaterSubmission({
      client: "amp",
      incomingVersion: 1,
      fullHistory: true,
      existingLegacyDays: oldDays,
      incomingDays: oldDays,
    });
    expect(v1.mode).toBe("status-quo");

    const v2 = planParserHighWaterSubmission({
      client: "amp",
      incomingVersion: 2,
      fullHistory: true,
      existingLegacyDays: oldDays,
      incomingDays: foldParserClientSnapshot([contribution("2026-04-09", 100)], "amp"),
    });
    const staleV1 = planParserHighWaterSubmission({
      client: "amp",
      incomingVersion: 1,
      fullHistory: true,
      existingLegacyDays: {},
      incomingDays: oldDays,
      state: v2.nextState,
    });
    expect(staleV1.mode).toBe("freeze");
  });
});

describe("unverified retention floor", () => {
  function floored(
    state: ParserClientHighWaterState,
    incomingDays: Record<string, ClientBreakdownData>,
    retentionFloor?: string
  ) {
    return planParserHighWaterSubmission({
      client: "copilot",
      incomingVersion: 2,
      fullHistory: true,
      retentionFloor,
      existingLegacyDays: {},
      incomingDays,
      state,
    });
  }

  const credited = baseline(
    {},
    snapshot(contribution("2026-07-01", 100), contribution("2026-07-02", 200))
  ).nextState!;

  it("keeps the lifetime bound when an unverified floor advances with a re-dated snapshot", () => {
    const plan = floored(
      credited,
      snapshot(contribution("2026-08-01", 300)),
      "2026-08-01"
    );

    expect(plan.mode).toBe("incremental");
    expectCreditedNothing(plan);
    expect(plan.nextState?.aggregate.tokens).toBe(300);
  });

  it("does not let an unverified floor erase a token deficit", () => {
    const plan = floored(
      credited,
      snapshot(contribution("2026-08-01", 50)),
      "2026-08-01"
    );

    expect(plan.mode).toBe("incremental");
    expectCreditedNothing(plan);
    expect(plan.highWaterDeficit).toBe(250);
  });

  it("does not let a floor remove a subset of credited days from the baseline", () => {
    const plan = floored(
      credited,
      snapshot(contribution("2026-07-02", 200), contribution("2026-08-01", 50)),
      "2026-07-02"
    );

    expectCreditedNothing(plan);
    expect(plan.nextState?.aggregate.tokens).toBe(300);
  });

  it("does not re-credit a known day under an unverified floor", () => {
    const plan = floored(
      credited,
      snapshot(contribution("2026-07-01", 100), contribution("2026-07-02", 200)),
      "2026-08-01"
    );

    expectCreditedNothing(plan);
    expect(plan.nextState?.aggregate.tokens).toBe(300);
  });

  it("keeps the lifetime bound for a re-dated snapshot with an unverified floor", () => {
    const plan = floored(
      credited,
      snapshot(contribution("2026-07-01", 100), contribution("2026-08-01", 200)),
      "2026-07-02"
    );
    expectCreditedNothing(plan);
    expect(plan.nextState?.aggregate.tokens).toBe(300);
  });

  it("keeps the lifetime bound at a legacy transition with an unverified floor", () => {
    const plan = planParserHighWaterSubmission({
      client: "copilot",
      incomingVersion: 2,
      fullHistory: true,
      retentionFloor: "2026-07-02",
      existingLegacyDays: credited.days,
      incomingDays: snapshot(contribution("2026-07-01", 100), contribution("2026-08-01", 200)),
    });
    expect(plan.mode).toBe("baseline-legacy");
    expectCreditedNothing(plan);
    expect(plan.nextState?.aggregate.tokens).toBe(300);
  });

  it("does not let an unverified floor unfreeze a partial snapshot", () => {
    const plan = planParserHighWaterSubmission({
      client: "copilot", incomingVersion: 2, fullHistory: false,
      retentionFloor: "2026-09-01", existingLegacyDays: {},
      incomingDays: snapshot(contribution("2026-09-01", 50)), state: credited,
    });
    expect(plan.mode).toBe("freeze");
    expect(plan.increments).toEqual({});
  });
});
