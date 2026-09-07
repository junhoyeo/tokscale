import { randomUUID } from "node:crypto";
import postgres from "postgres";
import { afterAll, beforeAll, describe, expect, it, vi } from "vitest";

import { getModerationCandidates } from "@/lib/moderation/candidates";
import {
  UNKNOWABLE_BUCKET_WIDTH,
  UNKNOWABLE_EVENT,
} from "@/lib/moderation/heuristics";

const integrationEnabled =
  process.env.MODERATION_CANDIDATES_DB_INTEGRATION === "1";
const describeWithPostgres = integrationEnabled ? describe : describe.skip;

/**
 * Executes the real candidate SQL against a migrated PostgreSQL instance.
 *
 * The moderation route tests mock `db.execute`, so a green unit suite says
 * nothing about what the statement extracts from
 * `daily_breakdown.source_breakdown`. Everything `slopTokens` promises — that
 * a legacy entry's scalar remainder is credited to its own `modelId` rather
 * than dropped, that the remainder is not credited when that `modelId` itself
 * names nothing, that the remainder is not also counted inside the nested
 * per-model sum, that a cell whose key names nothing (`unknown`, parser
 * debris) is not mistaken for attribution once the same remainder has been
 * normalized into the map, that an entry whose map sums past its own scalar
 * attributes nothing at all rather than a clamped amount, and that anything
 * short of full attribution comes back NULL so the scorer falls back to the
 * full fixed weight — is only observable by running it.
 *
 * Over-nesting is guarded in TWO places in candidates.ts, and only one of them
 * decides anything. `has_over_nested_entry` — BOOL_OR of a per-entry flag,
 * ANDed into the completeness gate — is the guard. Zeroing
 * attributed_nested_tokens for the same entry is defence in depth that no
 * assertion here can observe, because the flag has already forced the whole
 * submission to NULL before the attributed sum is compared to anything. Each
 * of these four counts was measured by editing the SQL and re-running this
 * file against postgres:16, not reasoned about:
 *
 *   - dropping the flag, leaving the zeroed attribution to carry the
 *     contradiction on its own -> 2 failed / 15 passed: ONLY
 *     overNestedZeroScalar and overNestedNoScalar, and none of the three
 *     over-nested personas that carry a real scalar
 *   - dropping the zeroing, leaving the flag -> 17 passed. The zeroing is
 *     currently unobservable. It is kept because it costs nothing and would
 *     matter if the flag were ever narrowed, NOT because anything tests it,
 *     and the same is true of any clamp put in its place: with the flag
 *     standing, GREATEST(LEAST(named, scalar - unnamed), 0) is green too.
 *     Do not read a green run as evidence that this expression is right.
 *   - dropping BOTH -> 5 failed / 12 passed: every over-nested persona. This
 *     is the revert-the-whole-treatment mutation and the one the three
 *     original personas still guard.
 *   - dropping `attributed_tokens >= total_tokens`, leaving the flag
 *     -> 5 failed / 12 passed: unclaimed, unknownBucket, debrisBucket,
 *     unknownModelId, remainderPlusUnknown. Eight before the flag existed;
 *     the three over-nested personas moved to the flag's column.
 *
 * Two further mutations of the name filters are each killed by a persona:
 * dropping the UNNAMED_MODEL_REGEX guard on the client-level `modelId`
 * -> unknownModelId, and dropping the UNNAMED_MODEL_REGEX filter on the named
 * nested sum -> remainderPlusUnknown, unknownBucket, debrisBucket.
 *
 * Why the flag rather than arithmetic, which is the whole reason
 * overNestedZeroScalar and overNestedNoScalar exist: the gate compares
 * attributed_tokens against total_tokens, and total_tokens is SUM(daily.tokens)
 * which is in turn the sum of the client scalars (recalculateDayTotals). An
 * entry whose own scalar is 0 or absent therefore adds nothing to the
 * threshold, so zeroing its attribution subtracts nothing from the other side
 * either — the gate passes EXACTLY, on the passing side, and the account
 * leaves the queue. Measured before the flag: slopTokens 2 against a 1,200,000
 * total, weight 35 * 2/1,200,000, Math.round -> 0, slopModelName absent. This
 * is the same shape as the clamp failure the original three personas were
 * written for (a ceiling equal to the entry's own scalar can never push a row
 * below a threshold summed from those same scalars) with the ceiling and the
 * scalar both at zero — which is exactly why a clamp-shaped fix, or a
 * zeroing-shaped one, cannot reach it and a flag can.
 *
 * The seventh survives and is meant to: dropping `every_day_attributed` from
 * the completeness gate leaves the suite green, because that clause is
 * redundant by arithmetic rather than untested. submissions.total_tokens is
 * written as SUM(daily.tokens) (submit/route.ts STEP 3d) and a day scalar as
 * the sum of its client scalars, so a daily row carrying no breakdown always
 * drags attributed_tokens below total_tokens on its own. It is kept as defence
 * in depth for the case where those two representations drift apart, which is
 * the class of bug #960 is. Do not read its survival as licence to delete it.
 *
 * Weakening any of the other six without a red test is how a fail-open
 * reaches the review queue unnoticed. Mutate before trusting this file.
 *
 * Unlike the ratchet census fixture, this one does not assume it owns the
 * database: every persona is `leaderboard_hidden`, so it stays in the ranked
 * queue whatever its score, and the assertions read only `slopTokens` and the
 * `slopModelName` signal, neither of which depends on site-wide totals.
 */
