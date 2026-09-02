import { describe, expect, it } from "vitest";
import {
  addClientBreakdownIncrement,
  foldParserClientSnapshot,
  planParserHighWaterSubmission,
  SUPPORTED_VERSIONED_PARSERS,
  type ParserClientHighWaterState,
  type ParserHighWaterPlan,
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
