import { createHash, randomInt, randomUUID } from "node:crypto";
import { sql as drizzleSql } from "drizzle-orm";
import type { Sql, TransactionSql } from "postgres";
import { afterAll, afterEach, beforeAll, beforeEach, describe, expect, it, vi } from "vitest";
import type { SubmissionData } from "@/lib/validation/submission";

const state = vi.hoisted(() => ({ client: undefined as Sql | undefined }));

vi.mock("@/lib/db", async () => {
  const { default: postgres } = await import("postgres");
  const { drizzle } = await import("drizzle-orm/postgres-js");
  const schema = await import("@/lib/db/schema");
  const client = postgres(process.env.DATABASE_URL ?? "postgres://unused", { max: 10, prepare: false });
  state.client = client;
  return { ...schema, db: drizzle(client, { schema }) };
});
vi.mock("next/cache", () => ({
  revalidateTag: vi.fn(), revalidatePath: vi.fn(), unstable_cache: <T>(fn: T) => fn,
}));
vi.mock("@/lib/groups/cache", () => ({ revalidateUserGroupLeaderboards: vi.fn() }));
vi.mock("@/lib/leaderboard/getLeaderboard", () => ({ getLeaderboardData: vi.fn(async () => ({})) }));

import { POST } from "@/app/api/submit/route";
import { db } from "@/lib/db";
import { validateSubmission } from "@/lib/validation/submission";
import { applyModerationAction, getModerationHistory } from "@/lib/moderation/actions";
import { getModerationCandidates } from "@/lib/moderation/candidates";

const integrationEnabled = process.env.SUBMISSION_WITHHOLDING_DB_INTEGRATION === "1";
const describeWithPostgres = integrationEnabled ? describe : describe.skip;
const fixtureIds: string[] = [];
type Day = SubmissionData["contributions"][number];
type Client = Day["clients"][number];

function database(): Sql {
  if (!state.client) throw new Error("Test database was not initialized");
  return state.client;
}
function tokens(input = 0, output = 0, cacheRead = 0): Day["tokenBreakdown"] {
  return { input, output, cacheRead, cacheWrite: 0, reasoning: 0 };
}
function client(client: Client["client"], modelId: string, t: Client["tokens"], cost = 0.01, messages?: number): Client {
  const total = Object.values(t).reduce((s, n) => s + n, 0);
  const msgCount = messages ?? Math.max(2, Math.ceil(total / 50_000));
  return { client, modelId, tokens: t, cost, messages: msgCount };
}
function payload(clients: Client[], opts: {
  device?: string; date?: string; metrics?: SubmissionData["timeMetrics"]; complete?: boolean;
} = {}): SubmissionData {
  const breakdown = tokens();
  for (const c of clients) {
    for (const k of Object.keys(breakdown) as Array<keyof typeof breakdown>) {
      breakdown[k] += c.tokens[k];
    }
  }
  const totalTokens = Object.values(breakdown).reduce((s, n) => s + n, 0);
  const totalCost = clients.reduce((s, c) => s + c.cost, 0);
  const totalMessages = clients.reduce((s, c) => s + c.messages, 0);
  const date = opts.date ?? "2026-04-30";
  return {
    meta: { generatedAt: "2026-05-01T00:00:00Z", version: "4.20.0", dateRange: { start: date, end: date } },
    device: { id: opts.device ?? "test-device" },
    summary: {
      totalTokens, totalCost, totalDays: 1, activeDays: totalTokens > 0 ? 1 : 0,
      averagePerDay: totalCost, maxCostInSingleDay: totalCost,
      clients: [...new Set(clients.map(c => c.client))], models: clients.map(c => c.modelId),
    },
    years: [{ year: date.slice(0, 4), totalTokens, totalCost, range: { start: date, end: date } }],
    contributions: [{
      date, clients, totals: { tokens: totalTokens, cost: totalCost, messages: totalMessages, ...(opts.complete === false ? { costIsComplete: false } : {}) },
      intensity: 1, tokenBreakdown: breakdown,
    }],
    ...(opts.metrics ? { timeMetrics: opts.metrics } : {}),
  };
}
async function account() {
  const id = randomUUID();
  const token = `tt_${randomUUID().replaceAll("-", "")}`;
  const username = `withholding${id.replaceAll("-", "").slice(0, 12)}`;
  const tokenId = randomUUID();
  const sql = database();
  fixtureIds.push(id);
  await sql`INSERT INTO users (id, github_id, username) VALUES (${id}, ${randomInt(1_000_000_000, 2_000_000_000)}, ${username})`;
  await sql`INSERT INTO api_tokens (id, user_id, token, name) VALUES (${tokenId}, ${id}, ${createHash("sha256").update(token).digest("hex")}, 'test')`;
  return { id, token, username };
}
async function submit(a: Awaited<ReturnType<typeof account>>, data: SubmissionData) {
  const validation = validateSubmission(data);
  expect(validation.errors, "payload must pass the real validator").toEqual([]);
  expect(validation.valid).toBe(true);
  const response = await POST(new Request("http://localhost/api/submit", {
    method: "POST", headers: { authorization: `Bearer ${a.token}`, "content-type": "application/json" },
    body: JSON.stringify(data),
  }));
  const body: unknown = await response.json();
  expect(response.status, JSON.stringify(body)).toBe(200);
  return body;
}
async function hidden(a: { id: string }) {
  const sql = database();
  const [r] = await sql`SELECT leaderboard_hidden FROM users WHERE id = ${a.id}`;
  return r.leaderboard_hidden;
}
async function signals(a: { id: string }) {
  const candidates = await getModerationCandidates();
  return candidates.find(c => c.userId === a.id)?.signals.map(s => s.key) ?? [];
}