describeWithPostgres("moderation candidates PostgreSQL integration", () => {
  const databaseUrl = process.env.DATABASE_URL;
  const fixtureSuffix = randomUUID().replaceAll("-", "").slice(0, 8);

  const personas = [
    // Scalar 1,200,000 with a nested map holding 2, and a client-level
    // `modelId` naming the slop model. modelsForHighWater() credits that
    // remainder to `modelId`, so all 1,200,000 are the slop model's.
    "partial",
    // Same partial map, but the remainder is claimed by a real model name.
    "blend",
    // Same partial map with no `modelId` at all: the remainder belongs to no
    // named model, so the share is unknowable.
    "unclaimed",
    // The pre-`models` shape: a client-level `modelId` and nothing nested.
    "legacy",
    // One legacy row with no breakdown alongside one attributed row.
    "mixed",
    // Fully attributed, and the slop model really is a rounding error (#1265).
    "artifact",
    // Fully attributed, and all of it is booked under the slop model.
    "wholly",
    // The remainder already normalized into the literal `unknown` cell that
    // modelsForHighWater() writes: the nested sum equals the scalar, so
    // nothing looks missing, yet the bucket names no model.
    "unknownBucket",
    // The same shape under the parser debris key heuristics.ts documents.
    "debrisBucket",
    // A real model name that merely contains the sentinel, guarding the
    // anchors on UNNAMED_MODEL_REGEX.
    "namedLikeUnknown",
    // The scalar is honest but the map outruns it: applyCostCompleteness()
    // unions the stored and incoming model maps while taking `tokens` from the
    // incoming entry, so a same-device resubmit that declares
    // costIsComplete:false and drops a previously-seen model stores a map whose
    // sum is larger than the entry's own scalar. Three constructions, because
    // where the excess sits is what a clamp is sensitive to and the property
    // is not:
    //   - the excess parked under an unnamed key,
    "overNested",
    //   - the same merge where the dropped model has a real name, so no
    //     unnamed cell exists to charge the excess against,
    "overNestedNamed",
    //   - the cheapest shape of all: a stored named map merged with an
    //     incoming legacy-partial one, over-nested by 2 tokens with no unnamed
    //     cell and no second large cell.
    "overNestedAllNamed",
    //   - the same contradiction in an entry whose OWN scalar is zero, which
    //     no arithmetic gate can catch: such an entry adds nothing to
    //     attributed_tokens and nothing to total_tokens (day scalars are
    //     summed from client scalars), so zeroing its attribution moves
    //     neither side of `attributed_tokens >= total_tokens` and the gate
    //     passes exactly. Only an explicit over_nested flag fails it closed.
    "overNestedZeroScalar",
    //   - the same shape with the `tokens` key absent rather than 0, since
    //     the query reaches the scalar through COALESCE(...->>'tokens', 0)
    //     and both spellings land on the same value.
    "overNestedNoScalar",
    // The legacy client-level `modelId` shape, but the id itself names nothing.
    "unknownModelId",
    // A scalar remainder AND an unnamed cell in the same entry: the only shape
    // where the clamp and the named-cell filter disagree.
    "remainderPlusUnknown",
  ] as const;
  type Persona = (typeof personas)[number];

  const ids = Object.fromEntries(
    personas.map((persona) => [
      persona,
      {
        userId: randomUUID(),
        submissionId: randomUUID(),
        deviceId: randomUUID(),
        username: `mod_${persona}_${fixtureSuffix}`,
      },
    ])
  ) as Record<
    Persona,
    { userId: string; submissionId: string; deviceId: string; username: string }
  >;

  // Distinct per persona and more than NEAR_DUPLICATE_TOKENS apart, so no
  // persona picks up a duplicate-total signal from another.
  const totals: Record<Persona, number> = {
    partial: 1_200_000,
    blend: 1_300_000,
    unclaimed: 1_400_000,
    legacy: 1_500_000,
    mixed: 1_600_000,
    artifact: 1_700_000,
    wholly: 1_800_000,
    unknownBucket: 2_100_000,
    debrisBucket: 2_200_000,
    namedLikeUnknown: 2_300_000,
    overNested: 3_100_000,
    overNestedNamed: 3_200_000,
    overNestedAllNamed: 4_100_000,
    overNestedZeroScalar: 5_100_000,
    overNestedNoScalar: 5_200_000,
    unknownModelId: 2_400_000,
    remainderPlusUnknown: 2_500_000,
  };

  const githubIdBase =
    -1_800_000_000 + Number.parseInt(fixtureSuffix.slice(0, 6), 16);

  let fixtureDb: ReturnType<typeof postgres>;

  const model = (tokens: number) => ({
    tokens,
    cost: 0,
    input: tokens,
    output: 0,
    cacheRead: 0,
    cacheWrite: 0,
    reasoning: 0,
    messages: 1,
  });

  beforeAll(async () => {
    if (!databaseUrl) {
      throw new Error(
        "DATABASE_URL is required when MODERATION_CANDIDATES_DB_INTEGRATION=1"
      );
    }

    fixtureDb = postgres(databaseUrl, { max: 1, prepare: false });

    await fixtureDb.begin(async (sql) => {
      const modelsUsed: Record<Persona, string[]> = {
        partial: ["fake-api"],
        blend: ["fake-api", "claude-sonnet-4"],
        unclaimed: ["fake-api"],
        legacy: ["fake-api"],
        mixed: ["fake-api"],
        artifact: ["fake-api", "claude-sonnet-4"],
        wholly: ["slopllm"],
        // submit/route.ts adds every key of the client's model map to
        // models_used, debris buckets included.
        unknownBucket: ["unknown", "fake-api"],
        debrisBucket: ["*", "fake-api"],
        namedLikeUnknown: ["unknown-model", "fake-api"],
        overNested: ["claude-sonnet-4", "unknown", "fake-api"],
        overNestedNamed: ["claude-sonnet-4", "claude-opus-4", "fake-api"],
        overNestedAllNamed: ["claude-sonnet-4", "fake-api"],
        overNestedZeroScalar: ["claude-sonnet-4", "fake-api"],
        overNestedNoScalar: ["claude-sonnet-4", "fake-api"],
        unknownModelId: ["unknown", "fake-api"],
        remainderPlusUnknown: ["claude-sonnet-4", "unknown", "fake-api"],
      };

      for (const [index, persona] of personas.entries()) {
        const { userId, submissionId, deviceId, username } = ids[persona];
        await sql`
          INSERT INTO users (id, github_id, username, leaderboard_hidden)
          VALUES (${userId}, ${githubIdBase + index}, ${username}, true)
        `;
        await sql`
          INSERT INTO submissions (
            id, user_id, total_tokens, total_cost, input_tokens, output_tokens,
            date_start, date_end, sources_used, models_used
          )
          VALUES (
            ${submissionId}, ${userId}, ${totals[persona]}, 0,
            ${totals[persona]}, 0, '2026-01-01', '2026-01-02',
            ARRAY['claude'], ${modelsUsed[persona]}
          )
        `;
        await sql`
          INSERT INTO submitted_devices (id, user_id, device_key)
          VALUES (${deviceId}, ${userId}, ${`mod-${persona}-device`})
        `;
      }

      /** One daily row. `breakdown` null reproduces a legacy pre-breakdown row. */
      const day = (
        persona: Persona,
        date: string,
        tokens: number,
        breakdown: postgres.JSONValue
      ) =>
        sql`
          INSERT INTO daily_breakdown (
            submission_id, submitted_device_id, date, tokens, cost,
            input_tokens, output_tokens, source_breakdown
          )
          VALUES (
            ${ids[persona].submissionId}, ${ids[persona].deviceId}, ${date},
            ${tokens}, 0, ${tokens}, 0,
            ${breakdown === null ? null : sql.json(breakdown)}
          )
        `;

      await day("partial", "2026-01-01", totals.partial, {
        claude: {
          ...model(totals.partial),
          modelId: "fake-api",
          models: { "fake-api": model(2) },
        },
      });

      await day("blend", "2026-01-01", totals.blend, {
        claude: {
          ...model(totals.blend),
          modelId: "claude-sonnet-4",
          models: { "fake-api": model(totals.blend / 2) },
        },
      });

      await day("unclaimed", "2026-01-01", totals.unclaimed, {
        claude: { ...model(totals.unclaimed), models: { "fake-api": model(2) } },
      });

      await day("legacy", "2026-01-01", totals.legacy, {
        claude: { ...model(totals.legacy), modelId: "fake-api" },
      });

      await day("mixed", "2026-01-01", totals.mixed - 2, null);
      await day("mixed", "2026-01-02", 2, {
        claude: { ...model(2), models: { "fake-api": model(2) } },
      });

      await day("artifact", "2026-01-01", totals.artifact, {
        claude: {
          ...model(totals.artifact),
          models: {
            "fake-api": model(2),
            "claude-sonnet-4": model(totals.artifact - 2),
          },
        },
      });

      await day("wholly", "2026-01-01", totals.wholly, {
        claude: {
          ...model(totals.wholly),
          models: { slopllm: model(totals.wholly) },
        },
      });

      // Both of these are what breakdownFromModels(modelsForHighWater(...))
      // stores for a high-water client: no `modelId`, and a scalar exactly
      // equal to the nested sum, so the implicit-remainder check sees nothing
      // missing.
      await day("unknownBucket", "2026-01-01", totals.unknownBucket, {
        copilot: {
          ...model(totals.unknownBucket),
          models: {
            unknown: model(totals.unknownBucket - 2),
            "fake-api": model(2),
          },
        },
      });

      await day("debrisBucket", "2026-01-01", totals.debrisBucket, {
        copilot: {
          ...model(totals.debrisBucket),
          models: {
            "*": model(totals.debrisBucket - 2),
            "fake-api": model(2),
          },
        },
      });

      await day("namedLikeUnknown", "2026-01-01", totals.namedLikeUnknown, {
        copilot: {
          ...model(totals.namedLikeUnknown),
          models: {
            "unknown-model": model(totals.namedLikeUnknown - 2),
            "fake-api": model(2),
          },
        },
      });

      // Exactly what mergeClientBreakdownsWithRegressionGuard() returns for a
      // stored {claude-sonnet-4: 3,100,000} merged with an incoming
      // {unknown: 3,099,998, fake-api: 2} under costIsComplete:false —
      // applyCostCompleteness() unions the two maps and keeps the incoming
      // scalar, so Sigma(models) is 6,200,000 against a scalar of 3,100,000.
      // The day scalar still equals Sigma(client scalars), so nothing upstream
      // of the model map looks wrong.
      await day("overNested", "2026-01-01", totals.overNested, {
        copilot: {
          ...model(totals.overNested),
          models: {
            "claude-sonnet-4": model(totals.overNested),
            unknown: model(totals.overNested - 2),
            "fake-api": model(2),
          },
        },
      });

      // Same merge, with the dropped model carrying a real name. Nothing here
      // is keyed `unknown`, so there is no unnamed cell for a clamp to charge
      // the excess against — the whole 3,200,000 scalar reads as attributed
      // under GREATEST(LEAST(named, scalar - unnamed), 0) while 2 tokens are
      // all the incoming submission actually accounted for.
      await day("overNestedNamed", "2026-01-01", totals.overNestedNamed, {
        copilot: {
          ...model(totals.overNestedNamed),
          models: {
            "claude-sonnet-4": model(totals.overNestedNamed),
            "claude-opus-4": model(totals.overNestedNamed - 2),
            "fake-api": model(2),
          },
        },
      });

      // The cheapest producible over-nesting: a stored {claude-sonnet-4: N}
      // merged with an incoming legacy-partial {fake-api: 2} under
      // costIsComplete:false. Sigma(models) is N + 2 against a scalar of N —
      // over by two tokens, with no unnamed cell and no second large cell. The
      // incoming half is the `partial` persona's own shape, which is the shape
      // the Codex finding was written about.
      await day("overNestedAllNamed", "2026-01-01", totals.overNestedAllNamed, {
        copilot: {
          ...model(totals.overNestedAllNamed),
          models: {
            "claude-sonnet-4": model(totals.overNestedAllNamed),
            "fake-api": model(2),
          },
        },
      });

      // The contradiction in an entry carrying no tokens of its own. The
      // account's whole scalar sits on a well-formed `claude` entry, so
      // attributed_tokens reaches total_tokens on that entry alone; the
      // `copilot` entry adds 0 to the attributed sum AND 0 to the day scalar
      // that total_tokens is built from, so no arithmetic comparison between
      // the two can register it. Its map still claims 2 slop tokens that
      // nothing in the submission backs.
      //
      // Reachable through applyReplaceLayouts() (submit/route.ts), which calls
      // applyCostCompleteness() directly and so bypasses the token-decrease
      // regression guard that would otherwise keep the larger stored entry;
      // applyCostCompleteness() returns {...next, models: union} — incoming
      // scalar, unioned map — and NonNegativeIntegerSchema permits tokens 0.
      await day(
        "overNestedZeroScalar",
        "2026-01-01",
        totals.overNestedZeroScalar,
        {
          claude: {
            ...model(totals.overNestedZeroScalar),
            models: {
              "claude-sonnet-4": model(totals.overNestedZeroScalar),
            },
          },
          copilot: { ...model(0), models: { "fake-api": model(2) } },
        }
      );

      // Identical, with the `tokens` key deleted rather than set to 0: the
      // query reads the scalar as COALESCE((value->>'tokens')::numeric, 0), so
      // an absent key and a 0 are the same number and must fail closed alike.
      const noScalarCopilot: Record<string, unknown> = { ...model(0) };
      delete noScalarCopilot.tokens;
      await day("overNestedNoScalar", "2026-01-01", totals.overNestedNoScalar, {
        claude: {
          ...model(totals.overNestedNoScalar),
          models: { "claude-sonnet-4": model(totals.overNestedNoScalar) },
        },
        copilot: { ...noScalarCopilot, models: { "fake-api": model(2) } },
      });

      // The eead1190..e4fd6668 shape with the `unknown` that
      // normalizeSubmissionData() writes for a blank modelId: the scalar
      // remainder is real but no model claims it.
      await day("unknownModelId", "2026-01-01", totals.unknownModelId, {
        claude: {
          ...model(totals.unknownModelId),
          modelId: "unknown",
          models: { "fake-api": model(2) },
        },
      });

      // 1,000 tokens under `unknown` and 2 under the slop model, inside an
      // entry whose scalar still leaves 2,498,998 over for its own modelId.
      // Both the clamp and the named-cell filter are active here and they
      // disagree: only the filter keeps the 1,000 out of the attributed sum.
      await day(
        "remainderPlusUnknown",
        "2026-01-01",
        totals.remainderPlusUnknown,
        {
          claude: {
            ...model(totals.remainderPlusUnknown),
            modelId: "claude-sonnet-4",
            models: { unknown: model(1_000), "fake-api": model(2) },
          },
        }
      );
    });
  });

  afterAll(async () => {
    if (!fixtureDb) return;

    for (const persona of personas) {
      await fixtureDb`DELETE FROM users WHERE id = ${ids[persona].userId}`;
    }
    await fixtureDb.end();
  });

  async function candidateFor(persona: Persona) {
    const candidates = await getModerationCandidates();
    const candidate = candidates.find(
      (row) => row.username === ids[persona].username
    );
    expect(candidate, `${persona} missing from the review queue`).toBeDefined();
    return candidate!;
  }

  interface UnknowableLogPayload {
    event: string;
    knowable: number;
    unknowable: number;
    byReason: Record<string, number>;
    /**
     * Decimal strings: the sums are exact bigints in the emitter, and
     * JSON.stringify refuses a bigint, so they travel as their digits.
     */
    unattributedTokens: string;
    unknowableTotalTokens: string;
    unattributedHistogram: Record<string, number>;
  }

  function parseUnknowableWarnings(calls: unknown[][]): UnknowableLogPayload[] {
    return calls
      .map((call) => String(call[0]))
      .filter((message) => message.includes(UNKNOWABLE_EVENT))
      .map((message) => {
        // getDb() memoizes its pool on globalThis, so other log lines from
        // this process can share the warn spy; match the payload, not the
        // whole line.
        const match = message.match(/\[moderation\] (\{.*\})$/);
        expect(match, `unparseable moderation log line: ${message}`).not.toBeNull();
        return JSON.parse(match![1]) as UnknowableLogPayload;
      });
  }

  it("credits a partial model map's scalar remainder to the entry's own modelId", async () => {
    const candidate = await candidateFor("partial");

    // Not 2 (the nested map alone) and not 1,200,002 (the remainder counted on
    // top of a nested sum that already contains it).
    expect(candidate.slopTokens).toBe(totals.partial);
    expect(
      candidate.signals.find((signal) => signal.key === "slopModelName")?.weight
    ).toBe(35);
  });

  it("credits the remainder to the named modelId rather than to the matching model", async () => {
    const candidate = await candidateFor("blend");

    // Half the entry sits in the nested `fake-api` cell; the remainder is
    // claude-sonnet-4's, so the share is 0.5 rather than 1.
    expect(candidate.slopTokens).toBe(totals.blend / 2);
    expect(
      candidate.signals.find((signal) => signal.key === "slopModelName")?.weight
    ).toBeCloseTo(17.5, 10);
  });

  it("reports unknown attribution when a remainder belongs to no named model", async () => {
    const candidate = await candidateFor("unclaimed");

    expect(candidate.slopTokens).toBeNull();
    expect(
      candidate.signals.find((signal) => signal.key === "slopModelName")?.weight
    ).toBe(35);
  });

  it("attributes a pre-models legacy entry entirely to its modelId", async () => {
    const candidate = await candidateFor("legacy");

    expect(candidate.slopTokens).toBe(totals.legacy);
    expect(
      candidate.signals.find((signal) => signal.key === "slopModelName")?.weight
    ).toBe(35);
  });

  it("reports unknown attribution when only some daily rows carry a breakdown", async () => {
    const candidate = await candidateFor("mixed");

    // 2 attributed tokens against a 1,600,000 total would scale the weight to
    // zero and drop the account out of the queue entirely.
    expect(candidate.slopTokens).toBeNull();
    expect(
      candidate.signals.find((signal) => signal.key === "slopModelName")?.weight
    ).toBe(35);
  });

  it("still scales a fully attributed config artifact out of the queue", async () => {
    const candidate = await candidateFor("artifact");

    expect(candidate.slopTokens).toBe(2);
    expect(candidate.signals.map((signal) => signal.key)).not.toContain(
      "slopModelName"
    );
  });

  it("keeps full weight when every token is booked under the matching model", async () => {
    const candidate = await candidateFor("wholly");

    expect(candidate.slopTokens).toBe(totals.wholly);
    expect(
      candidate.signals.find((signal) => signal.key === "slopModelName")?.weight
    ).toBe(35);
  });

  it("reports unknown attribution when the remainder sits in the `unknown` cell", async () => {
    const candidate = await candidateFor("unknownBucket");

    // 2 slop tokens against 2,100,000 would scale the weight to 0.00003 and
    // Math.round it away, dropping the account out of the queue.
    expect(candidate.slopTokens).toBeNull();
    expect(
      candidate.signals.find((signal) => signal.key === "slopModelName")?.weight
    ).toBe(35);
  });

  it("reports unknown attribution when the remainder sits in a debris cell", async () => {
    const candidate = await candidateFor("debrisBucket");

    expect(candidate.slopTokens).toBeNull();
    expect(
      candidate.signals.find((signal) => signal.key === "slopModelName")?.weight
    ).toBe(35);
  });

  it("reports unknown attribution when a model map outruns the entry's own scalar", async () => {
    const candidate = await candidateFor("overNested");

    // Sigma(models) is 6,200,000 against a 3,100,000 scalar, so the named cells
    // alone already sum past the account's total and the completeness gate
    // waves the row through unless the named sum is clamped by what the
    // `unknown` cell has already consumed. Counting only the named cells gives
    // 3,100,002 >= 3,100,000 -> slopTokens 2 -> weight 35 * 2/3,100,000, which
    // Math.round drops: the account leaves the queue while 3,099,998 tokens sit
    // unclaimed.
    expect(candidate.slopTokens).toBeNull();
    expect(
      candidate.signals.find((signal) => signal.key === "slopModelName")?.weight
    ).toBe(35);
  });

  it("reports unknown attribution when the over-nesting sits under a named key", async () => {
    const candidate = await candidateFor("overNestedNamed");

    // Identical to `overNested` except the dropped model has a real name. Any
    // clamp of the named sum has a ceiling of the entry's own scalar, and the
    // gate's threshold is total_tokens = the sum of those same scalars, so
    // clamping lands this row exactly ON the gate rather than below it:
    // attributed 3,200,000 >= total 3,200,000 -> slopTokens 2 -> weight
    // 35 * 2/3,200,000, which Math.round drops. The entry has to attribute
    // nothing, not a clamped amount.
    expect(candidate.slopTokens).toBeNull();
    expect(
      candidate.signals.find((signal) => signal.key === "slopModelName")?.weight
    ).toBe(35);
  });

  it("reports unknown attribution for a map over-nested by two tokens", async () => {
    const candidate = await candidateFor("overNestedAllNamed");

    // Sigma(models) is 4,100,002 against a 4,100,000 scalar. No `unknown` cell
    // exists anywhere in the payload, so this is the shape that needs no
    // unnamed key at all: a stored named map plus an incoming legacy-partial
    // entry. Under the clamp it read as fully attributed with slopTokens 2 and
    // the slopModelName signal absent, which drops the account out of
    // rankCandidates() entirely unless it is already hidden.
    expect(candidate.slopTokens).toBeNull();
    expect(
      candidate.signals.find((signal) => signal.key === "slopModelName")?.weight
    ).toBe(35);
  });

  it("reports unknown attribution when a contradictory entry carries no tokens of its own", async () => {
    const candidate = await candidateFor("overNestedZeroScalar");

    // The one case no arithmetic gate reaches. Attribution from the `claude`
    // entry alone is 5,100,000 >= total 5,100,000, and the `copilot` entry
    // moves neither number: zeroing its attribution subtracts nothing, and its
    // 0 scalar added nothing to the day total that total_tokens was summed
    // from. Measured before the over_nested flag existed: slopTokens 2, weight
    // 35 * 2/5,100,000, Math.round -> 0, slopModelName absent from the row.
    // Only an explicit per-entry flag fails this closed, which is why deleting
    // `has_over_nested_entry` from the gate must turn this test red.
    expect(candidate.slopTokens).toBeNull();
    expect(
      candidate.signals.find((signal) => signal.key === "slopModelName")?.weight
    ).toBe(35);
  });

  it("reports unknown attribution when the contradictory entry has no tokens key at all", async () => {
    const candidate = await candidateFor("overNestedNoScalar");

    // Same shape with the key absent instead of 0. COALESCE collapses the two
    // spellings to the same scalar, so this pins that an entry can go missing
    // its scalar entirely and still be caught.
    expect(candidate.slopTokens).toBeNull();
    expect(
      candidate.signals.find((signal) => signal.key === "slopModelName")?.weight
    ).toBe(35);
  });

  it("does not credit a remainder to a client-level modelId that names nothing", async () => {
    const candidate = await candidateFor("unknownModelId");

    // Without the UNNAMED_MODEL_REGEX guard on `modelId`, the 2,399,998-token
    // remainder (2,400,000 scalar less the 2 the map accounts for) is credited
    // to the literal string "unknown", attribution reads as complete at
    // 2,400,000, and slopTokens 2 against 2,400,000 rounds the weight away.
    expect(candidate.slopTokens).toBeNull();
    expect(
      candidate.signals.find((signal) => signal.key === "slopModelName")?.weight
    ).toBe(35);
  });

  it("does not attribute an unnamed cell that sits alongside a scalar remainder", async () => {
    const candidate = await candidateFor("remainderPlusUnknown");

    // Attribution reaches 2,499,000 of 2,500,000: the 2 nested slop tokens plus
    // the 2,498,998 remainder claude-sonnet-4 claims, with the 1,000 `unknown`
    // tokens left out. Dropping the UNNAMED_MODEL_REGEX filter on the nested
    // named sum lands on exactly 2,500,000 and the gate passes — the clamp
    // cannot catch it, because this map fits inside its own scalar.
    expect(candidate.slopTokens).toBeNull();
    expect(
      candidate.signals.find((signal) => signal.key === "slopModelName")?.weight
    ).toBe(35);
  });

  it("still counts a model whose name merely contains the sentinel", async () => {
    const candidate = await candidateFor("namedLikeUnknown");

    // `unknown-model` is a name, not the `unknown` bucket: dropping the
    // anchors from UNNAMED_MODEL_REGEX would turn this into null and pin a
    // real account at full weight forever.
    expect(candidate.slopTokens).toBe(2);
    expect(candidate.signals.map((signal) => signal.key)).not.toContain(
      "slopModelName"
    );
  });

  // Breadth telemetry for the fail-closed path. The assertions above prove
  // WHERE the gate fails; these prove the instrumentation reports exactly
  // that set and nothing else, since a breadth metric that fires on knowable
  // rows (or stays silent on unknowable ones) answers the wrong question.
  // The queries re-run against the same fixture rows inside each test; the
  // only moving part is the warn spy.
  it("emits one structured line per invocation covering exactly the fail-closed set", async () => {
    const warnSpy = vi.spyOn(console, "warn").mockImplementation(() => {});
    try {
      await getModerationCandidates();
      const payloads = parseUnknowableWarnings(warnSpy.mock.calls);
      expect(payloads).toHaveLength(1);
      const payload = payloads[0];

      expect(payload.event).toBe(UNKNOWABLE_EVENT);

      // Exactly the personas whose slopTokens is NULL, classified by the gate
      // clause that failed. Deleting has_over_nested_entry from the gate moves
      // the five over-nested personas into unattributed_tokens (their
      // attributed sum then falls short of the total), so this map goes red
      // the moment the instrumentation and the gate diverge.
      expect(payload.byReason).toEqual({
        missing_breakdown: 1, // mixed
        over_nested: 5, // the five overNested* personas
        // unclaimed, unknownBucket, debrisBucket, unknownModelId,
        // remainderPlusUnknown.
        unattributed_tokens: 5,
        unknown: 0,
      });
      expect(payload.unknowable).toBe(11);
      // partial, blend, legacy, artifact, wholly, namedLikeUnknown.
      expect(payload.knowable).toBe(6);

      const expectedUnattributed: Partial<Record<Persona, number>> = {
        unclaimed: totals.unclaimed - 2,
        mixed: totals.mixed - 2,
        // An over-nested entry attributes NOTHING — not the clamped scalar —
        // so the three all-scalar personas miss their whole totals...
        overNested: totals.overNested,
        overNestedNamed: totals.overNestedNamed,
        overNestedAllNamed: totals.overNestedAllNamed,
        // ...while the zero/no-scalar pair still attributes the well-formed
        // claude entry beside the contradictory one, so nothing is missing.
        overNestedZeroScalar: 0,
        overNestedNoScalar: 0,
        unknownModelId: totals.unknownModelId - 2,
        remainderPlusUnknown: 1_000,
        unknownBucket: totals.unknownBucket - 2,
        debrisBucket: totals.debrisBucket - 2,
      };
      expect(payload.unattributedTokens).toBe(
        String(Object.values(expectedUnattributed).reduce((sum, n) => sum + n, 0))
      );
      expect(payload.unknowableTotalTokens).toBe(
        String(
          (Object.keys(expectedUnattributed) as Persona[]).reduce(
            (sum, persona) => sum + totals[persona],
            0
          )
        )
      );

      const bucketAt = (power: number) =>
        payload.unattributedHistogram[String(power * UNKNOWABLE_BUCKET_WIDTH)];
      // The two zero-gap personas and remainderPlusUnknown's 1,000 cross no
      // boundary, so 1M holds eight of the eleven — the histogram is also
      // where "unknowable but fully covered" (overNestedZeroScalar/NoScalar)
      // separates from "tokens actually missing", which the flat counts
      // cannot say.
      expect(bucketAt(1)).toBe(8);
      // Only the 999,998 and 1,599,998 gaps (unclaimed, mixed) stop below
      // 2M; the six gaps at or above 2,099,998 all cross it.
      expect(bucketAt(2)).toBe(6);
      // 2.1M-3.2M stop at 2M; only the 4,100,000 gap crosses 4M.
      expect(bucketAt(4)).toBe(1);
      expect(bucketAt(8)).toBe(0);
    } finally {
      warnSpy.mockRestore();
    }
  });

  it("does not count a slop-less account as unknowable even with a null slopTokens", async () => {
    // A submissions row whose models_used holds no slop name never enters the
    // attribution CTEs; its slop_tokens is NULL by scoping, not by gate
    // failure. If the breadth counter read nulls off the whole queue, this
    // persona would move it and the metric would answer the wrong question.
    const suffix = randomUUID().replaceAll("-", "").slice(0, 8);
    const sloplessId = randomUUID();
    await fixtureDb.begin(async (sql) => {
      await sql`
        INSERT INTO users (id, github_id, username, leaderboard_hidden)
        VALUES (
          ${sloplessId},
          ${githubIdBase - 1_000_000 - Number.parseInt(suffix.slice(0, 6), 16)},
          ${`mod_slopless_${suffix}`},
          true
        )
      `;
      await sql`
        INSERT INTO submissions (
          id, user_id, total_tokens, total_cost, input_tokens, output_tokens,
          date_start, date_end, sources_used, models_used
        )
        VALUES (
          ${randomUUID()}, ${sloplessId}, 42_000_000, 0, 42_000_000, 0,
          '2026-01-01', '2026-01-02', ARRAY['claude'], ARRAY['claude-sonnet-4']
        )
      `;
    });

    const warnSpy = vi.spyOn(console, "warn").mockImplementation(() => {});
    try {
      await getModerationCandidates();
      const payloads = parseUnknowableWarnings(warnSpy.mock.calls);
      expect(payloads).toHaveLength(1);
      // Same counts as the previous test: the slop-less row moved nothing.
      expect(payloads[0].unknowable).toBe(11);
      expect(payloads[0].knowable).toBe(6);
    } finally {
      warnSpy.mockRestore();
    }

    await fixtureDb`DELETE FROM users WHERE id = ${sloplessId}`;
  });

  it("carries a one-token gate shortfall above 2^53 exactly through the driver", async () => {
    // The SQL gate can fail `attributed_tokens >= total_tokens` by exactly
    // one token above 2^53 — 9,007,199,254,740,995 < 9,007,199,254,740,996 —
    // while both operands collapse to the SAME Number on the way out of
    // postgres-js. Only the real driver strings can prove the classifier
    // still reports `unattributed_tokens` (not `unknown`) and the telemetry
    // the true one-token gap. Deltas against a same-fixture baseline keep
    // the assertions independent of the shared personas.
    const total = "9007199254740996"; // 2^53 + 4
    const attributedNamed = "9007199254740993"; // + fake-api's 2 = total - 1
    const suffix = randomUUID().replaceAll("-", "").slice(0, 8);
    const bigUserId = randomUUID();
    const bigSubmissionId = randomUUID();
    const bigDeviceId = randomUUID();

    const baselineSpy = vi.spyOn(console, "warn").mockImplementation(() => {});
    let baseline: UnknowableLogPayload;
    try {
      await getModerationCandidates();
      const payloads = parseUnknowableWarnings(baselineSpy.mock.calls);
      expect(payloads).toHaveLength(1);
      baseline = payloads[0];
    } finally {
      baselineSpy.mockRestore();
    }

    // JSON built by hand: routing the token counts through a JS object would
    // round 9,007,199,254,740,995 at JSON.stringify time, which is the exact
    // loss this test exists to rule out. The ::text::jsonb chain pins the
    // parameter as text so postgres-js does not double-encode it — a bare
    // ::jsonb cast makes Postgres infer a jsonb parameter, which the driver
    // JSON-stringifies into a jsonb STRING (jsonb_typeof = 'string').
    const cell = (tokens: string) =>
      `{"tokens":${tokens},"cost":0,"input":${tokens},"output":0,"cacheRead":0,"cacheWrite":0,"reasoning":0,"messages":1}`;
    const breakdown = `{"claude":{"tokens":${total},"cost":0,"input":${total},"output":0,"cacheRead":0,"cacheWrite":0,"reasoning":0,"messages":1,"models":{"claude-sonnet-4":${cell(
      attributedNamed
    )},"fake-api":${cell("2")}}}}`;

    await fixtureDb.begin(async (sql) => {
      await sql`
        INSERT INTO users (id, github_id, username, leaderboard_hidden)
        VALUES (
          ${bigUserId},
          ${githubIdBase - 2_000_000 - Number.parseInt(suffix.slice(0, 6), 16)},
          ${`mod_bigint_${suffix}`},
          true
        )
      `;
      await sql`
        INSERT INTO submissions (
          id, user_id, total_tokens, total_cost, input_tokens, output_tokens,
          date_start, date_end, sources_used, models_used
        )
        VALUES (
          ${bigSubmissionId}, ${bigUserId}, ${total}, 0, ${total}, 0,
          '2026-01-01', '2026-01-02', ARRAY['claude'],
          ARRAY['claude-sonnet-4', 'fake-api']
        )
      `;
      await sql`
        INSERT INTO submitted_devices (id, user_id, device_key)
        VALUES (${bigDeviceId}, ${bigUserId}, ${`mod-bigint-${suffix}-device`})
      `;
      await sql`
        INSERT INTO daily_breakdown (
          submission_id, submitted_device_id, date, tokens, cost,
          input_tokens, output_tokens, source_breakdown
        )
        VALUES (
          ${bigSubmissionId}, ${bigDeviceId}, '2026-01-01', ${total}, 0,
          ${total}, 0, ${breakdown}::text::jsonb
        )
      `;
    });

    try {
      const warnSpy = vi.spyOn(console, "warn").mockImplementation(() => {});
      try {
        await getModerationCandidates();
        const payloads = parseUnknowableWarnings(warnSpy.mock.calls);
        expect(payloads).toHaveLength(1);
        const payload = payloads[0];

        expect(payload.knowable).toBe(baseline.knowable);
        expect(payload.unknowable).toBe(baseline.unknowable + 1);
        expect(payload.byReason).toEqual({
          ...baseline.byReason,
          unattributed_tokens: baseline.byReason.unattributed_tokens + 1,
        });
        // `unknown` is the clause a Number pipeline reports for this row.
        expect(payload.byReason.unknown).toBe(baseline.byReason.unknown);
        expect(
          BigInt(payload.unattributedTokens) -
            BigInt(baseline.unattributedTokens)
        ).toBe(1n);
        expect(
          BigInt(payload.unknowableTotalTokens) -
            BigInt(baseline.unknowableTotalTokens)
        ).toBe(BigInt(total));
        // A one-token gap crosses no bucket boundary.
        expect(payload.unattributedHistogram).toEqual(
          baseline.unattributedHistogram
        );
      } finally {
        warnSpy.mockRestore();
      }
    } finally {
      await fixtureDb`DELETE FROM users WHERE id = ${bigUserId}`;
    }
  });

  it("still emits the line when every slop-matched candidate is knowable", async () => {
    // The breadth metric is a fraction, and a window where nothing failed
    // must contribute its denominator: a suppressed line is indistinguishable
    // from a window that was never measured, and aggregating only failure
    // lines can never produce a zero rate. Runs last because it removes the
    // unknowable personas for good — afterAll's per-persona DELETE is a no-op
    // for rows already gone.
    const unknowablePersonas: Persona[] = [
      "unclaimed",
      "mixed",
      "unknownBucket",
      "debrisBucket",
      "unknownModelId",
      "remainderPlusUnknown",
      "overNested",
      "overNestedNamed",
      "overNestedAllNamed",
      "overNestedZeroScalar",
      "overNestedNoScalar",
    ];
    for (const persona of unknowablePersonas) {
      await fixtureDb`DELETE FROM users WHERE id = ${ids[persona].userId}`;
    }

    const warnSpy = vi.spyOn(console, "warn").mockImplementation(() => {});
    try {
      await getModerationCandidates();
      const payloads = parseUnknowableWarnings(warnSpy.mock.calls);
      expect(payloads).toHaveLength(1);
      const payload = payloads[0];
      // partial, blend, legacy, artifact, wholly, namedLikeUnknown remain.
      expect(payload.knowable).toBe(6);
      expect(payload.unknowable).toBe(0);
      expect(payload.byReason).toEqual({
        missing_breakdown: 0,
        over_nested: 0,
        unattributed_tokens: 0,
        unknown: 0,
      });
      expect(payload.unattributedTokens).toBe("0");
      expect(payload.unknowableTotalTokens).toBe("0");
      expect(
        Object.values(payload.unattributedHistogram).every(
          (count) => count === 0
        )
      ).toBe(true);
    } finally {
      warnSpy.mockRestore();
    }
  });
});
