import { readFileSync } from "node:fs";
import { resolve } from "node:path";
import { afterEach, beforeAll, beforeEach, describe, expect, it, vi } from "vitest";
import { PgDialect } from "drizzle-orm/pg-core";
import { sql, type SQL } from "drizzle-orm";
import * as schema from "../../src/lib/db/schema";
import type { SubmissionData } from "../../src/lib/validation/submission";
import type { ClientBreakdownData } from "../../src/lib/db/helpers";
import { MICODE_FAMILY, MICODE_SUBMISSION_PARSER_VERSION } from "../../src/lib/db/micodeTransition";
import { LEGACY_DEVICE_KEY } from "../../src/lib/devices/shared";
import { DEVICE_CLIENT_TOTALS_WRITE_FLAG, type DeviceClientBucketTotal } from "../../src/lib/db/deviceClientTotals";

const mocks = vi.hoisted(() => ({ db: { transaction: vi.fn(), execute: vi.fn() }, auth: vi.fn() }));
vi.mock("@/lib/db", async () => ({
  ...await vi.importActual("../../src/lib/db/schema"), db: mocks.db,
}));
vi.mock("@/lib/auth/personalTokens", () => ({ authenticatePersonalToken: mocks.auth }));
vi.mock("next/cache", () => ({ revalidateTag: vi.fn() }));
vi.mock("@/lib/db/usernameLookup", () => ({
  normalizeUsernameCacheKey: (value: string) => value.toLowerCase(), revalidateUsernamePaths: vi.fn(),
}));
vi.mock("@/lib/groups/cache", () => ({ revalidateUserGroupLeaderboards: vi.fn() }));
vi.mock("@/lib/leaderboard/getLeaderboard", () => ({ getLeaderboardData: vi.fn() }));

// Real schema validation, route, merge code, and SQL construction. Only the
// transaction transport is doubled. SQL is decoded by Drizzle's own dialect,
// and device SELECT predicates actually select the corresponding ledger.
let POST: typeof import("../../src/app/api/submit/route")["POST"];
beforeAll(async () => { POST = (await import("../../src/app/api/submit/route")).POST; });
const dialect = new PgDialect();
type Breakdown = Record<string, ClientBreakdownData>;
type Day = { id: string; deviceId: string; date: string; sourceBreakdown: Breakdown; timestampMs: number | null; activeTimeMs: number | null };
type Device = { id: string; parserVersions: Record<string, number>; parserStates: Record<string, unknown> };
let days: Day[];
let devices: Map<string, Device>;
let sequence: number;
let hasSubmission: boolean;
function totals(rows = days) {
  const cells = rows.flatMap((day) => Object.values(day.sourceBreakdown));
  return { tokens: cells.reduce((s, c) => s + c.tokens, 0), cost: cells.reduce((s, c) => s + c.cost, 0) };
}
function decode(query: SQL) { return dialect.sqlToQuery(query); }

type Reported = { deviceId: string; date: string; client: string; tokens: number; cost: number; input: number; output: number; activeTimeMs: number | null; origin: string; reportedAt: string };
type CensusWork = { id: string; submissionId: string; submittedDeviceId: string; buckets: DeviceClientBucketTotal[] };
type CensusTotal = { deviceId: string; client: string; origin: string; width: string; key: string; tokens: number; cost: number; updatedAt: string };
let reported: Map<string, Reported>;
let work: Map<string, CensusWork>;
let highwater: Map<string, CensusTotal>;
let sqlFailures: string[];
let statementKinds: string[];
let transactionDepth: number;
let transport: ReturnType<typeof installStore>;
let savedSubmission: Record<string, unknown>;
let tokenLastUsedAt: Date | undefined;