// Uses a migrated, disposable PostgreSQL database. Only cache side effects are
// stubbed; authentication, validation, merging, SQL and locking are real.
describeWithPostgres("submission withholding PostgreSQL integration", () => {
  beforeAll(() => {
    if (!process.env.DATABASE_URL) throw new Error("DATABASE_URL is required");
  });
  beforeEach(() => {
    vi.stubEnv("ENABLE_SUBMISSION_AUTOHIDE", "1");
    vi.spyOn(console, "warn").mockImplementation(() => {});
  });
  afterEach(() => {
    vi.unstubAllEnvs();
    vi.restoreAllMocks();
  });
  afterAll(async () => {
    const sql = database();
    for (const id of fixtureIds) await sql`DELETE FROM users WHERE id = ${id}`;
    await sql.end();
  });

  it("keeps ordinary usage visible", async () => {
    const a = await account();
    await submit(a, payload([client("claude", "claude-sonnet-4", tokens(1000, 1000))], {
      metrics: { totalActiveTimeMs: 60_000, longestContinuousMs: 60_000, sessionCount: 1, maxConcurrentSessions: 1 },
    }));
    expect(await hidden(a)).toBe(false);
  });

  it("atomically withholds excessive daily output and records only one action", async () => {
    const a = await account();
    const data = payload([client("claude", "claude-sonnet-4", tokens(1000, 20_000_000_000), 1000)]);
    await submit(a, data);
    await submit(a, data);
    expect(await hidden(a)).toBe(true);
    const sql = database();
    const actions = await sql`SELECT * FROM moderation_actions WHERE target_user_id = ${a.id}`;
    expect(actions).toHaveLength(1);
    expect(actions[0].reason).toBe("Under investigation");
    expect(await signals(a)).toContain("usageLimit");
  });

  it("observes by default and keeps limit violations available in the review queue", async () => {
    vi.stubEnv("ENABLE_SUBMISSION_AUTOHIDE", undefined);
    const a = await account();
    await submit(a, payload([client("claude", "claude-sonnet-4", tokens(1000, 20_000_000_000), 1000)]));
    expect(await hidden(a)).toBe(false);
    expect(await signals(a)).toContain("usageLimit");
    const sql = database();
    expect(await sql`SELECT id FROM moderation_actions WHERE target_user_id = ${a.id}`).toHaveLength(0);
    expect(console.warn).toHaveBeenCalledWith(expect.stringContaining('"mode":"observe"'));
    expect(console.warn).toHaveBeenCalledWith(expect.stringContaining('"applied":false'));
  });

  it("respects an administrator restore on subsequent submissions", async () => {
    const a = await account();
    const admin = await account();
    const data = payload([client("claude", "claude-sonnet-4", tokens(1000, 20_000_000_000), 1000)]);
    await submit(a, data);
    const restored = await applyModerationAction({
      target: { id: a.id, username: a.username, leaderboardHidden: true },
      actorUserId: admin.id, actorUsername: admin.username, action: "unhide", reason: "Restored after review",
    });
    expect(restored.changed).toBe(true);
    await submit(a, data);
    expect(await hidden(a)).toBe(false);
    const sql = database();
    expect(await sql`SELECT id FROM moderation_actions WHERE target_user_id = ${a.id}`).toHaveLength(2);
  });

  it("orders decisions by the state change when an earlier administrator transaction is delayed", async () => {
    const a = await account();
    const admin = await account();
    let started!: () => void;
    let release!: () => void;
    const startedPromise = new Promise<void>(r => { started = r; });
    const releasePromise = new Promise<void>(r => { release = r; });
    const transaction = db.transaction.bind(db);
    // Model a scheduling delay after BEGIN. The production helper still
    // performs every moderation query and holds the real user row lock.
    vi.spyOn(db, "transaction").mockImplementationOnce((callback, config) =>
      transaction(async tx => {
        await tx.execute(drizzleSql`SELECT transaction_timestamp()`);
        started();
        await releasePromise;
        return callback(tx);
      }, config)
    );
    const restore = applyModerationAction({
      target: { id: a.id, username: a.username, leaderboardHidden: true },
      actorUserId: admin.id, actorUsername: admin.username,
      action: "unhide", reason: "Restored after review",
    });
    await startedPromise;
    const data = payload([client("claude", "claude-sonnet-4", tokens(1000, 20_000_000_000), 1000)]);
    try {
      await submit(a, data);
      expect(await hidden(a)).toBe(true);
    } finally {
      release();
      expect((await restore).changed).toBe(true);
    }
    expect(await hidden(a)).toBe(false);
    const history = await getModerationHistory(a.id);
    expect(history.map(row => row.action)).toEqual(["unhide", "hide"]);
    const candidate = (await getModerationCandidates()).find(row => row.userId === a.id);
    expect(candidate?.latestActorUsername).toBe(admin.username);
    expect(candidate?.latestAction).toBe("unhide");
    await submit(a, data);
    expect(await hidden(a)).toBe(false);
    expect(await getModerationHistory(a.id)).toHaveLength(2);
  });

  it("disables evaluation with the kill switch", async () => {
    vi.stubEnv("ENABLE_SUBMISSION_AUTOHIDE", "0");
    const a = await account();
    await submit(a, payload([client("claude", "claude-sonnet-4", tokens(1000, 20_000_000_000), 1000)]));
    expect(await hidden(a)).toBe(false);
    expect(console.warn).not.toHaveBeenCalled();
  });

  it("combines same-day device output for both withholding and the review queue", async () => {
    const a = await account();
    const cs = [client("claude", "claude-sonnet-4", tokens(1000, 6_000_000_000), 1000)];
    await submit(a, payload(cs, { device: "device-1" }));
    expect(await hidden(a)).toBe(false);
    await submit(a, payload(cs, { device: "device-2" }));
    expect(await hidden(a)).toBe(true);
    expect(await signals(a)).toContain("usageLimit");
  });

  it("does not combine different dates into one daily output limit", async () => {
    const a = await account();
    const cs = [client("claude", "claude-sonnet-4", tokens(1000, 6_000_000_000), 1000)];
    await submit(a, payload(cs, { date: "2026-04-29" }));
    await submit(a, payload(cs, { date: "2026-04-30" }));
    expect(await hidden(a)).toBe(false);
    expect(await signals(a)).not.toContain("usageLimit");
  });

  it("allows a concurrent administrator restore to win over a waiting submit", async () => {
    const a = await account();
    const admin = await account();
    const data = payload([client("claude", "claude-sonnet-4", tokens(1000, 20_000_000_000), 1000)]);
    await submit(a, data);
    let started!: () => void;
    let release!: () => void;
    const startedPromise = new Promise<void>(r => { started = r; });
    const releasePromise = new Promise<void>(r => { release = r; });
    const sql = database();
    const restore = sql.begin(async (tx: TransactionSql) => {
      await tx`UPDATE users SET leaderboard_hidden = false WHERE id = ${a.id}`;
      await tx`INSERT INTO moderation_actions (target_user_id, target_username, actor_user_id, actor_username, action, reason) VALUES (${a.id}, ${a.username}, ${admin.id}, ${admin.username}, 'unhide', 'Restored after review')`;
      started();
      await releasePromise;
    });
    await startedPromise;
    const inFlightSubmit = submit(a, data);
    try {
      let waiting = false;
      const deadline = Date.now() + 5000;
      while (Date.now() < deadline) {
        const [row] = await sql`SELECT EXISTS (SELECT 1 FROM pg_stat_activity WHERE datname = current_database() AND wait_event_type = 'Lock' AND query LIKE '%users%') AS waiting`;
        if (row.waiting) { waiting = true; break; }
        await new Promise(r => setTimeout(r, 10));
      }
      expect(waiting).toBe(true);
    } finally {
      release();
      await restore;
      await inFlightSubmit;
    }
    expect(await hidden(a)).toBe(false);
    expect(await sql`SELECT id FROM moderation_actions WHERE target_user_id = ${a.id}`).toHaveLength(2);
  }, 10_000);

  it("accepts the CLI's gap-merged continuous-time metrics", async () => {
    const a = await account();
    // sessionize.rs::test_longest_continuous_picks_max_active returns these.
    await submit(a, payload([client("opencode", "claude-sonnet-4", tokens(1000, 1000))], {
      metrics: { totalActiveTimeMs: 180_000, longestContinuousMs: 299_999, sessionCount: 2, maxConcurrentSessions: 1 },
    }));
    expect(await hidden(a)).toBe(false);
    expect(await signals(a)).not.toContain("outputRate");
  });

  it("excludes tokenless Cursor tool-call costs from price review signals", async () => {
    const a = await account();
    await submit(a, payload([
      client("cursor", "premium-tool-call", tokens(), 100),
      client("claude", "claude-sonnet-4", tokens(500, 500), 0.005),
    ]));
    expect(await hidden(a)).toBe(false);
    expect(await signals(a)).not.toContain("impliedRate");
  });

  it("also excludes tokenless Cursor costs in the legacy scalar modelId layout", async () => {
    const a = await account();
    await submit(a, payload([
      client("cursor", "premium-tool-call", tokens(), 100),
      client("claude", "claude-sonnet-4", tokens(500, 500), 0.005),
    ]));
    const sql = database();
    await sql`UPDATE daily_breakdown
      SET source_breakdown = jsonb_set(source_breakdown, '{cursor}',
        ((source_breakdown->'cursor') - 'models') || '{"modelId":"premium-tool-call"}'::jsonb)
      WHERE submission_id = (SELECT id FROM submissions WHERE user_id = ${a.id})`;
    expect(await signals(a)).not.toContain("impliedRate");
  });

  it.each([
    { name: "historical scalar-only cost floor", nestedCost: 0, modelId: null },
    { name: "partial legacy model map", nestedCost: 10, modelId: "premium-tool-call" },
  ])("excludes the tokenless Cursor remainder from a $name", async ({ nestedCost, modelId }) => {
    const a = await account();
    await submit(a, payload([
      client("cursor", "premium-tool-call", tokens(), 100),
      client("claude", "claude-sonnet-4", tokens(500, 500), 0.005),
    ]));
    const sql = database();
    // Older cost floors raised only client.cost, leaving nested costs lower.
    await sql`UPDATE daily_breakdown SET source_breakdown = jsonb_set(
      source_breakdown, '{cursor,models,premium-tool-call,cost}', ${JSON.stringify(nestedCost)}::jsonb)
      WHERE submission_id = (SELECT id FROM submissions WHERE user_id = ${a.id})`;
    if (modelId) {
      await sql`UPDATE daily_breakdown SET source_breakdown = jsonb_set(
        source_breakdown, '{cursor,modelId}', ${JSON.stringify(modelId)}::jsonb)
        WHERE submission_id = (SELECT id FROM submissions WHERE user_id = ${a.id})`;
    }
    expect(await hidden(a)).toBe(false);
    expect(await signals(a)).not.toContain("impliedRate");
  });

  it("assigns a tokenless legacy cost remainder beside fully represented Cursor tokens", async () => {
    const a = await account();
    await submit(a, payload([
      client("cursor", "premium-tool-call", tokens(), 100),
      client("cursor", "claude-sonnet-4", tokens(500, 500), 0.005),
    ]));
    const sql = database();
    await sql`UPDATE daily_breakdown SET source_breakdown = jsonb_set(
      jsonb_set(source_breakdown, '{cursor,models,premium-tool-call,cost}', '10'::jsonb),
      '{cursor,modelId}', '"premium-tool-call"'::jsonb)
      WHERE submission_id = (SELECT id FROM submissions WHERE user_id = ${a.id})`;
    expect(await signals(a)).not.toContain("impliedRate");
  });

  it.each([
    { namedCost: 0.005, expectedSignal: false },
    { namedCost: 25, expectedSignal: true },
  ])("does not price an unnamed Cursor cost remainder but preserves named cost $namedCost", async ({ namedCost, expectedSignal }) => {
    const a = await account();
    await submit(a, payload([
      client("cursor", "premium-tool-call", tokens(), 100),
      client("cursor", "claude-sonnet-4", tokens(500, 500), namedCost),
    ]));
    const sql = database();
    await sql`UPDATE daily_breakdown SET source_breakdown = jsonb_set(
      source_breakdown, '{cursor,models,premium-tool-call,cost}', '10'::jsonb)
      WHERE submission_id = (SELECT id FROM submissions WHERE user_id = ${a.id})`;
    expect((await signals(a)).includes("impliedRate")).toBe(expectedSignal);
  });

  it("does not exempt a named Cursor cost remainder that also carries tokens", async () => {
    const a = await account();
    await submit(a, payload([client("cursor", "premium-tool-call", tokens(500, 500), 100)]));
    const sql = database();
    // The model map no longer covers the scalar's tokens or most of its cost.
    // modelsForHighWater assigns both remainders to the explicit modelId.
    await sql`UPDATE daily_breakdown SET source_breakdown = jsonb_set(
      source_breakdown, '{cursor}', jsonb_set(source_breakdown->'cursor',
        '{models,premium-tool-call}', '{"tokens":0,"input":0,"output":0,"cost":10}'::jsonb)
        || '{"modelId":"premium-tool-call"}'::jsonb)
      WHERE submission_id = (SELECT id FROM submissions WHERE user_id = ${a.id})`;
    expect(await signals(a)).toContain("impliedRate");
  });

  it("excludes tokenless costs when Cursor also has token-bearing models that day", async () => {
    const a = await account();
    await submit(a, payload([
      client("cursor", "premium-tool-call", tokens(), 100),
      client("cursor", "claude-sonnet-4", tokens(500, 500), 0.005),
    ]));
    expect(await hidden(a)).toBe(false);
    expect(await signals(a)).not.toContain("impliedRate");
  });

  it("does not exempt a premium-tool-call entry that actually carries tokens", async () => {
    const a = await account();
    await submit(a, payload([client("cursor", "premium-tool-call", tokens(500, 500), 100)]));
    expect(await hidden(a)).toBe(false);
    expect(await signals(a)).toContain("impliedRate");
  });

  it("keeps high token-attributed costs advisory", async () => {
    const a = await account();
    await submit(a, payload([client("claude", "claude-sonnet-4", tokens(500, 500), 100)]));
    expect(await hidden(a)).toBe(false);
    expect(await signals(a)).toContain("impliedRate");
  });

  it("keeps configured model-name matches advisory", async () => {
    const a = await account();
    await submit(a, payload([client("claude", "fake-model", tokens(500_000, 500_000), 10)]));
    expect(await hidden(a)).toBe(false);
    expect(await signals(a)).toContain("slopModelName");
  });

  it("does not compare unmeasured history against one new device's active minute", async () => {
    const a = await account();
    await submit(a, payload([client("cursor", "claude-sonnet-4", tokens(2_000_000, 2_000_000), 20)], { device: "old-device", date: "2026-04-01" }));
    expect(await hidden(a)).toBe(false);
    await submit(a, payload([client("claude", "claude-sonnet-4", tokens(1000, 1000))], {
      device: "new-device", metrics: { totalActiveTimeMs: 60_000, longestContinuousMs: 60_000, sessionCount: 1, maxConcurrentSessions: 1 },
    }));
    expect(await hidden(a)).toBe(false);
    expect(await signals(a)).toContain("outputRate");
  });

  it("keeps separately filtered clients visible despite incomplete active-time coverage", async () => {
    const a = await account();
    const metrics = { totalActiveTimeMs: 100_000, longestContinuousMs: 100_000, sessionCount: 20, maxConcurrentSessions: 20 };
    await submit(a, payload([client("claude", "claude-sonnet-4", tokens(1_000_000, 600_000), 10)], { metrics }));
    await submit(a, payload([client("codex", "gpt-5", tokens(1_000_000, 600_000), 10)], { metrics }));
    const sql = database();
    const [r] = await sql`SELECT total_active_time_ms, output_tokens FROM submissions WHERE user_id = ${a.id}`;
    expect(Number(r.total_active_time_ms)).toBe(100_000);
    expect(Number(r.output_tokens)).toBe(1_200_000);
    expect(await hidden(a)).toBe(false);
    expect(await signals(a)).toContain("outputRate");
  });

  it("does not count cost-floor-preserved model maps as additional output", async () => {
    const a = await account();
    const metrics = { totalActiveTimeMs: 100_000, longestContinuousMs: 100_000, sessionCount: 20, maxConcurrentSessions: 20 };
    await submit(a, payload([client("claude", "claude-sonnet-4", tokens(1_000_000, 600_000), 10)], { metrics }));
    await submit(a, payload([client("claude", "claude-opus-4", tokens(1_000_000, 600_000), 0)], { metrics, complete: false }));
    const sql = database();
    const [r] = await sql`SELECT tokens, output_tokens, source_breakdown FROM daily_breakdown WHERE submission_id = (SELECT id FROM submissions WHERE user_id = ${a.id})`;
    expect(Number(r.output_tokens)).toBe(600_000);
    expect(Number(r.tokens)).toBe(1_600_000);
    expect(Object.keys(r.source_breakdown.claude.models)).toHaveLength(2);
    expect(await hidden(a)).toBe(false);
    expect(await signals(a)).not.toContain("outputRate");
    expect(await signals(a)).not.toContain("usageLimit");
  });

  it.each([
    { name: "output", usage: tokens(1_000_000, 6_000_000_000) },
    { name: "cached volume", usage: tokens(1_000_000, 1000, 300_000_000_000) },
  ])("does not cross the daily $name limit through retained model maps", async ({ usage }) => {
    const a = await account();
    await submit(a, payload([client("claude", "claude-sonnet-4", usage, 10)]));
    await submit(a, payload([client("claude", "claude-opus-4", usage, 0)], { complete: false }));
    const sql = database();
    const [r] = await sql`SELECT tokens, output_tokens, source_breakdown FROM daily_breakdown WHERE submission_id = (SELECT id FROM submissions WHERE user_id = ${a.id})`;
    expect(Number(r.tokens)).toBe(Object.values(usage).reduce((s, n) => s + n, 0));
    expect(Number(r.output_tokens)).toBe(usage.output);
    expect(Object.keys(r.source_breakdown.claude.models)).toHaveLength(2);
    expect(await hidden(a)).toBe(false);
    expect(await signals(a)).not.toContain("usageLimit");
  });

  it("atomically withholds physically impossible single-message token volume (1+ billion tokens / 1 msg)", async () => {
    const a = await account();
    const cs = [client("claude", "claude-sonnet-4", tokens(1_000_000_000, 500), 10, 1)];
    await submit(a, payload(cs));
    expect(await hidden(a)).toBe(true);
    expect(await signals(a)).toContain("usageLimit");
    expect(await signals(a)).toContain("tokensPerMessage");
    const [action] = await getModerationHistory(a.id);
    expect(action.action).toBe("hide");
    expect(action.actorUsername).toBe("system");
    expect(action.reason).toBe("Under investigation");
  });

  it("observes physically impossible single-message volume without hiding in observe mode", async () => {
    vi.stubEnv("ENABLE_SUBMISSION_AUTOHIDE", "observe");
    const a = await account();
    const cs = [client("claude", "claude-sonnet-4", tokens(1_200_000_000, 500), 10, 1)];
    await submit(a, payload(cs));
    expect(await hidden(a)).toBe(false);
    expect(await signals(a)).toContain("usageLimit");
    expect(await signals(a)).toContain("tokensPerMessage");
  });

  it("prevents dilution across multiple devices on the same date", async () => {
    const a = await account();
    const spoofedClient = [client("claude", "claude-sonnet-4", tokens(1_500_000_000, 500), 10, 1)];
    const normalClient = [client("codex", "gpt-4o", tokens(10_000, 5000), 1, 1000)];
    await submit(a, payload(spoofedClient, { device: "device-1" }));
    await submit(a, payload(normalClient, { device: "device-2" }));
    expect(await hidden(a)).toBe(true);
    expect(await signals(a)).toContain("usageLimit");
    expect(await signals(a)).toContain("tokensPerMessage");
  });

  it("flags impossible per-message volume when formatted in scientific notation in sourceBreakdown", async () => {
    const a = await account();
    const cs = [client("claude", "claude-sonnet-4", tokens(1000, 500), 10, 1)];
    await submit(a, payload(cs));
    const sql = database();
    await sql`
      UPDATE daily_breakdown
      SET source_breakdown = jsonb_set(
        source_breakdown,
        '{claude,tokens}',
        '"1.5e9"'::jsonb
      ),
      tokens = 1500000000
      WHERE submission_id = (SELECT id FROM submissions WHERE user_id = ${a.id})
    `;
    const candidates = await getModerationCandidates();
    const candidate = candidates.find(c => c.userId === a.id);
    expect(candidate).toBeDefined();
    expect(candidate!.maxTokensPerMessage).toBe(1_500_000_000);
    expect(candidate!.signals.map(s => s.key)).toContain("usageLimit");
    expect(candidate!.signals.map(s => s.key)).toContain("tokensPerMessage");
  });
});
