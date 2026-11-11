import { randomUUID } from "node:crypto";
import { beforeAll, afterAll, describe, expect, it, vi } from "vitest";
import { sql } from "drizzle-orm";

const auth = vi.hoisted(() => ({ user: null as null | { id: string; username: string } }));
vi.mock("next/cache", () => ({ revalidateTag: vi.fn(), revalidatePath: vi.fn() }));
vi.mock("@/lib/auth/requestSession", () => ({ getSessionFromRequest: async () => auth.user }));
vi.mock("@/lib/auth/session", () => ({ clearSession: async () => {} }));
vi.mock("@/lib/auth/personalTokens", () => ({ authenticatePersonalToken: vi.fn() }));
vi.mock("@/lib/groups/cache", () => ({ revalidateUserGroupLeaderboards: async () => {} }));

import { db } from "@/lib/db";
import { getExploreCacheGeneration, advanceExploreCacheRevision } from "@/lib/explore/revision";
import { applyModerationAction } from "@/lib/moderation/actions";

// Never run against the shared development DB or an inherited production URL.
// Opt in by explicitly selecting a migrated, isolated local database:
// DATABASE_URL=postgresql://...@127.0.0.1:5480/tokscale_analytics_revision_<name>
const url = process.env.DATABASE_URL ? new URL(process.env.DATABASE_URL) : null;
const isolatedLocal = url?.hostname === "127.0.0.1" && url.port === "5480"
  && url.pathname.startsWith("/tokscale_analytics_revision_");

(isolatedLocal ? describe : describe.skip)("explore revision PostgreSQL integration", () => {
  const suffix = randomUUID().slice(0, 8);
  const people = ["moderation", "account", "submission"].map((kind) => ({ id: randomUUID(), username: `revision_${kind}_${suffix}` }));
  const githubBase = 1_700_000_000 + Math.floor(Math.random() * 100_000_000);

  beforeAll(async () => {
    await db.execute(sql`SET statement_timeout = '5s'`);
    await db.execute(sql`SET lock_timeout = '3s'`);
    for (const [index, person] of people.entries()) {
      await db.execute(sql`INSERT INTO users (id, github_id, username) VALUES (${person.id}, ${githubBase + index}, ${person.username})`);
    }
    await db.execute(sql`
      INSERT INTO submissions (user_id, total_tokens, total_cost, input_tokens, output_tokens, date_start, date_end, sources_used, models_used)
      VALUES (${people[2].id}, 100, 1, 100, 0, '2026-10-01', '2026-10-01', ARRAY['codex'], ARRAY['gpt-5.5'])
    `);
  });

  afterAll(async () => {
    for (const person of people) {
      await db.execute(sql`DELETE FROM moderation_actions WHERE target_username = ${person.username}`);
      await db.execute(sql`DELETE FROM users WHERE id = ${person.id}`);
    }
  });

  it("reads the seeded singleton through its primary key", async () => {
    expect(await getExploreCacheGeneration()).toMatch(/^\d+$/);
    const rows = await db.execute<{ indexdef: string }>(sql`SELECT indexdef FROM pg_indexes WHERE tablename = 'explore_cache_revision'`);
    expect(rows.some(row => /UNIQUE INDEX.*USING btree \(id\)/.test(row.indexdef))).toBe(true);
  });

  it("increments with hide and show, but not an idempotent no-op", async () => {
    const person = people[0];
    const before = BigInt(await getExploreCacheGeneration());
    const hide = () => applyModerationAction({ target: { ...person, leaderboardHidden: false }, actorUserId: person.id, actorUsername: person.username, action: "hide", reason: "integration test" });
    expect((await hide()).changed).toBe(true);
    expect(BigInt(await getExploreCacheGeneration())).toBe(before + 1n);
    expect((await hide()).changed).toBe(false);
    expect(BigInt(await getExploreCacheGeneration())).toBe(before + 1n);
    await applyModerationAction({ target: { ...person, leaderboardHidden: true }, actorUserId: person.id, actorUsername: person.username, action: "unhide", reason: "integration test" });
    expect(BigInt(await getExploreCacheGeneration())).toBe(before + 2n);
  });

  it("rolls back the privacy state and revision together", async () => {
    const before = await getExploreCacheGeneration();
    await expect(db.transaction(async (tx) => {
      await tx.execute(sql`UPDATE users SET leaderboard_hidden = true WHERE id = ${people[0].id}`);
      await advanceExploreCacheRevision(tx);
      throw new Error("rollback probe");
    })).rejects.toThrow("rollback probe");
    expect(await getExploreCacheGeneration()).toBe(before);
    const rows = await db.execute<{ hidden: boolean }>(sql`SELECT leaderboard_hidden AS hidden FROM users WHERE id = ${people[0].id}`);
    expect(rows[0].hidden).toBe(false);
  });

  it("increments in the real account-deletion transaction", async () => {
    const before = BigInt(await getExploreCacheGeneration());
    auth.user = people[1];
    const { DELETE } = await import("@/app/api/settings/account/route");
    expect((await DELETE(new Request("http://localhost/api/settings/account", { method: "DELETE" }))).status).toBe(200);
    expect(BigInt(await getExploreCacheGeneration())).toBe(before + 1n);
    const rows = await db.execute(sql`SELECT id FROM users WHERE id = ${people[1].id}`);
    expect(rows).toHaveLength(0);
  });

  it("increments in submitted-data deletion while preserving the account", async () => {
    const before = BigInt(await getExploreCacheGeneration());
    auth.user = people[2];
    const { DELETE } = await import("@/app/api/settings/submitted-data/route");
    expect((await DELETE(new Request("http://localhost/api/settings/submitted-data", { method: "DELETE" }))).status).toBe(200);
    expect(BigInt(await getExploreCacheGeneration())).toBe(before + 1n);
    expect(await db.execute(sql`SELECT id FROM users WHERE id = ${people[2].id}`)).toHaveLength(1);
    expect(await db.execute(sql`SELECT id FROM submissions WHERE user_id = ${people[2].id}`)).toHaveLength(0);
  });
});
