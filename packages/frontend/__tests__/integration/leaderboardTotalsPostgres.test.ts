import { randomUUID } from "node:crypto";
import postgres from "postgres";
import { afterAll, beforeAll, describe, expect, it, vi } from "vitest";

// getLeaderboardData wraps its query in unstable_cache; outside a Next request
// there is no cache to consult, so run the query directly.
vi.mock("next/cache", () => ({ unstable_cache: (fn: () => unknown) => fn }));

import { getLeaderboardData } from "@/lib/leaderboard/getLeaderboard";

const integrationEnabled =
  process.env.LEADERBOARD_TOTALS_DB_INTEGRATION === "1";
const describeWithPostgres = integrationEnabled ? describe : describe.skip;

/**
 * Executes the real leaderboard SQL against a migrated PostgreSQL instance.
 *
 * The unit suites mock `db.execute`, so they can only prove that the
 * `leaderboard_hidden` filter text sits inside the `stats` CTE. Whether the
 * headline totals actually drop a hidden account is only observable by
 * running the statement: that is what this file does.
 *
 * The hidden persona is sized like the real accounts that prompted the
 * change (99% of all-time tokens), so a regression moves the totals by
 * orders of magnitude rather than by a rounding error.
 *
 * The period cases own their date (1999-03-04, before any real usage), so
 * their expectations are exact. The all-time case cannot own the database
 * (other integration files insert users into it), so it checks the
 * invariant instead: headline users == ranked users, and the hidden
 * persona's tokens are absent.
 */
describeWithPostgres("leaderboard totals PostgreSQL integration", () => {
  const databaseUrl = process.env.DATABASE_URL;
  const suffix = randomUUID().replaceAll("-", "").slice(0, 8);
  const day = "1999-03-04";
  const githubIdBase = 1_900_000_000 + Math.floor(Math.random() * 90_000_000);

  const personas = {
    alice: { tokens: 1_000, cost: 10, hidden: false },
    bob: { tokens: 500, cost: 5, hidden: false },
    // 9,040T tokens at $0.038 per 1M: the shape of the hidden accounts
    // on tokscale.ai on 2026-09-29.
    hidden: { tokens: 9_040_000_000_000_000, cost: 342_450_000, hidden: true },
  } as const;
  type Persona = keyof typeof personas;
  const names = Object.keys(personas) as Persona[];
  const ids = Object.fromEntries(
    names.map((name) => [
      name,
      {
        userId: randomUUID(),
        submissionId: randomUUID(),
        deviceId: randomUUID(),
        username: `lbtotals_${name}_${suffix}`,
      },
    ]),
  ) as Record<
    Persona,
    { userId: string; submissionId: string; deviceId: string; username: string }
  >;

  let fixtureDb: postgres.Sql | undefined;

  beforeAll(async () => {
    if (!databaseUrl) {
      throw new Error(
        "DATABASE_URL is required when LEADERBOARD_TOTALS_DB_INTEGRATION=1",
      );
    }
    fixtureDb = postgres(databaseUrl, { max: 1, prepare: false });
    await fixtureDb.begin(async (sql) => {
      for (const [index, name] of names.entries()) {
        const { tokens, cost, hidden } = personas[name];
        const { userId, submissionId, deviceId, username } = ids[name];
        await sql`
          INSERT INTO users (id, github_id, username, leaderboard_hidden)
          VALUES (${userId}, ${githubIdBase + index}, ${username}, ${hidden})
        `;
        await sql`
          INSERT INTO submissions (
            id, user_id, total_tokens, total_cost, input_tokens, output_tokens,
            date_start, date_end, sources_used, models_used
          )
          VALUES (
            ${submissionId}, ${userId}, ${tokens}, ${cost}, ${tokens}, 0,
            ${day}, ${day}, ARRAY['claude'], ARRAY['claude-sonnet-4']
          )
        `;
        await sql`
          INSERT INTO submitted_devices (id, user_id, device_key)
          VALUES (${deviceId}, ${userId}, ${`lbtotals-${name}-device`})
        `;
        await sql`
          INSERT INTO daily_breakdown (
            submission_id, submitted_device_id, date, tokens, cost,
            input_tokens, output_tokens, source_breakdown
          )
          VALUES (
            ${submissionId}, ${deviceId}, ${day}, ${tokens}, ${cost},
            ${tokens}, 0,
            ${sql.json({ claude: { tokens, cost, models: { "claude-sonnet-4": { tokens, cost } } } })}
          )
        `;
      }
    });
  });

  afterAll(async () => {
    if (!fixtureDb) return;
    for (const name of names) {
      await fixtureDb`DELETE FROM users WHERE id = ${ids[name].userId}`;
    }
    await fixtureDb.end();
  });

  it("drops a hidden user from period totals, not only from the ranking", async () => {
    const data = await getLeaderboardData("custom", 1, 50, "tokens", "", day, day);
    expect(data.users.map((user) => user.username)).toEqual([
      ids.alice.username,
      ids.bob.username,
    ]);
    expect(data.pagination.totalUsers).toBe(2);
    expect(data.stats).toEqual({ totalTokens: 1_500, totalCost: 15, uniqueUsers: 2 });
  });

  it("drops a hidden user from directive-filtered period totals", async () => {
    const data = await getLeaderboardData(
      "custom", 1, 50, "cost", "client:claude", day, day,
    );
    expect(data.pagination.totalUsers).toBe(2);
    expect(data.stats).toEqual({ totalTokens: 1_500, totalCost: 15, uniqueUsers: 2 });
  });

  it("counts the same users in all-time totals as in the all-time ranking", async () => {
    const data = await getLeaderboardData("all", 1, 1, "tokens");
    // Before the fix: uniqueUsers = ranked users + hidden users.
    expect(data.stats.uniqueUsers).toBe(data.pagination.totalUsers);
    // The hidden persona alone would put the total above this.
    expect(data.stats.totalTokens).toBeLessThan(personas.hidden.tokens);
    expect(data.users[0]?.username).not.toBe(ids.hidden.username);

    const [visible] = await fixtureDb!<{ tokens: string; cost: string }[]>`
      SELECT COALESCE(SUM(s.total_tokens), 0)::text AS tokens,
             COALESCE(SUM(CAST(s.total_cost AS DECIMAL(18,4))), 0)::text AS cost
      FROM submissions s INNER JOIN users u ON u.id = s.user_id
      WHERE u.leaderboard_hidden = false
    `;
    expect(data.stats.totalTokens).toBe(Number(visible.tokens));
    expect(data.stats.totalCost).toBeCloseTo(Number(visible.cost), 4);
  });
});
