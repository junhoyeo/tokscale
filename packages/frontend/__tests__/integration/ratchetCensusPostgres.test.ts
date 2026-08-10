import { randomUUID } from "node:crypto";
import postgres from "postgres";
import { afterAll, beforeAll, describe, expect, it } from "vitest";

import { getRatchetCensusReport } from "@/lib/reconciliation/ratchetCensus";

const integrationEnabled =
  process.env.RATCHET_CENSUS_DB_INTEGRATION === "1";
const describeWithPostgres = integrationEnabled ? describe : describe.skip;

/**
 * Executes the real census SQL against a migrated PostgreSQL instance.
 *
 * The unit suite mocks `db.execute`, so it can only prove the Zod contract and
 * that certain substrings appear in the query. Everything the report actually
 * promises — bigint-preserving `::text` casts, 6-digit coverage rounding, the
 * user and cell band cutoffs, and the origin/bucket-key strictness of the
 * coverage join — is only observable by running the statement. This fixture is
 * built so that each of those properties has at least one row that changes the
 * asserted output when the corresponding SQL is altered.
 *
 * The fixture assumes it owns the database (it asserts site-wide totals), which
 * is why it runs against the throwaway CI PostgreSQL service rather than in the
 * default `vitest run` suite.
 */
describeWithPostgres("ratchet census PostgreSQL integration", () => {
  const databaseUrl = process.env.DATABASE_URL;
  const fixtureSuffix = randomUUID().replaceAll("-", "").slice(0, 8);

  // Personas, one user + submission + device each. `cliVersion` values are
  // zero-padded so the `ORDER BY users DESC, cli_version` tiebreak is a plain
  // lexicographic sort.
  const personas = [
    "zero",
    "severe",
    "warming",
    "pending",
    "under",
    "clean",
    "clear",
    "partial",
  ] as const;
  type Persona = (typeof personas)[number];

  const ids = Object.fromEntries(
    personas.map((persona) => [
      persona,
      {
        userId: randomUUID(),
        submissionId: randomUUID(),
        deviceId: randomUUID(),
        username: `census_${persona}_${fixtureSuffix}`,
      },
    ])
  ) as Record<
    Persona,
    { userId: string; submissionId: string; deviceId: string; username: string }
  >;

  // Both totals are deliberately UNREPRESENTABLE as IEEE-754 doubles, so a
  // dropped `::text` cast cannot pass by accident:
  //   9007199254740993  = 2^53 + 1   -> nearest double is ...992
  //   90071992547409937              -> nearest double is ...936
  // If the SQL emits these as JSON numbers rather than text, the driver's
  // JSON.parse silently rounds them and the assertions below fail.
  const largeTokenTotal = BigInt("90071992547409937");
  const highwaterTotal = BigInt("9007199254740993");
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
      const submissionTotals: Record<
        Persona,
        { tokens: string; cliVersion: string }
      > = {
        zero: { tokens: "0", cliVersion: "4.00.0" },
        under: { tokens: "900", cliVersion: "4.01.0" },
        clean: { tokens: "1000", cliVersion: "4.02.0" },
        clear: { tokens: "1500", cliVersion: "4.03.0" },
        partial: { tokens: "300", cliVersion: "4.04.0" },
        pending: { tokens: "90", cliVersion: "4.10.0" },
        warming: { tokens: "100", cliVersion: "4.11.0" },
        severe: { tokens: largeTokenTotal.toString(), cliVersion: "4.12.0" },
      };

      for (const [index, persona] of personas.entries()) {
        const { userId, submissionId, deviceId, username } = ids[persona];
        const { tokens, cliVersion } = submissionTotals[persona];

        await sql`
          INSERT INTO users (id, github_id, username)
          VALUES (${userId}, ${githubIdBase + index}, ${username})
        `;
        await sql`
          INSERT INTO submissions (
            id, user_id, total_tokens, total_cost, input_tokens, output_tokens,
            date_start, date_end, sources_used, models_used, cli_version
          )
          VALUES (
            ${submissionId}, ${userId}, ${tokens}, 0, ${tokens}, 0,
            '2026-01-01', '2026-03-01', ARRAY['claude'],
            ARRAY['fixture-model'], ${cliVersion}
          )
        `;
        await sql`
          INSERT INTO submitted_devices (id, user_id, device_key)
          VALUES (${deviceId}, ${userId}, ${`census-${persona}-device`})
        `;
      }

      /** One guarded daily row, i.e. one (device, client, origin, month) cell. */
      const guardedDay = (
        persona: Persona,
        date: string,
        client: string,
        origin: string,
        tokens: number
      ) =>
        sql`
          INSERT INTO daily_breakdown (
            submission_id, submitted_device_id, date, tokens, cost,
            input_tokens, output_tokens, source_breakdown
          )
          VALUES (
            ${ids[persona].submissionId}, ${ids[persona].deviceId}, ${date},
            ${tokens}, 0, ${tokens}, 0,
            ${sql.json({ [client]: { tokens, provenance: { origin } } })}
          )
        `;

      /** One per-device high-water row, i.e. one *measured* cell. */
      const highwater = (
        persona: Persona,
        client: string,
        origin: string,
        bucketKey: string,
        tokens: string
      ) =>
        sql`
          INSERT INTO submitted_device_client_totals (
            submitted_device_id, client, origin, bucket_width, bucket_key,
            tokens_highwater, cost_highwater
          )
          VALUES (
            ${ids[persona].deviceId}, ${client}, ${origin}, 'month',
            ${bucketKey}, ${tokens}, 0
          )
        `;

      // `severe`: five guarded days in one month collapse to a single expected
      // cell, while the paired reported rows give one observation per band.
      const observedRatios = [
        { date: "2026-01-01", guardedTokens: 50, reportedTokens: 100 }, // 0.50 under
        { date: "2026-01-02", guardedTokens: 100, reportedTokens: 100 }, // 1.00 clean
        { date: "2026-01-03", guardedTokens: 110, reportedTokens: 100 }, // 1.10 mild
        { date: "2026-01-04", guardedTokens: 150, reportedTokens: 100 }, // 1.50 clear
        { date: "2026-01-05", guardedTokens: 300, reportedTokens: 100 }, // 3.00 severe
      ];

      for (const observation of observedRatios) {
        await guardedDay(
          "severe",
          observation.date,
          "claude",
          "cli",
          observation.guardedTokens
        );
        await sql`
          INSERT INTO daily_breakdown_reported (
            submitted_device_id, date, client, tokens, cost, input, output, origin
          )
          VALUES (
            ${ids.severe.deviceId}, ${observation.date}, 'claude',
            ${observation.reportedTokens}, 0, ${observation.reportedTokens}, 0,
            'cli'
          )
        `;
      }

      // A reported client with no matching guarded entry. `source_breakdown ?
      // r.client` must exclude it rather than read the absence as zero tokens.
      await sql`
        INSERT INTO daily_breakdown_reported (
          submitted_device_id, date, client, tokens, cost, input, output, origin
        )
        VALUES (
          ${ids.severe.deviceId}, '2026-01-01', 'omitted-client', 100, 0, 100, 0, 'cli'
        )
      `;
      await highwater(
        "severe",
        "claude",
        "cli",
        "2026-01",
        highwaterTotal.toString()
      );

      // `zero`: a durable zero-valued high-water row is complete evidence,
      // not a missing measurement. The 0/0 comparison is clean and has no
      // reason to appear in the ranked repair candidates.
      await guardedDay("zero", "2026-01-01", "claude", "cli", 0);
      await highwater("zero", "claude", "cli", "2026-01", "0");

      // `warming`: a backfill-origin cell with no high-water row yet.
      await guardedDay("warming", "2026-01-01", "claude", "backfill", 100);

      // `pending`: fully covered, but durable census work is outstanding.
      await guardedDay("pending", "2026-01-01", "codex", "cli", 90);
      await highwater("pending", "codex", "cli", "2026-01", "90");
      await sql`
        INSERT INTO ratchet_census_work (
          submission_id, submitted_device_id, buckets
        )
        VALUES (
          ${ids.pending.submissionId}, ${ids.pending.deviceId}, ${sql.json([])}
        )
      `;

      // Measured users, one per divergence band below `severe`.
      await guardedDay("under", "2026-01-01", "claude", "cli", 900);
      await highwater("under", "claude", "cli", "2026-01", "1000"); // 900/1000
      await guardedDay("clean", "2026-01-01", "claude", "cli", 1000);
      await highwater("clean", "claude", "cli", "2026-01", "1000"); // 1000/1000
      await guardedDay("clear", "2026-01-01", "claude", "cli", 1500);
      await highwater("clear", "claude", "cli", "2026-01", "1000"); // 1500/1000

      // `partial`: three expected cells, only ONE of which may count as
      // measured. The other two have a high-water row that matches on every
      // column except one, so dropping either the origin or the bucket-key
      // predicate from the coverage join silently inflates coverage.
      await guardedDay("partial", "2026-01-01", "claude", "cli", 100);
      await highwater("partial", "claude", "cli", "2026-01", "500"); // matches
      await guardedDay("partial", "2026-02-01", "gemini", "cli", 100);
      await highwater("partial", "gemini", "cli", "2026-07", "100"); // wrong month
      await guardedDay("partial", "2026-03-01", "codex", "backfill", 100);
      await highwater("partial", "codex", "cli", "2026-03", "100"); // wrong origin
    });
  });

  afterAll(async () => {
    if (!fixtureDb) return;

    for (const persona of personas) {
      await fixtureDb`DELETE FROM users WHERE id = ${ids[persona].userId}`;
    }
    await fixtureDb.end();
  });

  it("reports site coverage with bigint-safe totals and 6-digit rounding", async () => {
    const report = await getRatchetCensusReport({
      candidateLimit: 10,
      now: new Date("2026-08-09T12:00:00.000Z"),
    });

    expect(report.coverage).toEqual({
      totalUsers: 8,
      // zero + severe + under + clean + clear; warming/partial are incomplete
      // and pending still has durable work.
      measuredUsers: 5,
      totalTokens: (largeTokenTotal + BigInt(3890)).toString(),
      measuredTokens: (largeTokenTotal + BigInt(3400)).toString(),
      // 5/8 is exact; client coverage below pins ROUND(..., 6).
      userCoverage: 0.625,
      tokenCoverage: 1,
      pendingWorkItems: 1,
    });
    expect(report.generatedAt).toBe("2026-08-09T12:00:00.000Z");
  });

  it("classifies every user divergence band", async () => {
    const report = await getRatchetCensusReport({ candidateLimit: 10 });

    expect(report.divergenceBands).toEqual([
      { band: "pending", users: 1, tokens: "90" },
      // `warming` collects both the user with no high-water row and the user
      // whose cells are only partially measured.
      { band: "warming", users: 2, tokens: "400" },
      { band: "under", users: 1, tokens: "900" },
      { band: "clean", users: 2, tokens: "1000" },
      { band: "clear", users: 1, tokens: "1500" },
      { band: "severe", users: 1, tokens: largeTokenTotal.toString() },
    ]);
  });

  it("classifies every observed cell band and ignores unmatched clients", async () => {
    const report = await getRatchetCensusReport({ candidateLimit: 10 });

    expect(report.observedCells).toEqual({
      // The `omitted-client` reported row has no guarded counterpart and is
      // excluded instead of being compared against zero.
      comparableCells: 5,
      under: 1,
      clean: 1,
      mild: 1,
      clear: 1,
      severe: 1,
      maxRatio: 3,
    });
  });

  it("keeps segment coverage origin-, client-, and bucket-key-aware", async () => {
    const report = await getRatchetCensusReport({ candidateLimit: 10 });

    expect(report.segments.byOrigin).toEqual([
      { key: "backfill", expectedCells: 2, measuredCells: 0, cellCoverage: 0 },
      // It drops if the coverage join stops matching on origin or bucket key,
      // because `partial` would then measure cells it must not.
      { key: "cli", expectedCells: 8, measuredCells: 7, cellCoverage: 0.875 },
    ]);
    expect(report.segments.byClient).toEqual([
      { key: "claude", expectedCells: 7, measuredCells: 6, cellCoverage: 0.857143 },
      { key: "codex", expectedCells: 2, measuredCells: 1, cellCoverage: 0.5 },
      { key: "gemini", expectedCells: 1, measuredCells: 0, cellCoverage: 0 },
    ]);
    // Ordered by `users DESC, cli_version`; every fixture user is alone on its
    // version, so this is a lexicographic sort. Asserting the exact token
    // strings is what pins the `::text` casts on the per-version totals.
    expect(report.segments.byCliVersion).toEqual([
      { cliVersion: "4.00.0", users: 1, measuredUsers: 1, totalTokens: "0", measuredTokens: "0" },
      { cliVersion: "4.01.0", users: 1, measuredUsers: 1, totalTokens: "900", measuredTokens: "900" },
      { cliVersion: "4.02.0", users: 1, measuredUsers: 1, totalTokens: "1000", measuredTokens: "1000" },
      { cliVersion: "4.03.0", users: 1, measuredUsers: 1, totalTokens: "1500", measuredTokens: "1500" },
      { cliVersion: "4.04.0", users: 1, measuredUsers: 0, totalTokens: "300", measuredTokens: "0" },
      { cliVersion: "4.10.0", users: 1, measuredUsers: 0, totalTokens: "90", measuredTokens: "0" },
      { cliVersion: "4.11.0", users: 1, measuredUsers: 0, totalTokens: "100", measuredTokens: "0" },
      {
        cliVersion: "4.12.0",
        users: 1,
        measuredUsers: 1,
        totalTokens: largeTokenTotal.toString(),
        measuredTokens: largeTokenTotal.toString(),
      },
    ]);
  });

  it("ranks measured candidates by divergence and preserves bigint columns", async () => {
    const report = await getRatchetCensusReport({ candidateLimit: 10 });

    // `zero` and `clean` are measured but never candidates;
    // `warming`/`pending` are not measured at all. Ordering is
    // `ABS(ratio - 1) DESC`.
    expect(
      report.candidates.map((candidate) => [candidate.username, candidate.band])
    ).toEqual([
      [ids.severe.username, "severe"],
      [ids.clear.username, "clear"],
      [ids.under.username, "under"],
    ]);

    expect(report.candidates[0]).toMatchObject({
      username: ids.severe.username,
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
      9
    );
    expect(report.candidates[1]).toMatchObject({
      totalTokens: "1500",
      highwaterTokens: "1000",
      ratio: 1.5,
      cliVersion: "4.03.0",
    });
    expect(report.candidates[2]).toMatchObject({
      totalTokens: "900",
      highwaterTokens: "1000",
      ratio: 0.9,
      cliVersion: "4.01.0",
    });
  });

  it("honours the candidate limit", async () => {
    const report = await getRatchetCensusReport({ candidateLimit: 1 });

    expect(report.candidates).toHaveLength(1);
    expect(report.candidates[0].username).toBe(ids.severe.username);
  });
});
