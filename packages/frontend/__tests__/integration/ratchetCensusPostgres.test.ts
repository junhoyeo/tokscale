import { randomUUID } from "node:crypto";
import postgres from "postgres";
import { afterAll, beforeAll, describe, expect, it } from "vitest";

import { getRatchetCensusReport } from "@/lib/reconciliation/ratchetCensus";

const integrationEnabled =
  process.env.RATCHET_CENSUS_DB_INTEGRATION === "1";
const describeWithPostgres = integrationEnabled ? describe : describe.skip;

describeWithPostgres("ratchet census PostgreSQL integration", () => {
  const databaseUrl = process.env.DATABASE_URL;
  const userIds = [randomUUID(), randomUUID(), randomUUID()];
  const submissionIds = [randomUUID(), randomUUID(), randomUUID()];
  const deviceIds = [randomUUID(), randomUUID(), randomUUID()];
  const fixtureSuffix = randomUUID().replaceAll("-", "").slice(0, 8);
  const usernames = {
    severe: `census_severe_${fixtureSuffix}`,
    warming: `census_warming_${fixtureSuffix}`,
    pending: `census_pending_${fixtureSuffix}`,
  };
  const largeTokenTotal = BigInt("9007199254740993");
  const highwaterTotal = BigInt("3000000000000000");
  const githubIdBase =
    -1_900_000_000 + Number.parseInt(fixtureSuffix.slice(0, 6), 16);

  let fixtureDb: ReturnType<typeof postgres>;

  beforeAll(async () => {
    if (!databaseUrl) {
      throw new Error(
        "DATABASE_URL is required when RATCHET_CENSUS_DB_INTEGRATION=1"
      );
    }

    fixtureDb = postgres(databaseUrl, { max: 1, prepare: false });

    await fixtureDb.begin(async (sql) => {
      await sql`
        INSERT INTO users (id, github_id, username)
        VALUES
          (${userIds[0]}, ${githubIdBase}, ${usernames.severe}),
          (${userIds[1]}, ${githubIdBase + 1}, ${usernames.warming}),
          (${userIds[2]}, ${githubIdBase + 2}, ${usernames.pending})
      `;

      await sql`
        INSERT INTO submissions (
          id, user_id, total_tokens, total_cost, input_tokens, output_tokens,
          date_start, date_end, sources_used, models_used, cli_version
        )
        VALUES
          (
            ${submissionIds[0]}, ${userIds[0]}, ${largeTokenTotal.toString()},
            0, ${largeTokenTotal.toString()}, 0, '2026-01-01', '2026-01-05',
            ARRAY['claude'], ARRAY['fixture-model'], '4.12.0'
          ),
          (
            ${submissionIds[1]}, ${userIds[1]}, 100, 0, 100, 0,
            '2026-01-01', '2026-01-01', ARRAY['claude'],
            ARRAY['fixture-model'], '4.11.0'
          ),
          (
            ${submissionIds[2]}, ${userIds[2]}, 90, 0, 90, 0,
            '2026-01-01', '2026-01-01', ARRAY['codex'],
            ARRAY['fixture-model'], '4.10.0'
          )
      `;

      await sql`
        INSERT INTO submitted_devices (id, user_id, device_key)
        VALUES
          (${deviceIds[0]}, ${userIds[0]}, 'census-severe-device'),
          (${deviceIds[1]}, ${userIds[1]}, 'census-warming-device'),
          (${deviceIds[2]}, ${userIds[2]}, 'census-pending-device')
      `;

      const observedRatios = [
        { date: "2026-01-01", guardedTokens: 50, reportedTokens: 100 },
        { date: "2026-01-02", guardedTokens: 100, reportedTokens: 100 },
        { date: "2026-01-03", guardedTokens: 110, reportedTokens: 100 },
        { date: "2026-01-04", guardedTokens: 150, reportedTokens: 100 },
        { date: "2026-01-05", guardedTokens: 300, reportedTokens: 100 },
      ];

      for (const observation of observedRatios) {
        await sql`
          INSERT INTO daily_breakdown (
            submission_id, submitted_device_id, date, tokens, cost,
            input_tokens, output_tokens, source_breakdown
          )
          VALUES (
            ${submissionIds[0]}, ${deviceIds[0]}, ${observation.date},
            ${observation.guardedTokens}, 0, ${observation.guardedTokens}, 0,
            ${sql.json({
              claude: {
                tokens: observation.guardedTokens,
                provenance: { origin: "cli" },
              },
            })}
          )
        `;
        await sql`
          INSERT INTO daily_breakdown_reported (
            submitted_device_id, date, client, tokens, cost, input, output, origin
          )
          VALUES (
            ${deviceIds[0]}, ${observation.date}, 'claude',
            ${observation.reportedTokens}, 0, ${observation.reportedTokens}, 0,
            'cli'
          )
        `;
      }

      await sql`
        INSERT INTO daily_breakdown (
          submission_id, submitted_device_id, date, tokens, cost,
          input_tokens, output_tokens, source_breakdown
        )
        VALUES
          (
            ${submissionIds[1]}, ${deviceIds[1]}, '2026-01-01', 100, 0, 100, 0,
            ${sql.json({
              claude: {
                tokens: 100,
                provenance: { origin: "backfill" },
              },
            })}
          ),
          (
            ${submissionIds[2]}, ${deviceIds[2]}, '2026-01-01', 90, 0, 90, 0,
            ${sql.json({
              codex: { tokens: 90, provenance: { origin: "cli" } },
            })}
          )
      `;

      await sql`
        INSERT INTO daily_breakdown_reported (
          submitted_device_id, date, client, tokens, cost, input, output, origin
        )
        VALUES (${deviceIds[0]}, '2026-01-01', 'omitted-client', 100, 0, 100, 0, 'cli')
      `;

      await sql`
        INSERT INTO submitted_device_client_totals (
          submitted_device_id, client, origin, bucket_width, bucket_key,
          tokens_highwater, cost_highwater
        )
        VALUES
          (
            ${deviceIds[0]}, 'claude', 'cli', 'month', '2026-01',
            ${highwaterTotal.toString()}, 0
          ),
          (${deviceIds[2]}, 'codex', 'cli', 'month', '2026-01', 90, 0)
      `;

      await sql`
        INSERT INTO ratchet_census_work (
          submission_id, submitted_device_id, buckets
        )
        VALUES (${submissionIds[2]}, ${deviceIds[2]}, ${sql.json([])})
      `;
    });
  });

  afterAll(async () => {
    if (!fixtureDb) return;

    for (const userId of userIds) {
      await fixtureDb`DELETE FROM users WHERE id = ${userId}`;
    }
    await fixtureDb.end();
  });

  it("returns typed gate inputs from the migrated schema", async () => {
    const report = await getRatchetCensusReport({
      candidateLimit: 10,
      now: new Date("2026-08-09T12:00:00.000Z"),
    });

    expect(report.coverage).toEqual({
      totalUsers: 3,
      measuredUsers: 1,
      totalTokens: (largeTokenTotal + BigInt(190)).toString(),
      measuredTokens: largeTokenTotal.toString(),
      userCoverage: 0.333333,
      tokenCoverage: 1,
      pendingWorkItems: 1,
    });
    expect(report.divergenceBands).toEqual([
      { band: "pending", users: 1, tokens: "90" },
      { band: "warming", users: 1, tokens: "100" },
      {
        band: "severe",
        users: 1,
        tokens: largeTokenTotal.toString(),
      },
    ]);
    expect(report.observedCells).toEqual({
      comparableCells: 5,
      under: 1,
      clean: 1,
      mild: 1,
      clear: 1,
      severe: 1,
      maxRatio: 3,
    });
    expect(report.segments.byOrigin).toEqual([
      {
        key: "backfill",
        expectedCells: 1,
        measuredCells: 0,
        cellCoverage: 0,
      },
      {
        key: "cli",
        expectedCells: 2,
        measuredCells: 2,
        cellCoverage: 1,
      },
    ]);
    expect(report.segments.byClient).toEqual([
      {
        key: "claude",
        expectedCells: 2,
        measuredCells: 1,
        cellCoverage: 0.5,
      },
      {
        key: "codex",
        expectedCells: 1,
        measuredCells: 1,
        cellCoverage: 1,
      },
    ]);
    expect(report.candidates).toHaveLength(1);
    expect(report.candidates[0]).toMatchObject({
      username: usernames.severe,
      totalTokens: largeTokenTotal.toString(),
      highwaterTokens: highwaterTotal.toString(),
      band: "severe",
      expectedCells: 1,
      measuredCells: 1,
      cliVersion: "4.12.0",
      deviceCount: 1,
    });
    expect(report.candidates[0].ratio).toBeCloseTo(
      Number(largeTokenTotal) / Number(highwaterTotal),
      12
    );
    expect(report.generatedAt).toBe("2026-08-09T12:00:00.000Z");
  });
});