// This is a bounded statement interpreter, not a permissive SQL mock. Match
// the COMPLETE statement contract (columns, bind casts, predicates, conflicts),
// ignoring only comments/whitespace and generated placeholder numbering. A new
// statement or changed write policy needs an explicit model and a regression.
// No raw control/advisory SQL is currently emitted. Savepoints use transaction()
// below; arbitrary SELECT/SET/lock statements are deliberately not accepted.
function normalized(statement: string): string {
  return statement.replace(/--[^\n]*/g, "").replace(/\$\d+/g, "?").replace(/\s+/g, " ").trim();
}
function valuesStatement(statement: string, params: unknown[], prefix: string, tuple: string, suffix: string, arity: number): boolean {
  if (!statement.startsWith(`${prefix} VALUES `)) return false;
  expect(params.length).toBeGreaterThan(0);
  expect(params.length % arity).toBe(0);
  expect(statement).toBe(`${prefix} VALUES ${Array(params.length / arity).fill(tuple).join(", ")}${suffix}`);
  return true;
}
function knownDevice(id: unknown): asserts id is string {
  expect([...devices.values()].some((device) => device.id === id)).toBe(true);
}
function maxNullable(a: number | null, b: number | null): number | null {
  return a == null ? b : b == null ? a : Math.max(a, b);
}
function checkedBreakdown(raw: unknown, tokens: unknown, cost: unknown, input: unknown, output: unknown, complete: unknown): Breakdown {
  const result = JSON.parse(raw as string) as Breakdown;
  const cells = Object.values(result);
  expect(tokens).toBe(cells.reduce((sum, cell) => sum + cell.tokens, 0));
  expect(Number(cost)).toBeCloseTo(cells.reduce((sum, cell) => sum + cell.cost, 0), 4);
  expect(input).toBe(cells.reduce((sum, cell) => sum + cell.input, 0));
  expect(output).toBe(cells.reduce((sum, cell) => sum + cell.output, 0));
  expect(complete).toBe(cells.every((cell) => cell.provenance?.costIsComplete !== false));
  return result;
}
async function executeSql(fragment: SQL): Promise<unknown[]> {
  const query = decode(fragment);
  const statement = normalized(query.sql);
  const params = query.params;
  try {
    if (valuesStatement(statement, params,
      "INSERT INTO daily_breakdown ( submission_id, submitted_device_id, date, tokens, cost, input_tokens, output_tokens, timestamp_ms, active_time_ms, source_breakdown, cost_is_complete )",
      "(?::uuid, ?::uuid, ?, ?::bigint, ?::numeric(14,4), ?::bigint, ?::bigint, ?::bigint, ?::bigint, ?::jsonb, ?::boolean)",
      " ON CONFLICT (submission_id, submitted_device_id, date) DO UPDATE SET tokens = EXCLUDED.tokens, cost = EXCLUDED.cost, cost_is_complete = EXCLUDED.cost_is_complete, input_tokens = EXCLUDED.input_tokens, output_tokens = EXCLUDED.output_tokens, timestamp_ms = EXCLUDED.timestamp_ms, active_time_ms = GREATEST(daily_breakdown.active_time_ms, EXCLUDED.active_time_ms), source_breakdown = EXCLUDED.source_breakdown", 11)) {
      statementKinds.push("daily-insert");
      for (let i = 0; i < params.length; i += 11) {
        const [submissionId, deviceId, date, tokens, cost, input, output, timestampMs, activeTimeMs, raw, complete] = params.slice(i, i + 11);
        expect(submissionId).toBe("submission-one"); knownDevice(deviceId);
        const sourceBreakdown = checkedBreakdown(raw, tokens, cost, input, output, complete);
        const existing = days.find((day) => day.deviceId === deviceId && day.date === date);
        if (existing) {
          existing.sourceBreakdown = sourceBreakdown;
          existing.timestampMs = timestampMs as number | null;
          existing.activeTimeMs = maxNullable(existing.activeTimeMs, activeTimeMs as number | null);
        } else {
          days.push({ id: `day-${++sequence}`, deviceId, date: date as string, sourceBreakdown, timestampMs: timestampMs as number | null, activeTimeMs: activeTimeMs as number | null });
        }
      }
    } else if (statement.startsWith("UPDATE daily_breakdown AS d SET ")) {
      expect(params.length).toBeGreaterThan(0); expect(params.length % 9).toBe(0);
      const tuple = "(?::uuid, ?::bigint, ?::numeric(14,4), ?::bigint, ?::bigint, ?::bigint, ?::bigint, ?::jsonb, ?::boolean)";
      expect(statement).toBe(`UPDATE daily_breakdown AS d SET tokens = batch.tokens, cost = batch.cost, cost_is_complete = batch.cost_is_complete, input_tokens = batch.input_tokens, output_tokens = batch.output_tokens, timestamp_ms = batch.timestamp_ms, active_time_ms = batch.active_time_ms, source_breakdown = batch.source_breakdown FROM (VALUES ${Array(params.length / 9).fill(tuple).join(", ")}) AS batch(id, tokens, cost, input_tokens, output_tokens, timestamp_ms, active_time_ms, source_breakdown, cost_is_complete) WHERE d.id = batch.id`);
      statementKinds.push("daily-update");
      for (let i = 0; i < params.length; i += 9) {
        const [id, tokens, cost, input, output, timestampMs, activeTimeMs, raw, complete] = params.slice(i, i + 9);
        const existing = days.find((day) => day.id === id);
        expect(existing).toBeDefined();
        existing!.sourceBreakdown = checkedBreakdown(raw, tokens, cost, input, output, complete);
        existing!.timestampMs = timestampMs as number | null;
        existing!.activeTimeMs = activeTimeMs as number | null;
      }
    } else if (statement.startsWith("DELETE FROM daily_breakdown WHERE id IN (")) {
      expect(params.length).toBeGreaterThan(0);
      expect(statement).toBe(`DELETE FROM daily_breakdown WHERE id IN (${params.map(() => "?::uuid").join(", ")})`);
      statementKinds.push("daily-delete");
      days = days.filter((day) => !params.includes(day.id));
    } else if (statement === normalized(`
      UPDATE daily_breakdown AS db SET submitted_device_id = ?
      WHERE db.submission_id = ? AND db.submitted_device_id IN (
        SELECT sd.id FROM submitted_devices AS sd WHERE sd.user_id = ? AND sd.device_key = ?
      ) AND NOT EXISTS (
        SELECT 1 FROM daily_breakdown AS modern WHERE modern.submission_id = db.submission_id
        AND modern.submitted_device_id NOT IN (
          SELECT sd2.id FROM submitted_devices AS sd2 WHERE sd2.user_id = ? AND sd2.device_key = ?
        )
      ) AND NOT EXISTS (
        SELECT 1 FROM daily_breakdown AS dup WHERE dup.submission_id = db.submission_id
        AND dup.submitted_device_id = ? AND dup.date = db.date
      )`)) {
      expect(transactionDepth).toBe(2); // production uses a savepoint
      const [target, submissionId, user, legacy, secondUser, secondLegacy, duplicateTarget] = params;
      expect(params).toHaveLength(7); knownDevice(target);
      expect([submissionId, user, legacy, secondUser, secondLegacy, duplicateTarget]).toEqual([
        "submission-one", "user-one", LEGACY_DEVICE_KEY, "user-one", LEGACY_DEVICE_KEY, target,
      ]);
      statementKinds.push("legacy-adoption");
      const legacyId = devices.get(LEGACY_DEVICE_KEY)?.id;
      if (!days.some((day) => day.deviceId !== legacyId)) {
        for (const day of days) {
          if (day.deviceId === legacyId && !days.some((dup) => dup.deviceId === target && dup.date === day.date)) day.deviceId = target;
        }
      }
    } else if (valuesStatement(statement, params,
      "INSERT INTO daily_breakdown_reported ( submitted_device_id, date, client, tokens, cost, input, output, active_time_ms, origin, reported_at )",
      "(?::uuid, ?::date, ?, ?::bigint, ?::numeric(14,4), ?::bigint, ?::bigint, ?::bigint, ?, ?::timestamptz)",
      " ON CONFLICT (submitted_device_id, date, client) DO UPDATE SET tokens = EXCLUDED.tokens, cost = EXCLUDED.cost, input = EXCLUDED.input, output = EXCLUDED.output, active_time_ms = EXCLUDED.active_time_ms, origin = EXCLUDED.origin, reported_at = EXCLUDED.reported_at", 10)) {
      expect(transactionDepth).toBe(1);
      statementKinds.push("reported-upsert");
      for (let i = 0; i < params.length; i += 10) {
        const [deviceId, date, client, tokens, cost, input, output, activeTimeMs, origin, reportedAt] = params.slice(i, i + 10);
        knownDevice(deviceId);
        reported.set(JSON.stringify([deviceId, date, client]), { deviceId, date: date as string, client: client as string, tokens: tokens as number, cost: Number(cost), input: input as number, output: output as number, activeTimeMs: activeTimeMs as number | null, origin: origin as string, reportedAt: reportedAt as string });
      }
    } else if (statement === "INSERT INTO ratchet_census_work (submission_id, submitted_device_id, buckets) VALUES ( ?::uuid, ?::uuid, ?::jsonb )") {
      expect(params).toHaveLength(3); expect(transactionDepth).toBe(1);
      const [submissionId, submittedDeviceId, raw] = params;
      expect(submissionId).toBe("submission-one"); knownDevice(submittedDeviceId);
      const buckets = JSON.parse(raw as string) as DeviceClientBucketTotal[];
      expect(buckets.length).toBeGreaterThan(0);
      const id = `work-${++sequence}`;
      work.set(id, { id, submissionId: submissionId as string, submittedDeviceId, buckets });
      statementKinds.push("census-enqueue");
    } else if (statement === 'SELECT id, submitted_device_id AS "submittedDeviceId", buckets FROM ratchet_census_work WHERE submission_id = ?::uuid') {
      expect(params).toEqual(["submission-one"]); expect(transactionDepth).toBe(0);
      statementKinds.push("census-read-work");
      return structuredClone([...work.values()].filter((item) => item.submissionId === params[0]));
    } else if (valuesStatement(statement, params,
      "INSERT INTO submitted_device_client_totals ( submitted_device_id, client, origin, bucket_width, bucket_key, tokens_highwater, cost_highwater, updated_at )",
      "(?::uuid, ?, ?, ?, ?, ?::bigint, ?::numeric(18,4), ?::timestamptz)",
      " ON CONFLICT (submitted_device_id, client, origin, bucket_width, bucket_key) DO UPDATE SET tokens_highwater = GREATEST(submitted_device_client_totals.tokens_highwater, EXCLUDED.tokens_highwater), cost_highwater = GREATEST(submitted_device_client_totals.cost_highwater, EXCLUDED.cost_highwater), updated_at = EXCLUDED.updated_at", 8)) {
      expect(transactionDepth).toBe(0);
      statementKinds.push("census-highwater-upsert");
      for (let i = 0; i < params.length; i += 8) {
        const [deviceId, client, origin, width, key, tokens, cost, updatedAt] = params.slice(i, i + 8);
        knownDevice(deviceId);
        const identity = JSON.stringify([deviceId, client, origin, width, key]);
        const prior = highwater.get(identity);
        highwater.set(identity, { deviceId, client: client as string, origin: origin as string, width: width as string, key: key as string, tokens: Math.max(prior?.tokens ?? 0, tokens as number), cost: Math.max(prior?.cost ?? 0, Number(cost)), updatedAt: updatedAt as string });
      }
    } else if (statement === "DELETE FROM ratchet_census_work WHERE id = ?::uuid") {
      expect(params).toHaveLength(1); expect(transactionDepth).toBe(0);
      work.delete(params[0] as string);
      statementKinds.push("census-delete-work");
    } else if (statement === normalized(`SELECT
      ( SELECT LEAST(COALESCE(SUM(db.tokens), 0), 9223372036854775807)::bigint FROM daily_breakdown AS db WHERE db.submission_id = ?::uuid ) AS "snapshotTokens",
      ( SELECT COALESCE(SUM(CAST(db.cost AS DECIMAL(14,4))), 0)::text FROM daily_breakdown AS db WHERE db.submission_id = ?::uuid ) AS "snapshotCost",
      ( SELECT COUNT(*) FROM ratchet_census_work AS w WHERE w.submission_id = ?::uuid )::int AS "censusPending",
      COUNT(*)::int AS "bucketCount", LEAST(COALESCE(SUM(t.tokens_highwater), 0), 9223372036854775807)::bigint AS "tokens", COALESCE(SUM(t.cost_highwater), 0)::text AS "cost"
      FROM submitted_device_client_totals AS t JOIN submitted_devices AS d ON d.id = t.submitted_device_id
      WHERE d.user_id = ?::uuid AND t.bucket_width = ?`)) {
      expect(params).toEqual(["submission-one", "submission-one", "submission-one", "user-one", "month"]);
      expect(transactionDepth).toBe(0);
      statementKinds.push("census-dual-read");
      const buckets = [...highwater.values()].filter((bucket) => bucket.width === params[4]);
      const snapshot = totals();
      return [{ snapshotTokens: snapshot.tokens, snapshotCost: snapshot.cost.toFixed(4), censusPending: work.size,
        bucketCount: buckets.length, tokens: buckets.reduce((sum, bucket) => sum + bucket.tokens, 0), cost: buckets.reduce((sum, bucket) => sum + bucket.cost, 0).toFixed(4) }];
    } else {
      throw new Error(`Unexpected SQL statement: ${statement}`);
    }
    return [];
  } catch (error) {
    // Production intentionally swallows post-commit census errors. Fail the
    // test nevertheless if an unsupported statement reached this transport.
    sqlFailures.push(statement);
    throw error;
  }
}
async function inTransaction<T>(callback: () => Promise<T>): Promise<T> {
  const previous = {
    ...structuredClone({ days, devices, sequence, hasSubmission, reported, work, highwater, tokenLastUsedAt }),
    // Drizzle expressions are immutable SQL objects, not structured-cloneable.
    savedSubmission: { ...savedSubmission },
  };
  transactionDepth++;
  try { return await callback(); } catch (error) {
    ({ days, devices, sequence, hasSubmission, reported, work, highwater, savedSubmission, tokenLastUsedAt } = previous);
    throw error;
  } finally { transactionDepth--; }
}
function installStore() {
  const tx = {
    select(columns: Record<string, unknown>) {
      let predicate: SQL;
      const result = () => {
        if ("id" in columns && "date" in columns && "sourceBreakdown" in columns) {
          const query = decode(predicate);
          expect(query.sql).toContain('"daily_breakdown"."submitted_device_id"');
          const deviceId = query.params.find((value) => [...devices.values()].some((d) => d.id === value));
          expect(deviceId).toBeDefined();
          return days.filter((day) => day.deviceId === deviceId);
        }
        if ("sourceBreakdown" in columns) return days;
        if ("totalTokens" in columns) {
          const sum = totals();
          return [{ totalTokens: sum.tokens, totalCost: sum.cost.toFixed(4), inputTokens: sum.tokens, outputTokens: 0,
            dateStart: days[0]?.date ?? null, dateEnd: days.at(-1)?.date ?? null, activeDays: days.length, rowCount: days.length, costIsComplete: true }];
        }
        if ("id" in columns && "sessionCount" in columns) return hasSubmission ? [{ id: "submission-one" }] : [];
        if ("sessionCount" in columns) return [{}];
        if ("leaderboardHidden" in columns) return [{ leaderboardHidden: false }];
        if ("action" in columns && "actorUsername" in columns) return [];
        throw new Error(`Unexpected select: ${Object.keys(columns)}`);
      };
      const builder = {
        from: () => builder, for: () => builder, limit: () => builder, orderBy: () => builder,
        where: (sql: SQL) => { predicate = sql; return builder; },
        then: (resolve: (value: unknown) => unknown) => Promise.resolve(result()).then(resolve),
      };
      return builder;
    },
    insert(table: unknown) {
      if (table === schema.submissions) {
        return {
          values: (value: Record<string, unknown>) => {
            expect(value.dateStart).toMatch(/^\d{4}-\d{2}-\d{2}$/);
            expect(value.dateEnd).toMatch(/^\d{4}-\d{2}-\d{2}$/);
            hasSubmission = true;
            return { returning: async () => [{ id: "submission-one" }] };
          },
        };
      }
      expect(table).toBe(schema.submittedDevices);
      let key: string;
      const builder = {
        values: (value: { deviceKey: string }) => { key = value.deviceKey; return builder; },
        onConflictDoUpdate: () => builder,
        returning: async () => {
          if (!devices.has(key)) devices.set(key, { id: `device-${key}`, parserVersions: {}, parserStates: {} });
          return [devices.get(key)!];
        },
      };
      return builder;
    },
    update(table: unknown) {
      let value: Record<string, unknown>;
      const builder = {
        set: (payload: Record<string, unknown>) => { value = payload; return builder; },
        where: async (predicate: SQL) => {
          if (table === schema.apiTokens) {
            expect(decode(predicate).params).toEqual(["token-one"]);
            expect(Object.keys(value)).toEqual(["lastUsedAt"]);
            expect(value.lastUsedAt).toBeInstanceOf(Date);
            tokenLastUsedAt = value.lastUsedAt as Date;
            return;
          }
          if (table === schema.submissions) {
            expect(decode(predicate).params).toEqual(["submission-one"]);
            expect(value.dateStart).toMatch(/^\d{4}-\d{2}-\d{2}$/);
            expect(value.dateEnd).toMatch(/^\d{4}-\d{2}-\d{2}$/);
            savedSubmission = { ...value };
            return;
          }
          if (table !== schema.submittedDevices || !("parserVersions" in value) || !("parserStates" in value)) {
            throw new Error("Unexpected ORM update target or payload");
          }
          const device = [...devices.values()].find((d) => decode(predicate).params.includes(d.id));
          expect(device).toBeDefined();
          device!.parserVersions = structuredClone(value.parserVersions) as Device["parserVersions"];
          device!.parserStates = structuredClone(value.parserStates) as Device["parserStates"];
        },
      };
      return builder;
    },
    execute: executeSql,
    async transaction(callback: (transaction: object) => Promise<unknown>): Promise<unknown> {
      return inTransaction(() => callback(tx));
    },
  };
  mocks.db.transaction.mockImplementation((callback: (transaction: typeof tx) => Promise<unknown>) => inTransaction(() => callback(tx)));
  mocks.db.execute.mockImplementation(executeSql);
  return tx;
}
beforeEach(() => {
  days = []; devices = new Map(); sequence = 0; hasSubmission = true;
  reported = new Map(); work = new Map(); highwater = new Map();
  sqlFailures = []; statementKinds = []; transactionDepth = 0;
  savedSubmission = {}; tokenLastUsedAt = undefined;
  vi.stubEnv(DEVICE_CLIENT_TOTALS_WRITE_FLAG, "false");
  mocks.db.execute.mockReset(); mocks.db.transaction.mockReset();
  mocks.auth.mockResolvedValue({ status: "valid", tokenId: "token-one", userId: "user-one", username: "alice" });
  transport = installStore();
});
afterEach(() => {
  vi.unstubAllEnvs();
  vi.restoreAllMocks();
  expect(sqlFailures, "unsupported SQL must fail even if production swallowed its error").toEqual([]);
  expect(transactionDepth).toBe(0);
});

