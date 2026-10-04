import { randomUUID } from "node:crypto";
import postgres from "postgres";
import { afterAll, beforeAll, describe, expect, it, vi } from "vitest";

vi.mock("next/cache", () => ({ unstable_cache: (fn: () => unknown) => fn }));

let getLeaderboardData: (typeof import("../../src/lib/leaderboard/getLeaderboard"))["getLeaderboardData"];

const integrationEnabled = process.env.LEADERBOARD_DB_INTEGRATION === "1";
const describeWithPostgres = integrationEnabled ? describe : describe.skip;

describeWithPostgres("leaderboard PostgreSQL integration", () => {
  const databaseUrl = process.env.DATABASE_URL;
  const fixtureSuffix = randomUUID().replaceAll("-", "").slice(0, 8);
  const githubIdBase = -1_800_000_000 + Number.parseInt(fixtureSuffix.slice(0, 6), 16);
  let fixtureDb: ReturnType<typeof postgres>;

  const personas = [
    "alice",
    "bob",
    "cara",
    "dana",
    "eve",
    "hidden",
  ] as const;
  type Persona = (typeof personas)[number];

  const ids = Object.fromEntries(
    personas.map((persona) => [
      persona,
      {
        userId: randomUUID(),
        submissionId: randomUUID(),
        deviceId: randomUUID(),
        username: `lb_${persona}_${fixtureSuffix}`,
      },
    ]),
  ) as Record<Persona, { userId: string; submissionId: string; deviceId: string; username: string }>;

  const insertPersona = async (
    sql: ReturnType<typeof postgres>,
    persona: Persona,
    totals: { tokens: number; cost: number; sources: string[]; models: string[]; hidden?: boolean },
    sourceBreakdown: postgres.JSONValue,
  ) => {
    const { userId, submissionId, deviceId, username } = ids[persona];
    await sql`
      INSERT INTO users (id, github_id, username, display_name, leaderboard_hidden)
      VALUES (${userId}, ${githubIdBase + personas.indexOf(persona)}, ${username}, ${`Fixture ${persona}`}, ${totals.hidden === true})
    `;
    await sql`
      INSERT INTO submissions (
        id, user_id, total_tokens, total_cost, input_tokens, output_tokens,
        date_start, date_end, sources_used, models_used, cli_version
      )
      VALUES (
        ${submissionId}, ${userId}, ${totals.tokens}, ${totals.cost},
        ${totals.tokens}, 0, '2026-01-01', '2026-01-01',
        ${totals.sources}, ${totals.models}, '4.99.0'
      )
    `;
    await sql`
      INSERT INTO submitted_devices (id, user_id, device_key)
      VALUES (${deviceId}, ${userId}, ${`leaderboard-${persona}-${fixtureSuffix}`})
    `;
    await sql`
      INSERT INTO daily_breakdown (
        submission_id, submitted_device_id, date, tokens, cost,
        input_tokens, output_tokens, source_breakdown
      )
      VALUES (
        ${submissionId}, ${deviceId}, '2026-01-01', ${totals.tokens}, ${totals.cost},
        ${totals.tokens}, 0, ${sourceBreakdown === null ? null : sql.json(sourceBreakdown)}
      )
    `;
  };

  beforeAll(async () => {
    if (!databaseUrl) {
      throw new Error("DATABASE_URL is required when LEADERBOARD_DB_INTEGRATION=1");
    }

    ({ getLeaderboardData } = await import("@/lib/leaderboard/getLeaderboard"));
    fixtureDb = postgres(databaseUrl, { max: 1, prepare: false });
    await fixtureDb.begin(async (sql) => {
      await insertPersona(
        sql,
        "alice",
        { tokens: 100_000_000, cost: 1000, sources: ["codex", "claude"], models: ["gpt-5", "legacy"] },
        {
          codex: {
            tokens: 1_000_000,
            cost: 30,
            input: 1_000_000,
            output: 0,
            models: { "gpt-5": { tokens: 1_000_000, cost: 30, input: 1_000_000, output: 0 } },
          },
          claude: {
            tokens: 99_000_000,
            cost: 970,
            input: 99_000_000,
            output: 0,
            models: { legacy: { tokens: 99_000_000, cost: 970, input: 99_000_000, output: 0 } },
          },
        },
      );
      await insertPersona(
        sql,
        "bob",
        { tokens: 2_000_000, cost: 20, sources: ["codex"], models: ["gpt-5"] },
        {
          codex: {
            tokens: 2_000_000,
            cost: 20,
            input: 2_000_000,
            output: 0,
            models: { "gpt-5": { tokens: 2_000_000, cost: 20, input: 2_000_000, output: 0 } },
          },
        },
      );
      await insertPersona(
        sql,
        "cara",
        { tokens: 3_000_000, cost: 30, sources: ["claude"], models: ["gpt-5"] },
        {
          claude: {
            tokens: 3_000_000,
            cost: 30,
            input: 3_000_000,
            output: 0,
            models: { "gpt-5": { tokens: 3_000_000, cost: 30, input: 3_000_000, output: 0 } },
          },
        },
      );
      await insertPersona(
        sql,
        "dana",
        { tokens: 500, cost: 5, sources: ["codex", "claude"], models: ["gpt-5", "legacy"] },
        {
          codex: {
            tokens: 0,
            cost: 0,
            input: 0,
            output: 0,
            models: { "gpt-5": { tokens: 0, cost: 0, input: 0, output: 0 } },
          },
          claude: {
            tokens: 500,
            cost: 5,
            input: 500,
            output: 0,
            models: { legacy: { tokens: 500, cost: 5, input: 500, output: 0 } },
          },
        },
      );
      await insertPersona(
        sql,
        "eve",
        { tokens: 4_000_000, cost: 40, sources: ["codex"], models: ["gpt-5"] },
        null,
      );
      await insertPersona(
        sql,
        "hidden",
        { tokens: 7_000_000, cost: 70, sources: ["codex"], models: ["gpt-5"], hidden: true },
        {
          codex: {
            tokens: 7_000_000,
            cost: 70,
            input: 7_000_000,
            output: 0,
            models: { "gpt-5": { tokens: 7_000_000, cost: 70, input: 7_000_000, output: 0 } },
          },
        },
      );
    });
  });

  afterAll(async () => {
    if (!fixtureDb) return;
    await fixtureDb`
      DELETE FROM users
      WHERE id IN ${fixtureDb(personas.map((persona) => ids[persona].userId))}
    `;
    await fixtureDb.end({ timeout: 5 });
  });

  it("scopes all-time source and model directives to daily breakdown cells", async () => {
    const scoped = await getLeaderboardData("all", 1, 50, "tokens", "client:codex model:gpt-5");
    expect(scoped.users.map((user) => [user.username, user.rank, user.totalTokens])).toEqual([
      [ids.bob.username, 1, 2_000_000],
      [ids.alice.username, 2, 1_000_000],
      [ids.dana.username, 3, 0],
    ]);
    expect(scoped.users.some((user) => user.username === ids.cara.username)).toBe(false);
    expect(scoped.users.some((user) => user.username === ids.eve.username)).toBe(false);
    expect(scoped.stats).toEqual({ totalTokens: 109_000_500, totalCost: 1095, uniqueUsers: 5 });

    const modelOnly = await getLeaderboardData("all", 1, 50, "tokens", "model:gpt-5");
    expect(modelOnly.users.map((user) => [user.username, user.totalTokens])).toEqual([
      [ids.cara.username, 3_000_000],
      [ids.bob.username, 2_000_000],
      [ids.alice.username, 1_000_000],
      [ids.dana.username, 0],
    ]);

    const clientOnly = await getLeaderboardData("all", 1, 50, "tokens", "client:codex");
    expect(clientOnly.users.map((user) => [user.username, user.totalTokens, user.totalCost])).toEqual([
      [ids.bob.username, 2_000_000, 20],
      [ids.alice.username, 1_000_000, 30],
      [ids.dana.username, 0, 0],
    ]);

    const costSorted = await getLeaderboardData("all", 1, 50, "cost", "client:codex model:gpt-5");
    expect(costSorted.users.map((user) => [user.username, user.totalTokens, user.totalCost])).toEqual([
      [ids.alice.username, 1_000_000, 30],
      [ids.bob.username, 2_000_000, 20],
      [ids.dana.username, 0, 0],
    ]);

    const pageTwo = await getLeaderboardData("all", 2, 1, "tokens", "client:codex model:gpt-5 lb_");
    expect(pageTwo.users).toHaveLength(1);
    expect(pageTwo.users[0]).toMatchObject({ username: ids.alice.username, rank: 2, totalTokens: 1_000_000 });
    expect(pageTwo.pagination).toMatchObject({ page: 2, limit: 1, totalUsers: 3, totalPages: 3 });

    const textFiltered = await getLeaderboardData("all", 1, 50, "tokens", `client:codex model:gpt-5 ${ids.alice.username}`);
    expect(textFiltered.users).toHaveLength(1);
    expect(textFiltered.users[0]).toMatchObject({ username: ids.alice.username, rank: 2, totalTokens: 1_000_000 });
  });
});
