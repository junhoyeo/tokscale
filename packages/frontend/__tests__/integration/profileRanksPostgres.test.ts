import { randomUUID } from "node:crypto";
import postgres from "postgres";
import { afterAll, beforeAll, describe, expect, it, vi } from "vitest";
import type { ProfilePeriod } from "@/lib/reportingPeriods";

// Exercise date-sensitive cache keys as well as real SQL, without Next's runtime.
vi.mock("next/cache", () => {
  const cache = new Map<string, unknown>();
  return { unstable_cache: (fn: () => unknown, keys: string[]) => () => {
    const key = JSON.stringify(keys);
    if (!cache.has(key)) cache.set(key, fn());
    return cache.get(key);
  } };
});

const enabled = process.env.PROFILE_RANKS_DB_INTEGRATION === "1";
describe.skipIf(!enabled)("UTC ranks across public surfaces (PostgreSQL)", () => {
  const suffix = randomUUID().slice(0, 8);
  const names = ["alice", "bob", "edge", "future", "hidden", "zero"] as const;
  type Name = typeof names[number];
  const fixtures = Object.fromEntries(names.map((name) => [name, {
    id: randomUUID(), submission: randomUUID(), device: randomUUID(), username: `utc_${name}_${suffix}`,
  }])) as Record<Name, { id: string; submission: string; device: string; username: string }>;
  let sql: ReturnType<typeof postgres>;
  let leaderboard: typeof import("@/lib/leaderboard/getLeaderboard");
  let embed: typeof import("@/lib/embed/getUserEmbedStats");
  let profile: typeof import("@/lib/publicProfileData");

  beforeAll(async () => {
    if (!process.env.DATABASE_URL) throw new Error("DATABASE_URL is required");
    sql = postgres(process.env.DATABASE_URL, { max: 1, prepare: false });
    [leaderboard, embed, profile] = await Promise.all([
      import("@/lib/leaderboard/getLeaderboard"), import("@/lib/embed/getUserEmbedStats"), import("@/lib/publicProfileData"),
    ]);
    const days: Record<Name, [string, number][]> = {
      alice: [["2026-03-31", 100], ["2026-04-02", 10_000]],
      bob: [["2026-03-01", 20], ["2026-03-31", 100], ["2026-04-01", 20_000]],
      edge: [["2026-03-25", 150], ["2026-04-02", 0]],
      future: [["2026-04-01", 200_000]],
      hidden: [["2026-03-31", 1_000_000]],
      zero: [["2026-03-31", 0]],
    };
    await sql.begin(async (tx) => {
      for (const [index, name] of names.entries()) {
        const f = fixtures[name];
        const total = days[name].reduce((sum, [, tokens]) => sum + tokens, 0);
        await tx`INSERT INTO users (id, github_id, username, leaderboard_hidden)
          VALUES (${f.id}, ${-2_000_000_000 + Number.parseInt(suffix.slice(0, 6), 16) + index}, ${f.username}, ${name === "hidden"})`;
        await tx`INSERT INTO submissions (id, user_id, total_tokens, total_cost, input_tokens, output_tokens, date_start, date_end, sources_used, models_used, cli_version)
          VALUES (${f.submission}, ${f.id}, ${total}, ${total / 100}, ${total}, 0, ${days[name][0][0]}, ${days[name].at(-1)![0]}, ARRAY['codex'], ARRAY['gpt-5'], '4.99.0')`;
        await tx`INSERT INTO submitted_devices (id, user_id, device_key) VALUES (${f.device}, ${f.id}, ${f.device})`;
        for (const [date, tokens] of days[name]) {
          await tx`INSERT INTO daily_breakdown (submission_id, submitted_device_id, date, tokens, cost, input_tokens, output_tokens)
            VALUES (${f.submission}, ${f.device}, ${date}, ${tokens}, ${tokens / 100}, ${tokens}, 0)`;
        }
      }
    });
    vi.useFakeTimers({ toFake: ["Date"] });
  });

  afterAll(async () => {
    vi.useRealTimers();
    if (sql) {
      await sql`DELETE FROM users WHERE id IN ${sql(names.map((name) => fixtures[name].id))}`;
      await sql.end();
    }
  });

  async function readProfile(name: Name, period: ProfilePeriod) {
    const username = fixtures[name].username;
    const response = await profile.getPublicProfileResponse(new Request(`http://localhost/api/users/${username}?period=${period}`), { params: Promise.resolve({ username }) });
    expect(response.status).toBe(200);
    return response.json();
  }

  it("agrees across ties, hidden accounts, zero rows and +1/+2 local days", async () => {
    vi.setSystemTime(new Date("2026-03-31T23:59:59Z"));
    for (const sort of ["tokens", "cost"] as const) {
      const board = await leaderboard.getLeaderboardData("week", 1, 100, sort);
      expect(board.users.map((u) => u.username)).toEqual(["edge", "alice", "bob", "zero"].map((n) => fixtures[n as Name].username));
      for (const name of names) {
        const expected = board.users.find((u) => u.username === fixtures[name].username)?.rank ?? null;
        const card = await embed.getUserEmbedStats(fixtures[name].username, sort, "week");
        expect(card?.stats.rank).toBe(expected);
        expect(card?.rankDateRange).toEqual({ start: "2026-03-25", end: "2026-03-31" });
        expect((await leaderboard.getUserRank(fixtures[name].username, "week", sort))?.rank ?? null).toBe(expected);
        if (sort === "tokens") expect((await readProfile(name, "week")).user.rank).toBe(expected);
      }
    }
    const edge = await readProfile("edge", "week");
    expect(edge.stats.totalTokens).toBe(0);
    expect(edge.user.rank).toBe(1);
    expect(edge.dateRange).toEqual({ start: "2026-03-27", end: "2026-04-02" });
    expect((await readProfile("alice", "week")).stats.totalTokens).toBe(10_100);
    expect((await embed.getUserEmbedStats(fixtures.edge.username, "tokens", "week"))?.stats.rank).toBe(1);
  });

  it("preserves rolling 30d versus calendar-month semantics", async () => {
    vi.setSystemTime(new Date("2026-03-31T23:59:59Z"));
    const rolling = await leaderboard.getLeaderboardData("custom", 1, 100, "tokens", "", "2026-03-02", "2026-03-31");
    const calendar = await leaderboard.getLeaderboardData("month", 1, 100);
    for (const name of names) {
      const rollingRank = rolling.users.find((u) => u.username === fixtures[name].username)?.rank ?? null;
      const calendarRank = calendar.users.find((u) => u.username === fixtures[name].username)?.rank ?? null;
      expect((await readProfile(name, "month")).user.rank).toBe(rollingRank);
      expect((await embed.getUserEmbedStats(fixtures[name].username, "tokens", "month"))?.stats.rank).toBe(rollingRank);
      expect((await readProfile(name, "this-month")).user.rank).toBe(calendarRank);
    }
    expect((await readProfile("bob", "month")).user.rank).toBe(3);
    expect((await readProfile("bob", "this-month")).user.rank).toBe(2);
  });

  it("refreshes cached windows at UTC midnight and month rollover", async () => {
    vi.setSystemTime(new Date("2026-04-01T00:00:00Z"));
    const board = await leaderboard.getLeaderboardData("week", 1, 100);
    expect(board.users[0].username).toBe(fixtures.future.username);
    for (const name of names) {
      const expected = board.users.find((u) => u.username === fixtures[name].username)?.rank ?? null;
      expect((await readProfile(name, "week")).user.rank).toBe(expected);
      expect((await embed.getUserEmbedStats(fixtures[name].username, "tokens", "week"))?.stats.rank).toBe(expected);
      expect((await leaderboard.getUserRank(fixtures[name].username, "week"))?.rank ?? null).toBe(expected);
    }
    const month = await leaderboard.getLeaderboardData("month", 1, 100);
    expect(month.users.map((u) => u.username)).toEqual([fixtures.future.username, fixtures.bob.username]);
    expect((await readProfile("alice", "this-month")).user.rank).toBeNull();
    expect((await readProfile("bob", "last-month")).user.rank).toBe(2);
  });
});