type Cell = { client: SubmissionData["summary"]["clients"][number]; input: number; messages?: number; model?: string; date?: string; cost?: number; cacheRead?: number };
function payload(cells: Cell[], options: { versions?: Record<string, number> | null; fullHistory?: boolean; device?: string; incomplete?: boolean; backfill?: boolean } = {}): SubmissionData {
  const contributions: SubmissionData["contributions"] = [];
  for (const cell of cells) {
    const date = cell.date ?? "2026-08-01";
    let day = contributions.find((d) => d.date === date);
    if (!day) {
      day = { date, intensity: 0, totals: { tokens: 0, cost: 0, messages: 0, ...(options.incomplete ? { costIsComplete: false } : {}) }, tokenBreakdown: { input: 0, output: 0, cacheRead: 0, cacheWrite: 0, reasoning: 0 }, clients: [] };
      contributions.push(day);
    }
    const tokens = { input: cell.input, output: 0, cacheRead: cell.cacheRead ?? 0, cacheWrite: 0, reasoning: 0 };
    const cost = cell.cost ?? (cell.input + (cell.cacheRead ?? 0)) / 1_000_000;
    const messages = cell.messages ?? 1;
    day.clients.push({ client: cell.client, modelId: cell.model ?? "mimo-v2.5-pro", tokens, cost, messages });
    day.totals.tokens += tokens.input + tokens.cacheRead;
    day.totals.cost += cost; day.totals.messages += messages;
    day.tokenBreakdown.input += tokens.input; day.tokenBreakdown.cacheRead += tokens.cacheRead;
  }
  const tokens = contributions.reduce((s, d) => s + d.totals.tokens, 0);
  const cost = contributions.reduce((s, d) => s + d.totals.cost, 0);
  const dates = contributions.map((d) => d.date).sort();
  return {
    device: { id: options.device ?? "one" },
    meta: { generatedAt: "2026-08-02T00:00:00Z", version: "4.14.0", dateRange: { start: dates[0], end: dates.at(-1)! } },
    ...(options.versions === null ? {} : { scanScope: { parserVersions: options.versions ?? { micode: 2, "micode-desktop": 2 }, fullHistory: options.fullHistory ?? true } }),
    summary: { totalTokens: tokens, totalCost: cost, totalDays: contributions.length, activeDays: contributions.length, averagePerDay: cost / contributions.length, maxCostInSingleDay: Math.max(...contributions.map((d) => d.totals.cost)), clients: [...new Set(cells.map((c) => c.client))], models: [...new Set(cells.map((c) => c.model ?? "mimo-v2.5-pro"))] },
    contributions, years: [], ...(options.backfill ? { provenance: { origin: "backfill" } } : {}),
  };
}
async function submit(body: SubmissionData) {
  const response = await POST(new Request("http://localhost/api/submit", { method: "POST", headers: { authorization: "Bearer tt_valid", "content-type": "application/json" }, body: JSON.stringify(body) }));
  const json = await response.json();
  expect(response.status, JSON.stringify(json)).toBe(200);
  return json;
}
const split: Cell[] = [{ client: "micode", input: 500 }, { client: "micode-desktop", input: 1000 }];
async function seedLegacy() { await submit(payload([{ client: "micode", input: 1500, messages: 2 }], { versions: { micode: 1 } })); }
function cell(client: string, date = "2026-08-01", device = "device-one") {
  return days.find((d) => d.date === date && d.deviceId === device)?.sourceBreakdown[client];
}

