import { randomUUID } from "node:crypto";
import postgres from "postgres";
import { afterAll, beforeAll, beforeEach, describe, expect, it, vi } from "vitest";

// Multiple connections simulate independent serverless instances. Keep the
// production pool size unchanged; a single test connection would hide races.
vi.mock("@/lib/db", async () => {
  const schema = await import("@/lib/db/schema");
  const { drizzle } = await import("drizzle-orm/postgres-js");
  return { ...schema, db: drizzle({ connection: {
    url: process.env.DATABASE_URL!, max: 12, prepare: false, ssl: false,
  }, schema }) };
});

describe.skipIf(process.env.GROUP_INVITES_DB_INTEGRATION !== "1")("group invite transactions (PostgreSQL)", () => {
  const suffix = randomUUID().slice(0, 8);
  const groupId = randomUUID();
  const accounts = Array.from({ length: 12 }, (_, i) => ({
    id: randomUUID(), username: `invite_${suffix}_${i}`, displayName: null, avatarUrl: null, githubId: null,
  }));
  const [owner, admin, ...members] = accounts;
  let fixtureDb: ReturnType<typeof postgres>;
  let lib: typeof import("@/lib/groups/invites");
  let appDb: typeof import("@/lib/db").db;

  beforeAll(async () => {
    if (!process.env.DATABASE_URL) throw new Error("DATABASE_URL is required");
    fixtureDb = postgres(process.env.DATABASE_URL, { max: 3, prepare: false });
    lib = await import("@/lib/groups/invites");
    ({ db: appDb } = await import("@/lib/db"));
    await fixtureDb.begin(async (sql) => {
      for (const [i, account] of accounts.entries()) {
        await sql`INSERT INTO users (id, github_id, username) VALUES (${account.id}, ${-1_900_000_000 + Number.parseInt(suffix.slice(0, 6), 16) + i}, ${account.username})`;
      }
      await sql`INSERT INTO groups (id, name, slug, created_by) VALUES (${groupId}, 'Invite transaction fixture', ${`invite-${suffix}`}, ${owner.id})`;
    });
  });

  beforeEach(async () => {
    await fixtureDb`DELETE FROM group_invites WHERE group_id = ${groupId}`;
    await fixtureDb`DELETE FROM group_members WHERE group_id = ${groupId}`;
    await fixtureDb`INSERT INTO group_members (group_id, user_id, role) VALUES (${groupId}, ${owner.id}, 'owner'), (${groupId}, ${admin.id}, 'admin')`;
  });

  afterAll(async () => {
    if (fixtureDb) {
      await fixtureDb`DELETE FROM users WHERE id IN ${fixtureDb(accounts.map((a) => a.id))}`;
      await fixtureDb.end();
    }
    await appDb?.$client.end();
  });

  function create(maxUses = 1, invitedBy = owner.id) {
    return lib.createGroupInvite({ groupId, invitedBy, role: "member", maxUses });
  }

  async function state(id: string) {
    const [row] = await fixtureDb`SELECT status, use_count, max_uses, accepted_at,
      (SELECT COUNT(*)::int FROM group_invite_redemptions WHERE invite_id = ${id}) AS redemptions
      FROM group_invites WHERE id = ${id}`;
    return row;
  }

  it("lets exactly the configured number of concurrent distinct accounts join", async () => {
    const invite = await create(3);
    const results = await Promise.allSettled(members.map((a) => lib.acceptGroupInvite(invite.token, a)));
    expect(results.filter((r) => r.status === "fulfilled")).toHaveLength(3);
    expect(await state(invite.id)).toMatchObject({ status: "accepted", use_count: 3, redemptions: 3 });
    const [count] = await fixtureDb`SELECT COUNT(*)::int AS n FROM group_members WHERE group_id = ${groupId}`;
    expect(count.n).toBe(5);
    await expect(lib.getGroupInvitePreview(invite.token)).rejects.toMatchObject({ code: "not_found" });
  });

  it("allows only one redemption by an account, including after it leaves", async () => {
    const invite = await create(5);
    const results = await Promise.allSettled(Array.from({ length: 4 }, () => lib.acceptGroupInvite(invite.token, members[0])));
    expect(results.filter((r) => r.status === "fulfilled")).toHaveLength(1);
    await fixtureDb`DELETE FROM group_members WHERE group_id = ${groupId} AND user_id = ${members[0].id}`;
    await expect(lib.acceptGroupInvite(invite.token, members[0])).rejects.toMatchObject({ code: "conflict" });
    expect(await state(invite.id)).toMatchObject({ status: "pending", use_count: 1, redemptions: 1, accepted_at: null });
    expect((await lib.getGroupInvitePreview(invite.token)).remainingUses).toBe(4);
  });

  it("does not consume an invite or change an existing member's role", async () => {
    const invite = await create(2);
    await expect(lib.acceptGroupInvite(invite.token, admin)).rejects.toMatchObject({ code: "conflict" });
    expect(await state(invite.id)).toMatchObject({ use_count: 0, redemptions: 0 });
    const [membership] = await fixtureDb`SELECT role FROM group_members WHERE group_id = ${groupId} AND user_id = ${admin.id}`;
    expect(membership.role).toBe("admin");
  });

  it("rolls back the losing invite when different inviters concurrently admit one account", async () => {
    const first = await create(2, owner.id);
    const second = await create(2, admin.id);
    const results = await Promise.allSettled([first, second].map((invite) => lib.acceptGroupInvite(invite.token, members[0])));
    expect(results.filter((r) => r.status === "fulfilled")).toHaveLength(1);
    const states = await Promise.all([state(first.id), state(second.id)]);
    expect(states.reduce((n, r) => n + r.use_count, 0)).toBe(1);
    expect(states.reduce((n, r) => n + r.redemptions, 0)).toBe(1);
  });

  it("serializes revocation against acceptance and rejects every later join", async () => {
    const invite = await create(3);
    const [accept, revoke] = await Promise.allSettled([
      lib.acceptGroupInvite(invite.token, members[0]), lib.revokeGroupInvite(groupId, invite.id, admin.id),
    ]);
    expect(revoke.status).toBe("fulfilled");
    expect(await state(invite.id)).toMatchObject({ status: "revoked", use_count: accept.status === "fulfilled" ? 1 : 0 });
    await expect(lib.acceptGroupInvite(invite.token, members[1])).rejects.toMatchObject({ code: "not_found" });
  });

  it("rechecks permission after inviter demotion and protects admin invitations", async () => {
    const invite = await create(3, admin.id);
    await fixtureDb`UPDATE group_members SET role = 'member' WHERE group_id = ${groupId} AND user_id = ${admin.id}`;
    await expect(lib.acceptGroupInvite(invite.token, members[0])).rejects.toMatchObject({ code: "forbidden" });
    await expect(create(3, admin.id)).rejects.toMatchObject({ code: "forbidden" });
    await expect(lib.revokeGroupInvite(groupId, invite.id, admin.id)).rejects.toMatchObject({ code: "forbidden" });
    await fixtureDb`UPDATE group_members SET role = 'admin' WHERE group_id = ${groupId} AND user_id = ${admin.id}`;
    const elevated = await lib.createGroupInvite({ groupId, invitedBy: owner.id, role: "admin", maxUses: 2 });
    await expect(lib.revokeGroupInvite(groupId, elevated.id, admin.id)).rejects.toMatchObject({ code: "not_found" });
    expect((await lib.listGroupInvites(groupId, "admin")).some((i) => i.id === elevated.id)).toBe(false);
  });

  it("rejects expiry while a redemption waits for the inviter lock", async () => {
    const invite = await create(2);
    let attempt: Promise<unknown>;
    await fixtureDb.begin(async (sql) => {
      await sql`SELECT 1 FROM group_members WHERE group_id = ${groupId} AND user_id = ${owner.id} FOR UPDATE`;
      // Attach the rejection handler immediately, before the lock is released.
      attempt = lib.acceptGroupInvite(invite.token, members[0]).then(() => "accepted", (error) => error);
      let blocked = false;
      for (let i = 0; i < 100; i++) {
        const [waiting] = await fixtureDb`SELECT COUNT(*)::int AS n FROM pg_stat_activity WHERE datname = current_database() AND wait_event_type = 'Lock'`;
        if (waiting.n > 0) { blocked = true; break; }
        await new Promise((resolve) => setTimeout(resolve, 10));
      }
      expect(blocked).toBe(true);
      await sql`UPDATE group_invites SET expires_at = clock_timestamp() - interval '1 second' WHERE id = ${invite.id}`;
    });
    expect(await attempt!).toMatchObject({ code: "not_found" });
    expect(await state(invite.id)).toMatchObject({ use_count: 0, redemptions: 0 });
  });

  it("keeps targeted invites single-use and bound to their account", async () => {
    await expect(lib.createGroupInvite({ groupId, invitedBy: owner.id, role: "member", maxUses: 2, invitedUsername: members[0].username })).rejects.toMatchObject({ code: "invalid" });
    const invite = await lib.createGroupInvite({ groupId, invitedBy: owner.id, role: "member", invitedUserId: members[0].id });
    await expect(lib.acceptGroupInvite(invite.token, members[1])).rejects.toMatchObject({ code: "forbidden" });
    await lib.acceptGroupInvite(invite.token, members[0]);
    expect(await state(invite.id)).toMatchObject({ use_count: 1, max_uses: 1, status: "accepted" });
  });

  it("enforces limits in the service and database and never returns token hashes in lists", async () => {
    for (const maxUses of [0, -1, 1.5, 1001, NaN]) await expect(create(maxUses)).rejects.toMatchObject({ code: "invalid" });
    const invite = await create(2);
    for (const expiresAt of [new Date(0), new Date(NaN), new Date(Date.now() + 31 * 86400000)]) {
      await expect(lib.createGroupInvite({ groupId, invitedBy: owner.id, role: "member", expiresAt })).rejects.toMatchObject({ code: "invalid" });
    }
    await expect(fixtureDb`UPDATE group_invites SET use_count = 3 WHERE id = ${invite.id}`).rejects.toMatchObject({ code: "23514" });
    const listed = await lib.listGroupInvites(groupId, "owner");
    expect(listed[0]).not.toHaveProperty("tokenHash");
    expect(JSON.stringify(listed)).not.toContain(invite.token);
    await expect(lib.listGroupInvites(groupId, "member")).rejects.toMatchObject({ code: "forbidden" });
  });
});
