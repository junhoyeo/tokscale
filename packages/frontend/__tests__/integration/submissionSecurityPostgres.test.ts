import { createHash, randomInt, randomUUID } from "node:crypto";
import type { Sql } from "postgres";
import { afterAll, afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import type { SubmissionData } from "@/lib/validation/submission";

const state = vi.hoisted(() => ({ client: undefined as Sql | undefined, cookie: undefined as string | undefined }));

vi.mock("@/lib/db", async () => {
  const { default: postgres } = await import("postgres");
  const { drizzle } = await import("drizzle-orm/postgres-js");
  const schema = await import("@/lib/db/schema");
  const client = postgres(process.env.DATABASE_URL ?? "postgres://unused", { max: 5, prepare: false });
  state.client = client;
  return { ...schema, db: drizzle(client, { schema }) };
});
vi.mock("next/cache", () => ({
  revalidateTag: vi.fn(), revalidatePath: vi.fn(), unstable_cache: <T>(fn: T) => fn,
}));
vi.mock("next/headers", () => ({
  cookies: async () => ({ get: (name: string) => name === "tt_session" && state.cookie ? { value: state.cookie } : undefined }),
}));
vi.mock("@/lib/groups/cache", () => ({ revalidateUserGroupLeaderboards: vi.fn() }));
vi.mock("@/lib/leaderboard/getLeaderboard", () => ({ getLeaderboardData: vi.fn(async () => ({})) }));

import { POST } from "@/app/api/submit/route";
import { POST as moderate } from "@/app/api/admin/moderation/[username]/route";
import { GET as getCandidates } from "@/app/api/admin/moderation/candidates/route";
import { applyModerationAction, findModerationTarget, getModerationHistory } from "@/lib/moderation/actions";
import { getModerationCandidates } from "@/lib/moderation/candidates";
import { validateSubmission } from "@/lib/validation/submission";

const describeWithPostgres = process.env.SUBMISSION_WITHHOLDING_DB_INTEGRATION === "1" ? describe : describe.skip;
const fixtureIds: string[] = [];
const unauthorizedRequests: Array<{ name: string; token?: string; headers: Record<string, string> }> = [
  { name: "missing token", headers: {} },
  { name: "session cookie without a personal token", headers: { cookie: "tt_session=forged" } },
  { name: "invalid token", token: "tt_invalid", headers: {} },
  { name: "SQL injection in token", token: "tt_' OR 1=1; --", headers: {} },
  { name: "unsupported authorization scheme", headers: { authorization: "Basic tt_forged" } },
];

function database(): Sql {
  if (!state.client) throw new Error("Test database was not initialized");
  return state.client;
}

async function account(expiresAt: Date | null = null) {
  const id = randomUUID();
  const tokenId = randomUUID();
  const token = `tt_${randomUUID().replaceAll("-", "")}`;
  const username = `security${id.replaceAll("-", "").slice(0, 12)}`;
  const githubId = randomInt(1_000_000_000, 2_000_000_000);
  const sql = database();
  fixtureIds.push(id);
  await sql`INSERT INTO users (id, github_id, username) VALUES (${id}, ${githubId}, ${username})`;
  await sql`INSERT INTO api_tokens (id, user_id, token, name, expires_at) VALUES (${tokenId}, ${id}, ${createHash("sha256").update(token).digest("hex")}, 'test', ${expiresAt?.toISOString() ?? null})`;
  return { id, tokenId, token, username, githubId };
}

async function webSession(a: { id: string }, expiresAt = new Date(Date.now() + 60_000)) {
  const token = randomUUID();
  await database()`INSERT INTO sessions (user_id, token_hash, expires_at) VALUES (${a.id}, ${createHash("sha256").update(token).digest("hex")}, ${expiresAt.toISOString()})`;
  return token;
}

function moderateRequest(username: string, origin = "http://localhost") {
  return moderate(new Request(`http://localhost/api/admin/moderation/${username}`, {
    method: "POST", headers: { origin, "content-type": "application/json" },
    body: JSON.stringify({ action: "hide", reason: "Under investigation" }),
  }), { params: Promise.resolve({ username }) });
}

function payload(modelId = "gpt-5", input = 1000): SubmissionData {
  const tokens = { input, output: 100, cacheRead: 0, cacheWrite: 0, reasoning: 0 };
  const totalTokens = input + 100;
  return {
    meta: { generatedAt: "2026-05-01T00:00:00Z", version: "4.20.0", dateRange: { start: "2026-04-30", end: "2026-04-30" } },
    device: { id: "security-device" },
    summary: { totalTokens, totalCost: 1, totalDays: 1, activeDays: 1, averagePerDay: 1, maxCostInSingleDay: 1, clients: ["opencode"], models: [modelId] },
    years: [{ year: "2026", totalTokens, totalCost: 1, range: { start: "2026-04-30", end: "2026-04-30" } }],
    contributions: [{ date: "2026-04-30", clients: [{ client: "opencode", modelId, tokens, cost: 1, messages: 1 }], totals: { tokens: totalTokens, cost: 1, messages: 1 }, intensity: 1, tokenBreakdown: tokens }],
  };
}

function submit(token?: string, data: unknown = payload(), headers: Record<string, string> = {}) {
  return POST(new Request("http://localhost/api/submit", {
    method: "POST",
    headers: { "content-type": "application/json", ...(token ? { authorization: `Bearer ${token}` } : {}), ...headers },
    body: JSON.stringify(data),
  }));
}

async function expectNoSubmission(a: Awaited<ReturnType<typeof account>>) {
  const sql = database();
  expect(await sql`SELECT id FROM submissions WHERE user_id = ${a.id}`).toHaveLength(0);
  expect(await sql`SELECT id FROM submitted_devices WHERE user_id = ${a.id}`).toHaveLength(0);
  expect(await sql`SELECT id FROM moderation_actions WHERE target_user_id = ${a.id}`).toHaveLength(0);
  const [row] = await sql`SELECT last_used_at FROM api_tokens WHERE id = ${a.tokenId}`;
  if (row) expect(row.last_used_at).toBeNull();
}

// Real token hashing, validation, route logic and PostgreSQL queries; only
// Next.js cache side effects are replaced. Run against a migrated test DB.
describeWithPostgres("submission security PostgreSQL integration", () => {
  beforeEach(() => {
    state.cookie = undefined;
    vi.stubEnv("TOKSCALE_ADMIN_GITHUB_IDS", "");
    vi.stubEnv("APP_URL", "http://localhost");
    vi.stubEnv("CSRF_ALLOWED_ORIGINS", "http://localhost");
    vi.spyOn(console, "warn").mockImplementation(() => {});
  });
  afterEach(() => {
    vi.restoreAllMocks();
    vi.unstubAllEnvs();
  });
  afterAll(async () => {
    const sql = database();
    for (const id of fixtureIds) await sql`DELETE FROM users WHERE id = ${id}`;
    await sql.end();
  });

  it.each(unauthorizedRequests)("rejects $name without writing usage", async ({ token, headers }) => {
    const a = await account();
    const response = await submit(token, payload(), headers);
    expect(response.status).toBe(401);
    await expectNoSubmission(a);
  });

  it("rejects expired tokens before any submission write", async () => {
    const a = await account(new Date(Date.now() - 60_000));
    const response = await submit(a.token);
    expect(response.status).toBe(401);
    expect(await response.json()).toEqual({ error: "API token has expired" });
    await expectNoSubmission(a);
  });

  it("rejects a revoked token even when the former owner still exists", async () => {
    const a = await account();
    await database()`DELETE FROM api_tokens WHERE id = ${a.tokenId}`;
    expect((await submit(a.token)).status).toBe(401);
    await expectNoSubmission(a);
  });

  it("uses the authenticated owner despite forged user, device and moderation fields", async () => {
    const owner = await account();
    const victim = await account();
    const response = await submit(owner.token, {
      ...payload(), userId: victim.id, username: victim.username,
      device: { id: "security-device", userId: victim.id },
      leaderboardHidden: true, actorUsername: "system", action: "hide",
    });
    expect(response.status).toBe(200);
    expect((await response.json()).username).toBe(owner.username);
    const sql = database();
    expect(await sql`SELECT id FROM submissions WHERE user_id = ${owner.id}`).toHaveLength(1);
    expect(await sql`SELECT id FROM submitted_devices WHERE user_id = ${owner.id}`).toHaveLength(1);
    await expectNoSubmission(victim);
  });

  it("stores SQL and HTML model names literally without executing them", async () => {
    const a = await account();
    const model = "fake-'); UPDATE users SET leaderboard_hidden = true; DROP TABLE moderation_actions; -- <img src=x onerror=alert(1)>";
    const data = payload(model);
    expect(validateSubmission(data).valid).toBe(true);
    expect((await submit(a.token, data)).status).toBe(200);
    const sql = database();
    const [row] = await sql`SELECT d.source_breakdown, u.leaderboard_hidden FROM daily_breakdown d JOIN submissions s ON s.id = d.submission_id JOIN users u ON u.id = s.user_id WHERE u.id = ${a.id}`;
    expect(row.source_breakdown.opencode.models[model].tokens).toBe(1100);
    expect(row.leaderboard_hidden).toBe(false);
    expect(await getModerationHistory(a.id)).toEqual([]);
    const candidate = (await getModerationCandidates()).find(c => c.userId === a.id);
    expect(candidate?.slopModels).toContain(model);
  });

  it("binds hostile moderation usernames and reasons as values", async () => {
    const target = await account();
    const admin = await account();
    const sql = database();
    const injection = "' OR 1=1; DROP TABLE users; --";
    expect(await findModerationTarget(injection)).toBeNull();
    expect(await findModerationTarget(target.username.toUpperCase())).toMatchObject({ id: target.id });
    const reason = `${injection} <svg onload=alert(1)>`;
    await applyModerationAction({
      target: { id: target.id, username: target.username, leaderboardHidden: false },
      actorUserId: admin.id, actorUsername: admin.username, action: "hide", reason,
    });
    expect((await getModerationHistory(target.id))[0].reason).toBe(reason);
    const [actor] = await sql`SELECT leaderboard_hidden FROM users WHERE id = ${admin.id}`;
    expect(actor.leaderboard_hidden).toBe(false);
  });

  it("cannot forge an administrator restore through submission data", async () => {
    const a = await account();
    expect((await submit(a.token, payload("gpt-5", 200_000_000))).status).toBe(200);
    expect(await getModerationHistory(a.id)).toHaveLength(1);
    expect((await submit(a.token, {
      ...payload(), leaderboardHidden: false, action: "unhide", actorUsername: "admin",
      moderationActions: [{ targetUserId: a.id, action: "unhide", actorUsername: "admin" }],
    })).status).toBe(200);
    const [row] = await database()`SELECT leaderboard_hidden FROM users WHERE id = ${a.id}`;
    expect(row.leaderboard_hidden).toBe(true);
    const history = await getModerationHistory(a.id);
    expect(history).toHaveLength(1);
    expect(history[0]).toMatchObject({ action: "hide", actorUsername: "system" });
  });

  it("rejects expired and fabricated web-session cookies against the real session ledger", async () => {
    const admin = await account();
    const target = await account();
    vi.stubEnv("TOKSCALE_ADMIN_GITHUB_IDS", String(admin.githubId));
    const expired = await webSession(admin, new Date(Date.now() - 60_000));
    for (const cookie of [expired, "' OR 1=1; --", randomUUID()]) {
      state.cookie = cookie;
      expect((await moderateRequest(target.username)).status).toBe(404);
      expect((await getCandidates(new Request("http://localhost/api/admin/moderation/candidates"))).status).toBe(404);
    }
    expect(await getModerationHistory(target.id)).toEqual([]);
  });

  it("does not grant admin authority to a real personal token or ordinary web session", async () => {
    const admin = await account();
    const ordinary = await account();
    const target = await account();
    vi.stubEnv("TOKSCALE_ADMIN_GITHUB_IDS", String(admin.githubId));
    const request = new Request("http://localhost/api/admin/moderation/candidates", { headers: { authorization: `Bearer ${admin.token}` } });
    expect((await getCandidates(request)).status).toBe(404);
    state.cookie = await webSession(ordinary);
    expect((await moderateRequest(target.username)).status).toBe(404);
    expect(await getModerationHistory(target.id)).toEqual([]);
  });

  it("requires a valid admin session and matching Origin before writing a real moderation action", async () => {
    const admin = await account();
    const target = await account();
    vi.stubEnv("TOKSCALE_ADMIN_GITHUB_IDS", String(admin.githubId));
    state.cookie = await webSession(admin);
    expect((await moderateRequest(target.username, "https://attacker.example")).status).toBe(404);
    expect(await getModerationHistory(target.id)).toEqual([]);
    expect((await moderateRequest(target.username)).status).toBe(200);
    const history = await getModerationHistory(target.id);
    expect(history).toHaveLength(1);
    expect(history[0]).toMatchObject({ action: "hide", actorUsername: admin.username });
  });
});