describe("POST MiMo shared-store submission transition", () => {
  it("accepts the generation-2 wire contract through the real validator", async () => {
    const sender = readFileSync(resolve(__dirname, "../../../../crates/tokscale-cli/src/main.rs"), "utf8");
    expect(sender).toContain(`const MICODE_SUBMISSION_PARSER_VERSION: u32 = ${MICODE_SUBMISSION_PARSER_VERSION};`);
    hasSubmission = false;
    const result = await submit(payload(split));
    expect(result.metrics.totalTokens).toBe(1500);
    expect(devices.get("one")!.parserVersions).toEqual({ micode: 2, "micode-desktop": 2 });
    expect(cell("micode")!.tokens).toBe(500);
    expect(cell("micode-desktop")!.tokens).toBe(1000);
  });

  it("transfers legacy credit, replays identically, and credits covered growth exactly once", async () => {
    await seedLegacy();
    await submit(payload(split));
    expect(totals().tokens).toBe(1500);
    expect(cell("micode")!.tokens).toBe(500);
    expect(cell("micode-desktop")!.tokens).toBe(1000);
    const migrated = structuredClone(days);
    await submit(payload(split));
    expect(days).toEqual(migrated);
    const grown = payload([{ client: "micode", input: 700, messages: 2 }, split[1], { client: "micode-desktop", input: 300, date: "2026-08-02" }]);
    await submit(grown); await submit(grown);
    expect(totals().tokens).toBe(2000);
    expect(cell("micode")!.tokens).toBe(700);
    expect(cell("micode-desktop", "2026-08-02")!.tokens).toBe(300);
  });

  it("removes the old CLI label for entirely desktop history without doubling it", async () => {
    await seedLegacy();
    await submit(payload([{ client: "micode-desktop", input: 1500, messages: 2 }]));
    expect(cell("micode")).toBeUndefined();
    expect(cell("micode-desktop")!.tokens).toBe(1500);
    expect(totals().tokens).toBe(1500);
  });

  it.each([
    ["desktop-only", { versions: { "micode-desktop": 2 } }],
    ["CLI-only", { versions: { micode: 2 } }],
    ["date-filtered", { fullHistory: false }],
    ["unversioned", { versions: null }],
    ["original split generation", { versions: { micode: 1, "micode-desktop": 1 } }],
    ["unknown generation", { versions: { micode: 3, "micode-desktop": 3 } }],
    ["incomplete pricing", { incomplete: true }],
    ["backfill", { backfill: true }],
  ] as const)("freezes %s and makes the pending transition sticky", async (_name, options) => {
    await seedLegacy();
    const before = structuredClone(days);
    const result = await submit(payload(split, options));
    expect(days).toEqual(before);
    expect(result.warnings.join(" ")).toContain("No MiMo token or cost changes");
    expect(result.warnings.join(" ")).toContain("unfiltered");
    expect(devices.get("one")!.parserVersions.micode).toBe(2);
    // An old client cannot exploit a failed first transition to credit the
    // combined store alongside newly attributed desktop history.
    await submit(payload([{ client: "micode", input: 3000, messages: 3 }], { versions: { micode: 1 } }));
    expect(days).toEqual(before);
    await submit(payload(split));
    expect(cell("micode-desktop")!.tokens).toBe(1000);
  });

  it("does not add a surface-only generation-2 baseline on a new device", async () => {
    hasSubmission = false;
    const result = await submit(payload([split[0]], { versions: { micode: 2 } }));
    expect(totals().tokens).toBe(0);
    expect(result.warnings.join(" ")).toContain("both surfaces");
    await submit(payload(split));
    expect(totals().tokens).toBe(1500);
  });

  it("freezes truncated history even when unrelated model/day growth masks the lost total", async () => {
    await seedLegacy();
    const before = structuredClone(days);
    const result = await submit(payload([
      { client: "micode-desktop", input: 500 },
      { client: "micode", input: 5000, model: "new-model", date: "2026-08-02" },
    ]));
    expect(totals().tokens).toBe(1500);
    expect(days).toEqual(before);
    expect(result.warnings.join(" ")).toContain("credited MiMo day/model buckets");
  });

  it("rejects missing model or token-bucket coverage even with the same day total", async () => {
    await seedLegacy();
    for (const replacement of [
      { client: "micode-desktop" as const, input: 1500, messages: 2, model: "other-model" },
      { client: "micode-desktop" as const, input: 0, cacheRead: 1500, messages: 2 },
      { client: "micode-desktop" as const, input: 1500, messages: 1 },
    ]) {
      await submit(payload([replacement]));
      expect(cell("micode")!.tokens).toBe(1500);
      expect(cell("micode-desktop")).toBeUndefined();
    }
  });

  it("freezes rollback even if persisted generation markers are absent but desktop rows exist", async () => {
    await submit(payload(split));
    devices.get("one")!.parserVersions = {};
    await submit(payload([{ client: "micode", input: 1500, messages: 2 }], { versions: null }));
    expect(totals().tokens).toBe(1500);
    expect(cell("micode")!.tokens).toBe(500);
  });

  it("allows complete cost corrections but does not duplicate a cross-surface cost floor", async () => {
    await seedLegacy();
    await submit(payload(split.map((c) => ({ ...c, cost: 0 }))));
    expect(totals()).toEqual({ tokens: 1500, cost: 0 });
    expect(cell("micode")!.provenance?.costIsComplete).not.toBe(false);
  });

  it("freezes actual single-surface payloads without falsely filling in their sibling", async () => {
    await seedLegacy();
    for (const client of MICODE_FAMILY) {
      const result = await submit(payload([{ client, input: 2000, messages: 3 }], { versions: { [client]: 2 } }));
      expect(totals().tokens).toBe(1500);
      expect(result.warnings.join(" ")).toContain("both surfaces");
    }
  });

  it.each(["missing", "partial"])("preserves credited history with %s legacy model details", async (shape) => {
    await seedLegacy();
    const existing = cell("micode")!;
    existing.modelId = "legacy-model";
    if (shape === "missing") {
      existing.models = {};
    } else {
      existing.models["mimo-v2.5-pro"].tokens = 500;
      existing.models["mimo-v2.5-pro"].input = 500;
      existing.models["mimo-v2.5-pro"].messages = 1;
      existing.models["mimo-v2.5-pro"].cost = 0.0005;
    }
    const before = structuredClone(days);
    const wrong: Cell[] = shape === "missing"
      ? [{ client: "micode-desktop", input: 1500, messages: 2, model: "new-model" }]
      : [{ client: "micode-desktop", input: 500, messages: 1 },
         { client: "micode-desktop", input: 1000, messages: 1, model: "new-model" }];
    const result = await submit(payload(wrong));
    expect(days).toEqual(before);
    expect(result.warnings.join(" ")).toContain("No MiMo token or cost changes");
    const matching: Cell[] = shape === "missing"
      ? [{ client: "micode-desktop", input: 1500, messages: 2, model: "legacy-model" }]
      : [{ client: "micode-desktop", input: 500, messages: 1 },
         { client: "micode-desktop", input: 1000, messages: 1, model: "legacy-model" }];
    await submit(payload(matching));
    expect(cell("micode")).toBeUndefined();
    expect(cell("micode-desktop")!.models["legacy-model"].input).toBe(shape === "missing" ? 1500 : 1000);
    expect(totals().tokens).toBe(1500);
  });

  it("does not downgrade a stored future generation or clear its state", async () => {
    await submit(payload(split));
    const device = devices.get("one")!;
    device.parserVersions.micode = 3;
    device.parserStates.micode = { future: true };
    const before = structuredClone(days);
    await submit(payload([{ client: "micode-desktop", input: 2000, messages: 3 }]));
    expect(days).toEqual(before);
    expect(device.parserVersions.micode).toBe(3);
    expect(device.parserStates.micode).toEqual({ future: true });
  });

  it("keeps unrelated clients, parser states, and devices independent", async () => {
    await seedLegacy();
    const deviceOne = devices.get("one")!;
    deviceOne.parserVersions.other = 7; deviceOne.parserStates.other = { kept: true };
    await submit(payload([{ client: "claude", input: 20 }], { versions: { claude: 1 } }));
    const beforeOne = structuredClone(days);
    await submit(payload(split, { device: "two" }));
    expect(totals().tokens).toBe(3020);
    expect(days.filter((d) => d.deviceId === "device-one")).toEqual(beforeOne);
    const secondDevice = structuredClone(days.filter((d) => d.deviceId === "device-two"));
    // Frozen family does not freeze a healthy sibling client in the same POST.
    await submit(payload([...split, { client: "claude", input: 40 }], { versions: { "micode-desktop": 2, claude: 1 } }));
    expect(cell("micode")!.tokens).toBe(1500);
    expect(cell("claude")!.tokens).toBe(40);
    expect(days.filter((d) => d.deviceId === "device-two")).toEqual(secondDevice);
    await submit(payload(split));
    expect(cell("claude")!.tokens).toBe(40);
    expect(deviceOne.parserVersions.other).toBe(7);
    expect(deviceOne.parserStates.other).toEqual({ kept: true });
    expect(MICODE_FAMILY.every((client) => deviceOne.parserVersions[client] === 2)).toBe(true);
  });
});


