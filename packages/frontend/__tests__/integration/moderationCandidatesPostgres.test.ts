import { randomUUID } from "node:crypto";
import postgres from "postgres";
import { afterAll, beforeAll, describe, expect, it } from "vitest";

import { getModerationCandidates } from "@/lib/moderation/candidates";

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
 * than dropped, that the remainder is not also counted inside the nested
 * per-model sum, and that anything short of full attribution comes back NULL
 * so the scorer falls back to the full fixed weight — is only observable by
 * running it. Each persona below changes an assertion here when the
 * corresponding piece of SQL is altered.
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
});
