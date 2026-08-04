/**
 * Unguarded per-(device, date, client) snapshot of what the CLI last reported.
 *
 * Phase 4a of `docs/ratchet-inflation-recovery.md`.
 *
 * Written inside the submit transaction alongside `daily_breakdown`, but with
 * opposite merge semantics: last-write-wins, no `GREATEST`, no regression
 * guard, no alias-fold normalisation. **Nothing reads this table** — Phase 4b
 * is the only planned consumer, and it runs offline.
 */

import { sql, type SQL } from "drizzle-orm";
import { clientContributionToBreakdownData } from "./helpers";

/**
 * PostgreSQL caps a statement at 65,535 bound parameters. Each row binds 10,
 * so this stays clear even for a dense multi-client historical backfill.
 */
const UPSERT_CHUNK_SIZE = 1000;

export type DailyBreakdownReportedOrigin = "cli" | "backfill";

export interface DailyBreakdownReportedRow {
  date: string;
  client: string;
  tokens: number;
  cost: number;
  input: number;
  output: number;
  activeTimeMs: number | null;
  origin: DailyBreakdownReportedOrigin;
}

/**
 * Structural shape of one day of an already-validated payload. Kept structural
 * rather than importing the Zod-inferred type so this module can be exercised
 * without constructing a full `SubmissionData`.
 */
export interface DailyBreakdownReportedContribution {
  date: string;
  activeTimeMs?: number | null;
  clients: Array<{
    client: string;
    modelId: string;
    messages: number;
    cost: number;
    tokens: {
      input: number;
      output: number;
      cacheRead: number;
      cacheWrite: number;
      reasoning?: number;
    };
  }>;
}

export interface DailyBreakdownReportedExecutor {
  execute(query: SQL): Promise<unknown>;
}

function clampTokens(value: number): number {
  if (!Number.isFinite(value) || value <= 0) return 0;
  // Values arrive as JS numbers, so precision (not int8 range) is the binding
  // limit. Clamping at MAX_SAFE_INTEGER keeps the bound parameter exact and
  // stays far below int8 max.
  return Math.min(Math.round(value), Number.MAX_SAFE_INTEGER);
}

/**
 * numeric(14,4) holds up to 9999999999.9999 — matching `daily_breakdown.cost`.
 * `toFixed(4)` switches to exponential notation at 1e21, which would not parse
 * as numeric at all, so dishonest values are clamped before formatting.
 */
const COST_MAX = 9999999999;

function clampCost(value: number): number {
  if (!Number.isFinite(value) || value <= 0) return 0;
  return Math.min(value, COST_MAX);
}

function isIsoCalendarDate(date: string): boolean {
  if (!/^\d{4}-\d{2}-\d{2}$/.test(date)) return false;
  // Round-trip through UTC so impossible days (2026-02-31) are refused rather
  // than rolled into a neighbouring month the way `Date` parsing would.
  const parsed = new Date(`${date}T00:00:00.000Z`);
  if (Number.isNaN(parsed.getTime())) return false;
  return parsed.toISOString().slice(0, 10) === date;
}

/**
 * Fold a payload's daily contributions into one row per (date, client).
 *
 * Token arithmetic goes through `clientContributionToBreakdownData` — the same
 * helper the daily rows use — so the shadow and the guarded store cannot drift
 * apart on what counts as a token. Multiple models under one client on one day
 * sum into a single row, matching how the submit route builds
 * `incomingClientBreakdown` before the regression guard runs.
 *
 * `origin` is applied uniformly from the submission-level provenance tag,
 * matching how the submit route stamps `origin` into every client entry of a
 * backfill submission.
 */
export function foldContributionsIntoReportedRows(
  contributions: readonly DailyBreakdownReportedContribution[],
  origin: DailyBreakdownReportedOrigin
): DailyBreakdownReportedRow[] {
  const byKey = new Map<string, DailyBreakdownReportedRow>();

  for (const day of contributions) {
    if (!isIsoCalendarDate(day.date)) continue;
    const activeTimeMs =
      day.activeTimeMs == null || !Number.isFinite(day.activeTimeMs)
        ? null
        : Math.max(0, Math.round(day.activeTimeMs));

    for (const client_contrib of day.clients) {
      const modelData = clientContributionToBreakdownData(client_contrib);
      const key = `${day.date}\0${client_contrib.client}`;
      const existing = byKey.get(key);
      if (existing) {
        existing.tokens += modelData.tokens;
        existing.cost += modelData.cost;
        existing.input += modelData.input;
        existing.output += modelData.output;
        // Day-level: last non-null active time wins within one payload. A
        // contribution list should not carry the same date twice, but if it
        // does the later day's value is what the route would have used when
        // iterating `data.contributions` in order.
        if (activeTimeMs != null) existing.activeTimeMs = activeTimeMs;
      } else {
        byKey.set(key, {
          date: day.date,
          client: client_contrib.client,
          tokens: modelData.tokens,
          cost: modelData.cost,
          input: modelData.input,
          output: modelData.output,
          activeTimeMs,
          origin,
        });
      }
    }
  }

  return Array.from(byKey.values(), (row) => ({
    ...row,
    tokens: clampTokens(row.tokens),
    cost: clampCost(row.cost),
    input: clampTokens(row.input),
    output: clampTokens(row.output),
  }));
}

/**
 * Last-write-wins upsert of the unguarded per-(date, client) report.
 *
 * Deliberately NOT monotonic: a truthful lower rescan must replace the stored
 * shadow so Phase 4b can see the emptied-day / moved-day divergence that the
 * guarded `daily_breakdown` merge freezes over. That is also why this write
 * lives inside the submit transaction — it is the counterpart of the daily
 * write, not a deferred measurement.
 *
 * Returns the number of rows sent.
 */
export async function recordDailyBreakdownReported(params: {
  executor: DailyBreakdownReportedExecutor;
  submittedDeviceId: string;
  rows: readonly DailyBreakdownReportedRow[];
  now?: Date;
}): Promise<number> {
  const { executor, submittedDeviceId, rows } = params;
  if (rows.length === 0) return 0;

  const reportedAt = (params.now ?? new Date()).toISOString();

  for (let i = 0; i < rows.length; i += UPSERT_CHUNK_SIZE) {
    const chunk = rows.slice(i, i + UPSERT_CHUNK_SIZE);
    const valuesClauses = chunk.map(
      (row) =>
        sql`(${submittedDeviceId}::uuid, ${row.date}::date, ${row.client}, ${row.tokens}::bigint, ${row.cost.toFixed(4)}::numeric(14,4), ${row.input}::bigint, ${row.output}::bigint, ${row.activeTimeMs}::bigint, ${row.origin}, ${reportedAt}::timestamptz)`
    );

    await executor.execute(sql`
      INSERT INTO daily_breakdown_reported (
        submitted_device_id, date, client,
        tokens, cost, input, output, active_time_ms, origin, reported_at
      )
      VALUES ${sql.join(valuesClauses, sql`, `)}
      ON CONFLICT (submitted_device_id, date, client) DO UPDATE SET
        tokens = EXCLUDED.tokens,
        cost = EXCLUDED.cost,
        input = EXCLUDED.input,
        output = EXCLUDED.output,
        active_time_ms = EXCLUDED.active_time_ms,
        origin = EXCLUDED.origin,
        reported_at = EXCLUDED.reported_at
    `);
  }

  return rows.length;
}