describe("submission transaction transport", () => {
  it("rejects unknown writes and predicates instead of silently succeeding", async () => {
    await seedLegacy();
    const before = structuredClone(days);
    for (const statement of [
      "UPDATE unexpected_ledger SET tokens = 999",
      "DELETE FROM daily_breakdown WHERE true",
      "INSERT INTO daily_breakdown_reported (tokens) VALUES (999)",
      "SELECT 1",
    ]) {
      await expect(transport.execute(sql.raw(statement))).rejects.toThrow("Unexpected SQL statement");
      expect(sqlFailures.pop()).toBe(statement);
      expect(days).toEqual(before);
    }
  });

  it("rolls back route writes when a newly emitted statement is not modeled", async () => {
    await seedLegacy();
    const before = structuredClone({ days, devices, reported, work });
    const previousSubmission = savedSubmission;
    const errorLog = vi.spyOn(console, "error").mockImplementation(() => {});
    mocks.db.transaction.mockImplementationOnce((callback: (tx: typeof transport) => Promise<unknown>) =>
      inTransaction(async () => {
        await callback(transport); // all legitimate route writes execute first
        await transport.execute(sql`INSERT INTO unexpected_ledger (tokens) VALUES (${999})`);
      })
    );
    const response = await POST(new Request("http://localhost/api/submit", {
      method: "POST", headers: { authorization: "Bearer tt_valid", "content-type": "application/json" },
      body: JSON.stringify(payload(split)),
    }));
    expect(response.status).toBe(500);
    expect(errorLog).toHaveBeenCalledWith("Submit error:", expect.objectContaining({ message: expect.stringContaining("Unexpected SQL statement") }));
    expect(sqlFailures.pop()).toBe("INSERT INTO unexpected_ledger (tokens) VALUES (?)");
    expect({ days, devices, reported, work }).toEqual(before);
    expect(savedSubmission).toEqual(previousSubmission);
  });

  it("persists unguarded reported cells separately from a frozen family, using last-write-wins", async () => {
    await seedLegacy();
    const first = [...reported.values()][0];
    expect(first).toMatchObject({ deviceId: "device-one", client: "micode", tokens: 1500, input: 1500, origin: "cli" });
    await submit(payload([{ client: "micode-desktop", input: 300 }], { versions: { "micode-desktop": 2 }, backfill: true }));
    expect(totals().tokens).toBe(1500);
    expect([...reported.values()]).toEqual(expect.arrayContaining([
      expect.objectContaining({ client: "micode", tokens: 1500, origin: "cli" }),
      expect.objectContaining({ client: "micode-desktop", tokens: 300, origin: "backfill" }),
    ]));
    await submit(payload([{ client: "micode-desktop", input: 100 }], { versions: { "micode-desktop": 2 } }));
    expect(reported.size).toBe(2);
    expect([...reported.values()].find((cell) => cell.client === "micode-desktop")).toMatchObject({ tokens: 100, cost: 0.0001, input: 100, origin: "cli", activeTimeMs: null });
    expect([...reported.values()].every((cell) => Number.isFinite(Date.parse(cell.reportedAt)))).toBe(true);
    expect(totals().tokens).toBe(1500);
    expect(statementKinds.filter((kind) => kind === "reported-upsert")).toHaveLength(3);
  });

  it("adopts a legacy bucket inside a savepoint before planning the family transition", async () => {
    await submit(payload([{ client: "micode", input: 1500, messages: 2 }], { device: LEGACY_DEVICE_KEY, versions: { micode: 1 } }));
    expect(days[0].deviceId).toBe(`device-${LEGACY_DEVICE_KEY}`);
    await submit(payload(split));
    expect(days).toHaveLength(1);
    expect(days[0].deviceId).toBe("device-one");
    expect(totals().tokens).toBe(1500);
    expect(cell("micode")!.tokens).toBe(500);
    expect(cell("micode-desktop")!.tokens).toBe(1000);
    expect(statementKinds).toContain("legacy-adoption");
    expect(savedSubmission.totalTokens).toBe(1500);
    expect(tokenLastUsedAt).toBeInstanceOf(Date);
  });

  it("does not adopt ambiguous legacy rows once another modern device has usage", async () => {
    await submit(payload([{ client: "micode", input: 1500, messages: 2 }], { versions: { micode: 1 } }));
    await submit(payload([{ client: "micode", input: 900 }], { device: LEGACY_DEVICE_KEY, versions: { micode: 1 } }));
    const existing = structuredClone(days);
    await submit(payload(split, { device: "two" }));
    expect(days.filter((day) => day.deviceId !== "device-two")).toEqual(existing);
    expect(totals().tokens).toBe(3900);
    expect(days.some((day) => day.deviceId === `device-${LEGACY_DEVICE_KEY}`)).toBe(true);
  });

  it("models the route's explicit daily-row deletion when a covered parser layout moves dates", async () => {
    const options = { versions: { droid: 1 } };
    await submit(payload([{ client: "droid", input: 1500 }], options));
    const originalId = days[0].id;
    await submit(payload([{ client: "droid", input: 1500, date: "2026-08-02" }], options));
    expect(days).toHaveLength(1);
    expect(days[0]).toMatchObject({ date: "2026-08-02", deviceId: "device-one" });
    expect(days[0].id).not.toBe(originalId);
    expect(totals().tokens).toBe(1500);
    expect(statementKinds).toContain("daily-delete");
    // Observation storage is not a whole-history snapshot: omitted old dates
    // remain reported even when the guarded layout explicitly removes one.
    expect([...reported.values()].map((row) => row.date).sort()).toEqual(["2026-08-01", "2026-08-02"]);
  });

  it("records and replays enabled census writes without changing served family totals", async () => {
    vi.stubEnv(DEVICE_CLIENT_TOTALS_WRITE_FLAG, "true");
    const log = vi.spyOn(console, "log").mockImplementation(() => {});
    await seedLegacy();
    const result = await submit(payload(split));
    expect(result.metrics.totalTokens).toBe(1500);
    expect(work.size).toBe(0);
    expect([...highwater.values()]).toEqual(expect.arrayContaining([
      expect.objectContaining({ deviceId: "device-one", client: "micode", tokens: 1500, width: "month", key: "2026-08", origin: "cli" }),
      expect.objectContaining({ deviceId: "device-one", client: "micode-desktop", tokens: 1000, width: "month", key: "2026-08", origin: "cli" }),
    ]));
    // Census records observations, not the atomic transition's served ledger.
    // Its two per-client high-waters intentionally expose this divergence.
    const record = JSON.parse((log.mock.calls.at(-1)![0] as string).replace(/^ratchet-census /, ""));
    expect(record).toMatchObject({ servedTokens: 1500, snapshotTokens: 1500, highwaterTokens: 2500, censusPending: 0 });
    expect(statementKinds).toEqual(expect.arrayContaining([
      "census-enqueue", "census-read-work", "census-highwater-upsert", "census-delete-work", "census-dual-read",
    ]));
    expect(statementKinds.indexOf("reported-upsert")).toBeLessThan(statementKinds.indexOf("census-enqueue"));
    expect(statementKinds.indexOf("census-enqueue")).toBeLessThan(statementKinds.indexOf("census-read-work"));
  });

  it("keeps durable census work when post-commit transport fails and replays it on the next submit", async () => {
    vi.stubEnv(DEVICE_CLIENT_TOTALS_WRITE_FLAG, "true");
    vi.spyOn(console, "log").mockImplementation(() => {});
    const errorLog = vi.spyOn(console, "error").mockImplementation(() => {});
    mocks.db.execute.mockRejectedValueOnce(new Error("simulated post-commit disconnect"));
    await seedLegacy();
    expect(totals().tokens).toBe(1500);
    expect(work.size).toBe(1);
    expect([...work.values()][0]).toMatchObject({ submissionId: "submission-one", submittedDeviceId: "device-one",
      buckets: [expect.objectContaining({ client: "micode", tokens: 1500 })] });
    expect(highwater.size).toBe(0);
    expect(errorLog).toHaveBeenCalledWith("Ratchet census write failed (submission unaffected):", expect.objectContaining({ message: "simulated post-commit disconnect" }));
    await submit(payload(split));
    expect(work.size).toBe(0);
    expect(highwater.size).toBe(2);
    expect(totals().tokens).toBe(1500);
    expect(errorLog).toHaveBeenCalledTimes(1);
  });
});
