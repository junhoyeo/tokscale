import { NextResponse, after } from "next/server";
import { revalidateTag } from "next/cache";
import { db, apiTokens, submissions, submittedDevices, dailyBreakdown } from "@/lib/db";
import { and, eq, sql } from "drizzle-orm";
import {
  validateSubmission,
  generateSubmissionHash,
  type SubmissionData,
} from "@/lib/validation/submission";
import { authenticatePersonalToken } from "@/lib/auth/personalTokens";
import { getBearerToken } from "../../../lib/auth/bearerToken";
import {
  mergeClientBreakdownsWithRegressionGuard,
  recalculateDayTotals,
  clientContributionToBreakdownData,
  deriveClientBreakdownProvenance,
  mergeTimestampMs,
  breakdownCostIsComplete,
  tagBreakdownCostCompleteness,
  applyCostCompleteness,
  replaceLayoutCostFloors,
  reapplyReplaceLayoutCostFloors,
  type ClientBreakdownData,
} from "@/lib/db/helpers";
import {
  buildDualDerivationRecord,
  foldContributionsIntoBuckets,
  isDeviceClientTotalsWriteEnabled,
  readDualDerivation,
  recoverRatchetCensusWork,
  DUAL_DERIVATION_LOG_PREFIX,
} from "@/lib/db/deviceClientTotals";
import {
  foldContributionsIntoReportedRows,
  recordDailyBreakdownReported,
} from "@/lib/db/dailyBreakdownReported";
import {
  addClientBreakdownIncrement,
  foldParserClientSnapshot,
  planParserHighWaterSubmission,
  SUPPORTED_VERSIONED_PARSERS,
  type DeviceParserStates,
  type ParserHighWaterPlan,
} from "@/lib/db/parserHighWater";
import { SOURCE_DISPLAY_NAMES } from "@/lib/constants";
import { normalizeUsernameCacheKey, revalidateUsernamePaths } from "@/lib/db/usernameLookup";
import { getLeaderboardData } from "@/lib/leaderboard/getLeaderboard";
import { revalidateUserGroupLeaderboards } from "@/lib/groups/cache";
import { LEGACY_DEVICE_KEY } from "@/lib/devices/shared";
import { createSafeRecord, ownValue } from "@/lib/safeRecord";

const LEGACY_SUBMIT_DEVICE_KEY = LEGACY_DEVICE_KEY;
const LEGACY_SUBMIT_DEVICE_NAME = "Legacy submissions";
// PostgreSQL caps a single statement at 65,535 bound parameters. Each
// inserted row binds 11 params, so chunk large backfills (e.g. ~6,000+ days)
// across multiple INSERT statements to stay well under that limit.
const INSERT_CHUNK_SIZE = 1000;

// "kilocode" is a legacy alias of "kilo" (mirrors LEGACY_CLIENT_ALIASES in
// lib/validation/submission.ts and lib/publicProfileData.ts). Incoming
// contributions are already normalized to "kilo" by validateSubmission's
// preprocessing, but a daily_breakdown row's stored source_breakdown JSON
// can still carry a "kilocode" key written before that normalization
// existed. Fold it into "kilo" before merging so the day-level merge treats
// the two as the SAME client instead of summing them as disjoint clients
// (which would double-count the same underlying usage).
const LEGACY_CLIENT_ALIASES: Record<string, string> = { kilocode: "kilo" };

function emptyLayoutDay(date: string): SubmissionData["contributions"][number] {
  return {
    date,
    clients: [],
    totals: { tokens: 0, cost: 0, messages: 0 },
    intensity: 0,
    tokenBreakdown: {
      input: 0,
      output: 0,
      cacheRead: 0,
      cacheWrite: 0,
      reasoning: 0,
    },
  };
}

function isReplacePlan(plan: ParserHighWaterPlan): boolean {
  return plan.mode === "replace";
}

function applyReplaceLayouts(
  merged: Record<string, ClientBreakdownData>,
  date: string,
  parserPlans: Map<string, ParserHighWaterPlan>,
  incomingCostIsComplete: boolean
): void {
  for (const [client, plan] of parserPlans) {
    if (!isReplacePlan(plan) || !plan.layoutDays) continue;
    const next = ownValue(plan.layoutDays, date);
    if (next) {
      merged[client] = applyCostCompleteness(
        next,
        ownValue(merged, client),
        incomingCostIsComplete
      );
    } else {
      delete merged[client];
    }
  }
}

/** Returns the dates added here, i.e. the ones the submission never covered. */
function expandDaysForReplaceLayouts(
  daysToProcess: Map<string, SubmissionData["contributions"][number]>,
  parserPlans: Map<string, ParserHighWaterPlan>,
  existingDeviceDays: Array<{
    date: string;
    sourceBreakdown: unknown;
  }>
): Set<string> {
  const layoutOnlyDates = new Set<string>();
  for (const [client, plan] of parserPlans) {
    if (!isReplacePlan(plan) || !plan.layoutDays) continue;
    for (const date of Object.keys(plan.layoutDays)) {
      if (!daysToProcess.has(date)) {
        daysToProcess.set(date, emptyLayoutDay(date));
        layoutOnlyDates.add(date);
      }
    }
    for (const existing of existingDeviceDays) {
      const breakdown = existing.sourceBreakdown as Record<
        string,
        ClientBreakdownData
      > | null;
      if (
        breakdown &&
        ownValue(breakdown, client) &&
        !daysToProcess.has(existing.date)
      ) {
        daysToProcess.set(existing.date, emptyLayoutDay(existing.date));
        layoutOnlyDates.add(existing.date);
      }
    }
  }
  return layoutOnlyDates;
}

function mergeModelBreakdowns(
  target: Record<string, ClientBreakdownData["models"][string]>,
  incoming: Record<string, ClientBreakdownData["models"][string]>
): void {
  for (const [modelId, modelData] of Object.entries(incoming)) {
    const existingModel = ownValue(target, modelId);
    if (existingModel) {
      existingModel.tokens += modelData.tokens || 0;
      existingModel.cost += modelData.cost || 0;
      existingModel.input += modelData.input || 0;
      existingModel.output += modelData.output || 0;
      existingModel.cacheRead += modelData.cacheRead || 0;
      existingModel.cacheWrite += modelData.cacheWrite || 0;
      existingModel.reasoning = (existingModel.reasoning || 0) + (modelData.reasoning || 0);
      existingModel.messages += modelData.messages || 0;
    } else {
      // ModelBreakdownData is entirely scalar, so a spread is a full copy --
      // the merged model never shares state with `incoming`. Adding a nested
      // field to that interface would silently turn this back into an alias.
      target[modelId] = { ...modelData };
    }
  }
}

// `normalized` is mutated in place below (scalar `+=` on the client entry and
// mergeModelBreakdowns on its models), while the fold-preservation writeback
// in POST re-reads the ORIGINAL raw entries afterwards to restore the legacy
// alias keys. Those two only coexist if the normalized view owns its data
// outright: a shallow `{ ...data, models: { ...data.models } }` copies the
// client scalars and the models MAP but leaves the model VALUE objects shared
// with the stored breakdown, so folding a day mutated the very raw data the
// writeback then persisted -- and each later submit re-folded the
// already-folded values, compounding the nested models without bound. Client
// totals hid it because those are scalars the spread genuinely copied, which
// is why day totals and the leaderboard stayed correct while per-model views
// drifted.
function cloneClientBreakdownForFold(data: ClientBreakdownData): ClientBreakdownData {
  const models = createSafeRecord<ClientBreakdownData["models"][string]>();
  for (const [modelId, modelData] of Object.entries(data.models ?? {})) {
    models[modelId] = { ...modelData };
  }

  return {
    ...data,
    models,
    ...(data.provenance ? { provenance: { ...data.provenance } } : {}),
  };
}

type FoldableClientContribution = Parameters<
  typeof clientContributionToBreakdownData
>[0] & { client: string };

export function foldIncomingClientContributions(
  clients: FoldableClientContribution[]
): Record<string, ClientBreakdownData> {
  const incomingClientBreakdown = createSafeRecord<ClientBreakdownData>();
  for (const clientContribution of clients) {
    const modelData = clientContributionToBreakdownData(clientContribution);
    const existing = ownValue(incomingClientBreakdown, clientContribution.client);
    if (existing) {
      existing.tokens += modelData.tokens;
      existing.cost += modelData.cost;
      existing.input += modelData.input;
      existing.output += modelData.output;
      existing.cacheRead += modelData.cacheRead;
      existing.cacheWrite += modelData.cacheWrite;
      existing.reasoning = (existing.reasoning || 0) + modelData.reasoning;
      existing.messages += modelData.messages;
      const existingModel = ownValue(existing.models, clientContribution.modelId);
      if (existingModel) {
        existingModel.tokens += modelData.tokens;
        existingModel.cost += modelData.cost;
        existingModel.input += modelData.input;
        existingModel.output += modelData.output;
        existingModel.cacheRead += modelData.cacheRead;
        existingModel.cacheWrite += modelData.cacheWrite;
        existingModel.reasoning =
          (existingModel.reasoning || 0) + modelData.reasoning;
        existingModel.messages += modelData.messages;
      } else {
        existing.models[clientContribution.modelId] = { ...modelData };
      }
      existing.provenance = deriveClientBreakdownProvenance(existing);
    } else {
      const models = createSafeRecord<ClientBreakdownData["models"][string]>();
      models[clientContribution.modelId] = { ...modelData };
      const clientBreakdown = { ...modelData, models };
      incomingClientBreakdown[clientContribution.client] = {
        ...clientBreakdown,
        provenance: deriveClientBreakdownProvenance(clientBreakdown),
      };
    }
  }
  return incomingClientBreakdown;
}

interface NormalizedClientBreakdownAliases {
  breakdown: Record<string, ClientBreakdownData>;
  // Canonical client names where MULTIPLE raw source keys folded together
  // (e.g. a stale legacy "kilocode" key alongside "kilo" for the same
  // underlying usage, summed by this function), mapped to the largest token
  // count any single raw key contributed. The merge guard uses that value as
  // the healing floor: a truthful complete-day resubmit must report at least
  // as many tokens as the largest component of the fold, while anything
  // below it looks like a partial re-parse and keeps the normal regression
  // guard. A pure rename -- only the legacy key present, nothing to sum it
  // with -- is NOT included: that's a single contributor, not a suspect
  // double count, so the regression guard should still defend it normally.
  foldedClientFloors: Map<string, number>;
}

function normalizeClientBreakdownAliases(
  breakdown: Record<string, ClientBreakdownData>
): NormalizedClientBreakdownAliases {
  const normalized = createSafeRecord<ClientBreakdownData>();
  const foldedClients = new Set<string>();
  const largestComponentTokens = new Map<string, number>();

  for (const [rawClientName, data] of Object.entries(breakdown)) {
    const clientName = ownValue(LEGACY_CLIENT_ALIASES, rawClientName) ?? rawClientName;
    const existing = ownValue(normalized, clientName);

    largestComponentTokens.set(
      clientName,
      Math.max(largestComponentTokens.get(clientName) ?? 0, data.tokens || 0)
    );

    if (!existing) {
      normalized[clientName] = cloneClientBreakdownForFold(data);
      continue;
    }

    foldedClients.add(clientName);
    existing.tokens += data.tokens || 0;
    existing.cost += data.cost || 0;
    existing.input += data.input || 0;
    existing.output += data.output || 0;
    existing.cacheRead += data.cacheRead || 0;
    existing.cacheWrite += data.cacheWrite || 0;
    existing.reasoning = (existing.reasoning || 0) + (data.reasoning || 0);
    existing.messages += data.messages || 0;
    mergeModelBreakdowns(existing.models, data.models || {});
    existing.provenance = deriveClientBreakdownProvenance(existing);
  }

  const foldedClientFloors = new Map<string, number>();
  for (const clientName of foldedClients) {
    foldedClientFloors.set(clientName, largestComponentTokens.get(clientName) ?? 0);
  }

  return { breakdown: normalized, foldedClientFloors };
}

function normalizeSubmissionData(data: unknown): void {
  if (!data || typeof data !== "object") return;
  const obj = data as Record<string, unknown>;
  if (!Array.isArray(obj.contributions)) return;

  for (const contribution of obj.contributions) {
    if (!contribution || typeof contribution !== "object") continue;
    const day = contribution as Record<string, unknown>;
    // Handle both legacy "sources" and new "clients" formats
    const items = Array.isArray(day.sources)
      ? day.sources
      : Array.isArray(day.clients)
      ? day.clients
      : null;
    if (!items) continue;
    for (const entry of items) {
      if (!entry || typeof entry !== "object") continue;
      const s = entry as Record<string, unknown>;
      if (s.modelId == null || typeof s.modelId !== "string") {
        s.modelId = "unknown";
      } else {
        const trimmed = s.modelId.trim();
        s.modelId = trimmed === "" ? "unknown" : trimmed;
      }
    }
  }
}

// Submission schema versions:
//   0 = legacy CLI: no per-day timestamps, no device metadata.
//   1 = timestamp-aware CLI (>=v2.1): per-day `timestampMs` set, still no device.
//   2 = device-aware CLI (>=v2.1.x post-#517): caller sends a `device` object,
//       so daily_breakdown rows are keyed by submittedDeviceId.
// The submissions row keeps the GREATEST() of stored vs. incoming so a single
// device-aware submit cannot regress an account back to v1 hash semantics.
function getSubmitDevice(data: SubmissionData): { key: string; name: string | null; schemaVersion: number } {
  if (data.device) {
    return {
      key: data.device.id,
      name: data.device.name ?? null,
      schemaVersion: 2,
    };
  }

  return {
    key: LEGACY_SUBMIT_DEVICE_KEY,
    name: LEGACY_SUBMIT_DEVICE_NAME,
    schemaVersion: data.contributions.some((c) => c.timestampMs != null) ? 1 : 0,
  };
}

function isUniqueConstraintViolation(error: unknown): boolean {
  if (!error || typeof error !== "object") return false;
  const maybeError = error as { code?: unknown; cause?: unknown };
  if (maybeError.code === "23505") return true;
  const cause = maybeError.cause;
  return Boolean(cause && typeof cause === "object" && (cause as { code?: unknown }).code === "23505");
}

function mergeActiveTimeMs(
  existing: number | null | undefined,
  incoming: number | null | undefined,
): number | null {
  if (existing == null) return incoming ?? null;
  if (incoming == null) return existing;
  return Math.max(existing, incoming);
}

/**
 * Phase 1 + Phase 1.5 of docs/ratchet-inflation-recovery.md.
 *
 * Populates the per-device/client/bucket high-water table and records a
 * measurement. **This changes no value served to the caller** — it runs after
 * the response payload has already been computed, and its return value is
 * discarded.
 *
 * Placement is the whole point. The submit path above is one `db.transaction`
 * holding `.for('update')` on the submissions row, which serializes a user's
 * submits across every device until commit. A defect in a write placed there
 * fails the user's submission — "nothing reads it" is no protection. Because
 * the write is a `GREATEST` upsert it is idempotent, so running it after
 * commit downgrades any failure from a rejected submission to one deferred
 * measurement, which the next submit repairs. The cost is that the table can
 * lag the daily rows by one submit, which is why a missing bucket must read as
 * UNKNOWN and never as zero.
 *
 * Every failure mode is swallowed here on purpose.
 */
async function recordRatchetCensus(params: {
  userId: string;
  submissionId: string;
  servedTokens: number;
  servedCost: number;
  enabled: boolean;
}): Promise<void> {
  if (!params.enabled) return;

  try {
    await recoverRatchetCensusWork({
      executor: db,
      submissionId: params.submissionId,
    });

    // Phase 1.5: derive the total the OTHER way and record the pair. Both
    // derivations are read in ONE statement so they share a snapshot — pairing
    // this request's in-transaction total with a later high-water read would
    // report a divergence neither derivation had whenever a second device of
    // the same user commits in between. The value already SERVED is untouched
    // either way; it is recorded alongside as evidence of that.
    const { snapshotTokens, snapshotCost, censusPending, highwater } = await readDualDerivation({
      executor: db,
      userId: params.userId,
      submissionId: params.submissionId,
    });
    const record = buildDualDerivationRecord({
      userId: params.userId,
      submissionId: params.submissionId,
      servedTokens: params.servedTokens,
      servedCost: params.servedCost,
      snapshotTokens,
      snapshotCost,
      censusPending,
      highwater,
    });
    console.log(`${DUAL_DERIVATION_LOG_PREFIX} ${JSON.stringify(record)}`);
  } catch (e) {
    // A deferred measurement, not a failed submission. Durable work remains
    // available for the next enabled submit to replay.
    console.error("Ratchet census write failed (submission unaffected):", e);
  }
}

/**
 * POST /api/submit
 * Submit token usage data from CLI
 * 
 * IMPLEMENTS CLIENT-LEVEL MERGE:
 * - Only updates clients present in submission
 * - Preserves data for clients NOT in submission
 * - Recalculates totals from dailyBreakdown
 *
 * Headers:
 *   Authorization: Bearer <api_token>
 *
 * Body: TokenContributionData JSON
 */
export async function POST(request: Request) {
  try {
    // ========================================
    // STEP 1: Authentication
    // ========================================
    const token = getBearerToken(request.headers.get("Authorization"));
    if (!token) {
      return NextResponse.json(
        { error: "Missing or invalid Authorization header" },
        { status: 401 }
      );
    }

    const authResult = await authenticatePersonalToken(token, {
      touchLastUsedAt: false,
    });

    if (authResult.status === "invalid") {
      return NextResponse.json({ error: "Invalid API token" }, { status: 401 });
    }

    if (authResult.status === "expired") {
      return NextResponse.json({ error: "API token has expired" }, { status: 401 });
    }

    const tokenRecord = authResult;

    // ========================================
    // STEP 2: Parse and Validate
    // ========================================
    let rawData: unknown;
    try {
      rawData = await request.json();
    } catch {
      return NextResponse.json({ error: "Invalid JSON body" }, { status: 400 });
    }

    normalizeSubmissionData(rawData);

    const mcpServers: string[] | null =
      rawData != null && typeof rawData === "object" &&
      Array.isArray((rawData as Record<string, unknown>).mcpServers)
        ? ((rawData as Record<string, unknown>).mcpServers as unknown[]).filter(
            (s): s is string => typeof s === "string" && s.length > 0
          )
        : null;

    const validation = validateSubmission(rawData);

    if (!validation.valid || !validation.data) {
      return NextResponse.json(
        { error: "Validation failed", details: validation.errors },
        { status: 400 }
      );
    }

    const data = validation.data;
    const warnings = [...validation.warnings];

    // Phase 1 backfill-provenance persistence (issue #888): a submission
    // tagged `provenance.origin === "backfill"` (from `tokscale import`)
    // sets the sticky submissions.has_backfill flag and stamps a per-client
    // origin tag into daily_breakdown.source_breakdown. The tag is excluded
    // from generateSubmissionHash, so it never affects idempotency.
    const isBackfill = data.provenance?.origin === "backfill";

    if (data.contributions.length === 0) {
      return NextResponse.json(
        { error: "No contribution data to submit" },
        { status: 400 }
      );
    }

    const submittedClients = new Set<SubmissionData["summary"]["clients"][number]>(data.summary.clients);
    for (const contribution of data.contributions) {
      for (const client_contrib of contribution.clients) {
        submittedClients.add(client_contrib.client);
      }
    }
    if (submittedClients.has("kilo")) {
      submittedClients.add("kilocode" as SubmissionData["summary"]["clients"][number]);
    }
    const hashData: SubmissionData = {
      ...data,
      summary: {
        ...data.summary,
        clients: Array.from(submittedClients).sort(),
      },
    };

    // ========================================
    // STEP 3: DATABASE OPERATIONS IN TRANSACTION
    // ========================================
    const ratchetCensusEnabled = isDeviceClientTotalsWriteEnabled();
    const ratchetCensusBuckets = ratchetCensusEnabled
      ? foldContributionsIntoBuckets(data.contributions, isBackfill ? "backfill" : "cli")
      : [];
    // Phase 4a: unguarded per-(date, client) observations. Folded once up front
    // so the write sees the same pre-guard totals the merge loop builds into
    // `incomingClientBreakdown`, not the post-guard values that land in
    // `daily_breakdown`. Always on — nothing reads the table.
    const reportedRows = foldContributionsIntoReportedRows(
      data.contributions,
      isBackfill ? "backfill" : "cli"
    );
    const result = await db.transaction(async (tx) => {
      await tx
        .update(apiTokens)
        .set({ lastUsedAt: new Date() })
        .where(eq(apiTokens.id, tokenRecord.tokenId));

      // ------------------------------------------
      // STEP 3a: Get or create user's submission
      // ------------------------------------------
      const [existingSubmission] = await tx
        .select({
          id: submissions.id,
          totalActiveTimeMs: submissions.totalActiveTimeMs,
          longestContinuousMs: submissions.longestContinuousMs,
          maxConcurrentSessions: submissions.maxConcurrentSessions,
          sessionCount: submissions.sessionCount,
        })
        .from(submissions)
        .where(eq(submissions.userId, tokenRecord.userId))
        .for('update')
        .limit(1);

      let submissionId: string;
      let isNewSubmission = false;
      let storedSessionMetrics = existingSubmission;

      if (existingSubmission) {
        submissionId = existingSubmission.id;
      } else {
        try {
          const [newSubmission] = await tx.transaction(async (sp) =>
            sp
              .insert(submissions)
              .values({
                userId: tokenRecord.userId,
                totalTokens: 0,
                totalCost: "0",
                inputTokens: 0,
                outputTokens: 0,
                cacheCreationTokens: 0,
                cacheReadTokens: 0,
                dateStart: data.meta.dateRange.start,
                dateEnd: data.meta.dateRange.end,
                sourcesUsed: [],
                modelsUsed: [],
                cliVersion: data.meta.version,
                submissionHash: generateSubmissionHash(hashData),
                hasBackfill: isBackfill,
              })
              .returning({ id: submissions.id })
          );

          submissionId = newSubmission.id;
          isNewSubmission = true;
        } catch (creationErr) {
          if (!isUniqueConstraintViolation(creationErr)) {
            throw creationErr;
          }

          const [racedSubmission] = await tx
            .select({
              id: submissions.id,
              totalActiveTimeMs: submissions.totalActiveTimeMs,
              longestContinuousMs: submissions.longestContinuousMs,
              maxConcurrentSessions: submissions.maxConcurrentSessions,
              sessionCount: submissions.sessionCount,
            })
            .from(submissions)
            .where(eq(submissions.userId, tokenRecord.userId))
            .for('update')
            .limit(1);

          if (!racedSubmission) {
            throw creationErr;
          }

          submissionId = racedSubmission.id;
          storedSessionMetrics = racedSubmission;
        }
      }

      const submitDevice = getSubmitDevice(data);
      const submittedAt = new Date();
      // Session-shape metrics are recorded PER DEVICE as monotonic high-water
      // marks. The per-device max still protects against a truncated local
      // rescan (the d9df8c9c case), but keeping the metrics device-scoped is
      // what lets the submission-level values be derived additively below
      // instead of one device's snapshot overwriting another's.
      const incomingDeviceMetrics = data.timeMetrics
        ? {
            totalActiveTimeMs: data.timeMetrics.totalActiveTimeMs,
            longestContinuousMs: data.timeMetrics.longestContinuousMs,
            maxConcurrentSessions: data.timeMetrics.maxConcurrentSessions,
            sessionCount: data.timeMetrics.sessionCount,
          }
        : null;
      const [submittedDevice] = await tx
        .insert(submittedDevices)
        .values({
          userId: tokenRecord.userId,
          deviceKey: submitDevice.key,
          displayName: submitDevice.name,
          lastSubmittedAt: submittedAt,
          updatedAt: submittedAt,
          ...(incomingDeviceMetrics ?? {}),
        })
        .onConflictDoUpdate({
          target: [submittedDevices.userId, submittedDevices.deviceKey],
          set: {
            displayName: sql`COALESCE(EXCLUDED.display_name, ${submittedDevices.displayName})`,
            lastSubmittedAt: submittedAt,
            updatedAt: submittedAt,
            // A submit filtered by --clients/--date reports metrics for only
            // that slice, so it must never lower the device's stored value.
            // GREATEST ignores NULLs in Postgres, so a first-time metric write
            // onto a pre-migration row adopts the incoming value cleanly.
            ...(incomingDeviceMetrics
              ? {
                  totalActiveTimeMs: sql`GREATEST(${submittedDevices.totalActiveTimeMs}, EXCLUDED.total_active_time_ms)`,
                  longestContinuousMs: sql`GREATEST(${submittedDevices.longestContinuousMs}, EXCLUDED.longest_continuous_ms)`,
                  maxConcurrentSessions: sql`GREATEST(${submittedDevices.maxConcurrentSessions}, EXCLUDED.max_concurrent_sessions)`,
                  sessionCount: sql`GREATEST(${submittedDevices.sessionCount}, EXCLUDED.session_count)`,
                }
              : {}),
          },
        })
        .returning({
          id: submittedDevices.id,
          parserVersions: submittedDevices.parserVersions,
          parserStates: submittedDevices.parserStates,
        });

      // ------------------------------------------
      // STEP 3b: Fetch existing daily breakdown for merge
      // ------------------------------------------
      const fetchExistingDeviceDays = () =>
        tx
          .select({
            id: dailyBreakdown.id,
            date: dailyBreakdown.date,
            timestampMs: dailyBreakdown.timestampMs,
            activeTimeMs: dailyBreakdown.activeTimeMs,
            sourceBreakdown: dailyBreakdown.sourceBreakdown,
          })
          .from(dailyBreakdown)
          .where(
            and(
              eq(dailyBreakdown.submissionId, submissionId),
              eq(dailyBreakdown.submittedDeviceId, submittedDevice.id)
            )
          )
          .for('update');

      let existingDeviceDays = await fetchExistingDeviceDays();

      if (
        existingDeviceDays.length === 0 &&
        !isNewSubmission &&
        submitDevice.key !== LEGACY_SUBMIT_DEVICE_KEY
      ) {
        // The first device-aware submit after the migration should continue
        // the user's legacy bucket instead of counting the same history twice.
        // Once any modern device rows exist, attribution is ambiguous, so the
        // legacy bucket stays separate.
        //
        // Race note: two concurrent submits from the same user can both reach
        // this branch before either has committed. The second UPDATE will try
        // to re-stamp submitted_device_id on rows the first already claimed,
        // which can violate the (submission_id, submitted_device_id, date)
        // unique constraint. The NOT EXISTS dup guard below makes the UPDATE
        // skip rows that would collide, and the savepoint + outer try/catch
        // fall through to the normal insert path if a unique violation still
        // escapes (e.g. via a concurrent INSERT racing the UPDATE window).
        try {
          // Wrap the UPDATE in a savepoint so a unique-constraint violation
          // from a concurrent submit does not poison the enclosing
          // transaction. Drizzle's nested transaction maps to a Postgres
          // SAVEPOINT; throwing inside the inner block rolls back to the
          // savepoint and leaves the outer tx in a usable state.
          await tx.transaction(async (sp) => {
            await sp.execute(sql`
              UPDATE daily_breakdown AS db
              SET submitted_device_id = ${submittedDevice.id}
              WHERE db.submission_id = ${submissionId}
                AND db.submitted_device_id IN (
                  SELECT sd.id
                  FROM submitted_devices AS sd
                  WHERE sd.user_id = ${tokenRecord.userId}
                    AND sd.device_key = ${LEGACY_SUBMIT_DEVICE_KEY}
                )
                AND NOT EXISTS (
                  SELECT 1
                  FROM daily_breakdown AS modern
                  WHERE modern.submission_id = db.submission_id
                    AND modern.submitted_device_id NOT IN (
                      SELECT sd2.id
                      FROM submitted_devices AS sd2
                      WHERE sd2.user_id = ${tokenRecord.userId}
                        AND sd2.device_key = ${LEGACY_SUBMIT_DEVICE_KEY}
                    )
                )
                AND NOT EXISTS (
                  SELECT 1
                  FROM daily_breakdown AS dup
                  WHERE dup.submission_id = db.submission_id
                    AND dup.submitted_device_id = ${submittedDevice.id}
                    AND dup.date = db.date
                )
            `);
          });
        } catch (adoptionErr) {
          // Only a unique-constraint violation (23505) from a concurrent submit
          // racing this UPDATE is a recoverable fall-through: the savepoint
          // rolled back, the outer tx is still usable, and fetchExistingDeviceDays()
          // below picks up rows the other request already claimed so subsequent
          // logic merges rather than re-adopts.
          //
          // Any other failure (timeout, deadlock, permission error) leaves the
          // legacy rows unclaimed. Falling through would then insert the incoming
          // device's overlapping history as a SECOND row, silently inflating
          // totals. Re-throw so the request fails loudly instead of double-counting.
          if (!isUniqueConstraintViolation(adoptionErr)) {
            throw adoptionErr;
          }
          console.warn("Legacy adoption conflict (concurrent submit), falling through:", adoptionErr);
        }
        existingDeviceDays = await fetchExistingDeviceDays();
      }

      const deviceParserStates = (submittedDevice.parserStates ?? {}) as DeviceParserStates;
      // Every client whose parser can re-attribute already-submitted history
      // runs through the high-water path, not just Copilot. A re-attribution
      // moves a day's tokens without changing the lifetime total, and the
      // per-day merge guard defends each stored day against a decrease, so the
      // days that fall are pinned while the days that rise are written: the
      // device's stored total inflates by exactly what moved. Bounding each
      // submission by the device/client lifetime high-water makes a pure
      // reshuffle contribute nothing, which is the only reading of it that is
      // both non-destructive and non-inflating.
      const parserPlans = new Map<string, ParserHighWaterPlan>();
      for (const [client, supportedVersion] of Object.entries(
        SUPPORTED_VERSIONED_PARSERS
      )) {
        const wasScanned =
          submittedClients.has(client) ||
          Boolean(
            data.scanScope &&
              Object.prototype.hasOwnProperty.call(
                data.scanScope.parserVersions,
                client
              )
          );
        if (!wasScanned) continue;
        const existingClientDays = Object.fromEntries(
          existingDeviceDays.flatMap((day) => {
            const breakdown = day.sourceBreakdown as Record<string, ClientBreakdownData> | null;
            const stored = breakdown ? ownValue(breakdown, client) : undefined;
            return stored ? [[day.date, stored] as const] : [];
          })
        );
        const plan = planParserHighWaterSubmission({
          client,
          incomingVersion: isBackfill
            ? undefined
            : ownValue(
                data.scanScope?.parserVersions as
                  | Record<string, number>
                  | undefined,
                client
              ),
          fullHistory: data.scanScope?.fullHistory === true,
          existingLegacyDays: existingClientDays,
          incomingDays: foldParserClientSnapshot(data.contributions, client),
          state: ownValue(deviceParserStates, client),
          persistedVersion: ownValue(
            submittedDevice.parserVersions as Record<string, number> | undefined,
            client
          ),
        });
        if (plan.mode === "status-quo") continue;
        parserPlans.set(client, plan);
        const label = ownValue(SOURCE_DISPLAY_NAMES, client) ?? client;
        if (plan.mode === "baseline-legacy") {
          warnings.push(
            `Established the ${label} parser generation ${supportedVersion} baseline; existing same-device history was preserved and only bounded lifetime growth was added.`
          );
        } else if (plan.mode === "replace") {
          warnings.push(
            `Rewrote ${label} daily layout from the full parser snapshot without changing the lifetime high-water.`
          );
        } else if (plan.mode === "freeze") {
          warnings.push(
            `Ignored ${label} changes because this parser generation or partial snapshot cannot safely advance the device high-water.`
          );
        }
      }
      const plannedIncrementClients = [...parserPlans].filter(
        ([, plan]) =>
          plan.mode === "incremental" || plan.mode === "baseline-legacy"
      );
      const replaceClients = [...parserPlans]
        .filter(([, plan]) => isReplacePlan(plan))
        .map(([client]) => client);
      const replaceCostFloors = replaceLayoutCostFloors(
        existingDeviceDays,
        replaceClients
      );
      const incompleteReplaceClients = new Set<string>();

      const existingDaysMap = new Map(
        existingDeviceDays.map((d) => [d.date, d])
      );

      const daysToProcess = new Map(
        data.contributions.map((day) => [day.date, day] as const)
      );
      const layoutOnlyDates = expandDaysForReplaceLayouts(
        daysToProcess,
        parserPlans,
        existingDeviceDays
      );

      // ------------------------------------------
      // STEP 3c: Compute merge results in memory, then batch write
      // ------------------------------------------
      const toInsert: Array<{
        submissionId: string;
        submittedDeviceId: string;
        date: string;
        tokens: number;
        cost: string;
        inputTokens: number;
        outputTokens: number;
        timestampMs: number | null;
        activeTimeMs: number | null;
        sourceBreakdown: Record<string, ClientBreakdownData>;
        costIsComplete: boolean;
      }> = [];

      const toUpdate: Array<{
        id: string;
        tokens: number;
        cost: string;
        inputTokens: number;
        outputTokens: number;
        timestampMs: number | null;
        activeTimeMs: number | null;
        sourceBreakdown: Record<string, ClientBreakdownData>;
        costIsComplete: boolean;
      }> = [];

      const toDelete: string[] = [];

      for (const incomingDay of daysToProcess.values()) {
        if (incomingDay.totals?.costIsComplete === false) {
          for (const client of replaceClients) incompleteReplaceClients.add(client);
        }
        const incomingClientBreakdown = foldIncomingClientContributions(
          incomingDay.clients
        );

        if (isBackfill) {
          // Stamp the per-client origin tag AFTER provenance derivation so it
          // is persisted alongside the coverage metrics. The merge helper
          // re-derives provenance via deriveClientBreakdownProvenance, which
          // carries `origin` through, so the tag survives the merge path too.
          for (const clientBreakdown of Object.values(incomingClientBreakdown)) {
            clientBreakdown.provenance = {
              ...(clientBreakdown.provenance ??
                deriveClientBreakdownProvenance(clientBreakdown)),
              origin: "backfill",
            };
          }
        }

        for (const [client, plan] of parserPlans) {
          if (plan.mode === "freeze" || plan.mode === "replace") {
            delete incomingClientBreakdown[client];
          } else if (
            plan.mode === "incremental" ||
            plan.mode === "baseline-legacy"
          ) {
            const increment = ownValue(plan.increments, incomingDay.date);
            if (increment) {
              incomingClientBreakdown[client] = increment;
            } else {
              delete incomingClientBreakdown[client];
            }
          }
        }

        const clientsToMerge = new Set(submittedClients);
        // A day the submission never covered is in the write set only so a
        // replace layout can empty its own cell there. The other clients were
        // not resubmitted for that day at all, so their stored cells did not
        // disappear -- merging them would preserve them with a false warning.
        if (layoutOnlyDates.has(incomingDay.date)) clientsToMerge.clear();
        for (const [client, plan] of parserPlans) {
          if (plan.mode === "replace") {
            clientsToMerge.delete(client);
            continue;
          }
          const planRewroteClient =
            plan.mode === "freeze" ||
            plan.mode === "incremental" ||
            plan.mode === "baseline-legacy";
          if (planRewroteClient && !incomingClientBreakdown[client]) {
            clientsToMerge.delete(client);
          }
        }

        const existingDay = existingDaysMap.get(incomingDay.date);

        if (existingDay) {
          const rawExistingBreakdown = (existingDay.sourceBreakdown || {}) as Record<
            string,
            ClientBreakdownData
          >;
          const { breakdown: existingClientBreakdown, foldedClientFloors } =
            normalizeClientBreakdownAliases(rawExistingBreakdown);
          for (const [client] of plannedIncrementClients) {
            const increment = incomingClientBreakdown[client];
            if (increment) {
              incomingClientBreakdown[client] = addClientBreakdownIncrement(
                ownValue(existingClientBreakdown, client),
                increment
              );
            }
          }
          const mergeResult = mergeClientBreakdownsWithRegressionGuard(
            existingClientBreakdown,
            incomingClientBreakdown,
            clientsToMerge,
            foldedClientFloors,
            incomingDay.totals?.costIsComplete ?? true
          );
          warnings.push(
            ...mergeResult.warnings.map((warning) => `Day ${incomingDay.date}: ${warning}`)
          );
          if (
            existingDay.activeTimeMs != null &&
            incomingDay.activeTimeMs != null &&
            incomingDay.activeTimeMs < existingDay.activeTimeMs
          ) {
            warnings.push(
              `Day ${incomingDay.date}: Preserved ${existingDay.activeTimeMs}ms active time because this same-device resubmit reported only ${incomingDay.activeTimeMs}ms.`
            );
          }
          const mergedClientBreakdown = mergeResult.merged;
          for (const [client] of plannedIncrementClients) {
            const merged = mergedClientBreakdown[client];
            if (
              merged &&
              ownValue(existingClientBreakdown, client)?.provenance
                ?.costIsComplete === false
            ) {
              merged.provenance = {
                ...deriveClientBreakdownProvenance(merged),
                costIsComplete: false,
              };
            }
          }
          // A preserved fold must keep its ORIGINAL raw alias keys in storage
          // (e.g. both "kilocode" and "kilo"), not the collapsed sum: the
          // collapsed form is indistinguishable from real usage, so writing it
          // back would burn the heal floor on the first partial resubmit and
          // permanently re-cement the double count. Day totals are identical
          // either way (recalculateDayTotals sums all keys).
          for (const clientName of mergeResult.foldPreservedClients) {
            delete mergedClientBreakdown[clientName];
            for (const [rawKey, rawData] of Object.entries(rawExistingBreakdown)) {
              if ((LEGACY_CLIENT_ALIASES[rawKey] ?? rawKey) === clientName) {
                mergedClientBreakdown[rawKey] = rawData;
              }
            }
          }
          applyReplaceLayouts(
            mergedClientBreakdown,
            incomingDay.date,
            parserPlans,
            incomingDay.totals?.costIsComplete ?? true
          );
          if (Object.keys(mergedClientBreakdown).length === 0) {
            toDelete.push(existingDay.id);
            continue;
          }
          const dayTotals = recalculateDayTotals(mergedClientBreakdown);

          toUpdate.push({
            id: existingDay.id,
            tokens: dayTotals.tokens,
            cost: dayTotals.cost.toFixed(4),
            inputTokens: dayTotals.inputTokens,
            outputTokens: dayTotals.outputTokens,
            timestampMs: mergeTimestampMs(existingDay.timestampMs, incomingDay.timestampMs ?? null),
            activeTimeMs: mergeActiveTimeMs(existingDay.activeTimeMs, incomingDay.activeTimeMs),
            sourceBreakdown: mergedClientBreakdown,
            // Derived from the MERGED breakdown, not the incoming day: a
            // filtered resubmit naming only healthy clients must not clear a
            // preserved sibling's incompleteness. The merge has already
            // floored each incoming client's cost, so this agrees with the
            // scalar recomputed above by construction.
            costIsComplete: breakdownCostIsComplete(mergedClientBreakdown),
          });
        } else {
          // A day with no stored row has nothing to floor against, but its
          // clients still carry the tag forward for later merges.
          const insertedClientBreakdown = tagBreakdownCostCompleteness(
            incomingClientBreakdown,
            incomingDay.totals?.costIsComplete ?? true
          );
          applyReplaceLayouts(
            insertedClientBreakdown,
            incomingDay.date,
            parserPlans,
            incomingDay.totals?.costIsComplete ?? true
          );
          const dayTotals = recalculateDayTotals(insertedClientBreakdown);
          if (Object.keys(insertedClientBreakdown).length === 0) continue;

          toInsert.push({
            submissionId,
            submittedDeviceId: submittedDevice.id,
            date: incomingDay.date,
            tokens: dayTotals.tokens,
            cost: dayTotals.cost.toFixed(4),
            inputTokens: dayTotals.inputTokens,
            outputTokens: dayTotals.outputTokens,
            timestampMs: incomingDay.timestampMs ?? null,
            activeTimeMs: incomingDay.activeTimeMs ?? null,
            sourceBreakdown: insertedClientBreakdown,
            costIsComplete: breakdownCostIsComplete(insertedClientBreakdown),
          });
        }
      }

      reapplyReplaceLayoutCostFloors(
        [...toInsert, ...toUpdate],
        replaceCostFloors,
        incompleteReplaceClients
      );
      for (const row of [...toInsert, ...toUpdate]) {
        const dayTotals = recalculateDayTotals(row.sourceBreakdown);
        row.tokens = dayTotals.tokens;
        row.cost = dayTotals.cost.toFixed(4);
        row.inputTokens = dayTotals.inputTokens;
        row.outputTokens = dayTotals.outputTokens;
        row.costIsComplete = breakdownCostIsComplete(row.sourceBreakdown);
      }

      const advancedParserStates = [...parserPlans].flatMap(([client, plan]) =>
        plan.nextState ? [[client, plan.nextState] as const] : []
      );
      if (advancedParserStates.length > 0) {
        await tx
          .update(submittedDevices)
          .set({
            parserVersions: {
              ...(submittedDevice.parserVersions ?? {}),
              ...Object.fromEntries(
                advancedParserStates.map(([client, state]) => [
                  client,
                  state.version,
                ])
              ),
            },
            parserStates: {
              ...deviceParserStates,
              ...Object.fromEntries(advancedParserStates),
            },
          })
          .where(eq(submittedDevices.id, submittedDevice.id));
      }

      // Batch INSERT new days via raw SQL VALUES list, chunked to stay under
      // PostgreSQL's 65,535 bound-parameter limit (11 params/row here --
      // a large historical backfill can otherwise exceed it in one statement).
      // ON CONFLICT (submission_id, submitted_device_id, date) is a defensive
      // fallback for concurrent submits from the same device racing between
      // the SELECT above and this INSERT. Distinct devices own distinct rows,
      // so their independent usage remains additive.
      for (let i = 0; i < toInsert.length; i += INSERT_CHUNK_SIZE) {
        const chunk = toInsert.slice(i, i + INSERT_CHUNK_SIZE);
        const insertValuesClauses = chunk.map(
          (row) =>
            sql`(${row.submissionId}::uuid, ${row.submittedDeviceId}::uuid, ${row.date}, ${row.tokens}::bigint, ${row.cost}::numeric(14,4), ${row.inputTokens}::bigint, ${row.outputTokens}::bigint, ${row.timestampMs}::bigint, ${row.activeTimeMs}::bigint, ${JSON.stringify(row.sourceBreakdown)}::jsonb, ${row.costIsComplete}::boolean)`
        );

        const insertValuesList = sql.join(insertValuesClauses, sql`, `);

        await tx.execute(sql`
          INSERT INTO daily_breakdown (
            submission_id, submitted_device_id, date, tokens, cost,
            input_tokens, output_tokens, timestamp_ms, active_time_ms, source_breakdown,
            cost_is_complete
          )
          VALUES ${insertValuesList}
          ON CONFLICT (submission_id, submitted_device_id, date) DO UPDATE SET
            tokens = EXCLUDED.tokens,
            -- Both of these are plain overwrites on purpose. The #1044 floor is
            -- applied per client during the in-memory merge, so cost here is
            -- already recomputed from the floored breakdown and cannot be lower
            -- than what an incomplete submission is allowed to store. Guarding
            -- the scalar in SQL while replacing source_breakdown wholesale
            -- would instead leave the row's two representations disagreeing.
            cost = EXCLUDED.cost,
            cost_is_complete = EXCLUDED.cost_is_complete,
            input_tokens = EXCLUDED.input_tokens,
            output_tokens = EXCLUDED.output_tokens,
            timestamp_ms = EXCLUDED.timestamp_ms,
            -- Mirrors mergeActiveTimeMs(): this arm must not be a hole in the
            -- monotonic guard the in-memory merge path applies. GREATEST
            -- ignores NULLs in Postgres, matching the helper's null handling.
            active_time_ms = GREATEST(daily_breakdown.active_time_ms, EXCLUDED.active_time_ms),
            source_breakdown = EXCLUDED.source_breakdown
        `);
      }

      // Batch UPDATE existing rows for this device via a raw SQL VALUES list,
      // chunked for the same parameter-limit reason as the INSERT above.
      // Device ownership is immutable here: another device writes its own row.
      for (let i = 0; i < toUpdate.length; i += INSERT_CHUNK_SIZE) {
        const chunk = toUpdate.slice(i, i + INSERT_CHUNK_SIZE);
        const valuesClauses = chunk.map(
          (row) =>
            sql`(${row.id}::uuid, ${row.tokens}::bigint, ${row.cost}::numeric(14,4), ${row.inputTokens}::bigint, ${row.outputTokens}::bigint, ${row.timestampMs}::bigint, ${row.activeTimeMs}::bigint, ${JSON.stringify(row.sourceBreakdown)}::jsonb, ${row.costIsComplete}::boolean)`
        );

        const valuesList = sql.join(valuesClauses, sql`, `);

        await tx.execute(sql`
          UPDATE daily_breakdown AS d SET
            tokens = batch.tokens,
            -- Plain overwrites for the same reason as the ON CONFLICT arm: the
            -- floor lives in the merge, so this value already respects it.
            cost = batch.cost,
            cost_is_complete = batch.cost_is_complete,
            input_tokens = batch.input_tokens,
            output_tokens = batch.output_tokens,
            timestamp_ms = batch.timestamp_ms,
            active_time_ms = batch.active_time_ms,
            source_breakdown = batch.source_breakdown
          FROM (VALUES ${valuesList})
            AS batch(id, tokens, cost, input_tokens, output_tokens, timestamp_ms, active_time_ms, source_breakdown, cost_is_complete)
          WHERE d.id = batch.id
        `);
      }

      if (toDelete.length > 0) {
        const deleteIds = sql.join(
          toDelete.map((id) => sql`${id}::uuid`),
          sql`, `
        );
        await tx.execute(sql`
          DELETE FROM daily_breakdown
          WHERE id IN (${deleteIds})
        `);
      }

      // Phase 4a observation write — same transaction as the daily rows above,
      // so an explicitly reported cell cannot commit without its guarded
      // daily_breakdown counterpart. Last-write-wins (no GREATEST) for that cell only: omitted
      // cells are not zero or absent, and this is not a whole-scan snapshot.
      if (reportedRows.length > 0) {
        await recordDailyBreakdownReported({
          executor: tx,
          submittedDeviceId: submittedDevice.id,
          rows: reportedRows,
        });
      }

      // ------------------------------------------
      // STEP 3d: Recalculate submission totals from ALL daily breakdown
      // ------------------------------------------
      const [aggregates] = await tx
        .select({
          // SUM(int8) returns numeric, so casting back to bigint raises "out of
          // range" once the day rows total past int8 -- aborting the submit for
          // a user whose history is otherwise fine. Clamp rather than abort;
          // see the same treatment on the per-device aggregate below. The bound
          // is int8 max (~9.2e18 tokens), orders of magnitude above any honest
          // total, so this cannot alter a real ranking -- it only replaces a
          // 500 with a saturated value.
          totalTokens: sql<number>`LEAST(COALESCE(SUM(${dailyBreakdown.tokens}), 0), 9223372036854775807)::bigint`,
          totalCost: sql<string>`COALESCE(SUM(CAST(${dailyBreakdown.cost} AS DECIMAL(14,4))), 0)::text`,
          inputTokens: sql<number>`LEAST(COALESCE(SUM(${dailyBreakdown.inputTokens}), 0), 9223372036854775807)::bigint`,
          outputTokens: sql<number>`LEAST(COALESCE(SUM(${dailyBreakdown.outputTokens}), 0), 9223372036854775807)::bigint`,
          // The filter keeps an emptied day from stretching the reported range,
          // but it returns NULL when NO row in scope has tokens -- and
          // date_start/date_end are NOT NULL, so STEP 3e would abort the whole
          // submit. A user whose entire stored history is legacy tokenless
          // Cursor rows is exactly that shape and is explicitly valid. Fall
          // back to the unfiltered bounds the earlier producers of these
          // columns used (migrations 0015/0016).
          dateStart: sql<string>`COALESCE(MIN(CASE WHEN ${dailyBreakdown.tokens} > 0 THEN ${dailyBreakdown.date} END), MIN(${dailyBreakdown.date}))`,
          dateEnd: sql<string>`COALESCE(MAX(CASE WHEN ${dailyBreakdown.tokens} > 0 THEN ${dailyBreakdown.date} END), MAX(${dailyBreakdown.date}))`,
          activeDays: sql<number>`COUNT(DISTINCT CASE WHEN ${dailyBreakdown.tokens} > 0 THEN ${dailyBreakdown.date} END)::int`,
          rowCount: sql<number>`COUNT(*)::int`,
          // One floored day on ONE device makes the summed total a lower bound,
          // so completeness composes with AND, not OR (#1044). COALESCE covers
          // the no-rows case, where an empty total is trivially complete.
          costIsComplete: sql<boolean>`COALESCE(BOOL_AND(${dailyBreakdown.costIsComplete}), true)`,
        })
        .from(dailyBreakdown)
        .where(eq(dailyBreakdown.submissionId, submissionId));

      // Session-shape totals come from the PER-DEVICE high-water marks, not
      // from SUM(daily_breakdown.active_time_ms).
      //
      // Two reasons. (1) Additivity: sessionCount is a count of independent
      // local sessions, so two devices reporting 100 and 40 total 140 -- taking
      // a max across devices would report 100 and silently drop the second
      // machine. (2) Timezone stability: the daily rows apportion each interval
      // across LOCAL calendar days, so rescanning the same history under a
      // different TZ re-splits it; combined with the monotonic per-day merge
      // that permanently inflates SUM(daily). The CLI's timeMetrics totals are
      // plain sums of interval durations and carry no date bucketing, so they
      // survive a TZ change unchanged.
      const [deviceTotals] = await tx
        .select({
          // LEAST() clamps rather than aborting the submit. SUM() widens its
          // input (int4 -> bigint, int8 -> numeric), so the casts back down to
          // the column type raise "out of range" as soon as the per-device
          // values total past it. TimeMetricsSchema bounds these at min(0)
          // with no maximum, so a client reporting ~2.1B sessions on one
          // device would otherwise make every submit from a SECOND device
          // fail: the aggregate reads both rows, overflows ::int, and rolls
          // the transaction back. Clamping keeps a dishonest payload from
          // locking a real device out. MAX() needs no clamp -- it returns a
          // value that already fit the column.
          totalActiveTimeMs: sql<number>`LEAST(COALESCE(SUM(${submittedDevices.totalActiveTimeMs}), 0), 9223372036854775807)::bigint`,
          sessionCount: sql<number>`LEAST(COALESCE(SUM(${submittedDevices.sessionCount}), 0), 2147483647)::int`,
          longestContinuousMs: sql<number>`COALESCE(MAX(${submittedDevices.longestContinuousMs}), 0)::bigint`,
          maxConcurrentSessions: sql<number>`COALESCE(MAX(${submittedDevices.maxConcurrentSessions}), 0)::int`,
        })
        .from(submittedDevices)
        .where(eq(submittedDevices.userId, tokenRecord.userId));

      const allDays = await tx
        .select({
          sourceBreakdown: dailyBreakdown.sourceBreakdown,
        })
        .from(dailyBreakdown)
        .where(eq(dailyBreakdown.submissionId, submissionId));

      const allClients = new Set<string>();
      const allModels = new Set<string>();
      let totalCacheRead = 0;
      let totalCacheCreation = 0;
      let totalReasoning = 0;

      for (const day of allDays) {
        if (day.sourceBreakdown) {
          for (const [rawClientName, clientData] of Object.entries(day.sourceBreakdown)) {
            const clientName = rawClientName === "kilocode" ? "kilo" : rawClientName;
            allClients.add(clientName);
            const cd = clientData as ClientBreakdownData;
            if (cd.models) {
              for (const modelId of Object.keys(cd.models)) {
                allModels.add(modelId);
              }
            } else if (cd.modelId) {
              allModels.add(cd.modelId);
            }
            totalCacheRead += cd.cacheRead || 0;
            totalCacheCreation += cd.cacheWrite || 0;
            totalReasoning += cd.reasoning || 0;
          }
        }
      }

      // ------------------------------------------
      // STEP 3e: Update submission record
      // ------------------------------------------
      await tx
        .update(submissions)
        .set({
          totalTokens: aggregates.totalTokens,
          totalCost: aggregates.totalCost,
          inputTokens: aggregates.inputTokens,
          outputTokens: aggregates.outputTokens,
          cacheReadTokens: totalCacheRead,
          cacheCreationTokens: totalCacheCreation,
          reasoningTokens: totalReasoning,
          dateStart: aggregates.dateStart,
          dateEnd: aggregates.dateEnd,
           sourcesUsed: Array.from(allClients),
           modelsUsed: Array.from(allModels),
          cliVersion: data.meta.version,
          submissionHash: generateSubmissionHash(hashData),
          // Sticky: only ever set to true. A later live CLI submit omits the
          // key entirely, so it can never reset an account's backfill flag —
          // the merged totals still include the imported history.
          ...(isBackfill ? { hasBackfill: true } : {}),
          // Deliberately NOT sticky, unlike hasBackfill: recomputed from the
          // day rows every submit, so a user whose pricing recovers earns an
          // exact total back once every contributing row is complete.
          costIsComplete: aggregates.costIsComplete,
          submitCount: sql`COALESCE(submit_count, 0) + 1`,
          schemaVersion: sql`GREATEST(COALESCE(${submissions.schemaVersion}, 0), ${submitDevice.schemaVersion})`,
          // Derived from the per-device high-water marks (see deviceTotals
          // above), floored by whatever is already stored.
          //
          // The floor exists for the migration transition: every device row
          // starts with NULL metrics, so until a device submits again its
          // contribution to the SUM is 0. Without the floor a user's first
          // post-migration submit from one of two machines would drop the
          // account total -- exactly the regression d9df8c9c fixed. It also
          // preserves monotonicity generally.
          //
          // Consequence worth knowing: a total that was already inflated by a
          // pre-migration TZ re-split stays frozen at the inflated value; the
          // floor can only hold it, never correct it down. New inflation is
          // prevented, historical inflation is not repaired.
          totalActiveTimeMs: Math.max(
            deviceTotals?.totalActiveTimeMs ?? 0,
            storedSessionMetrics?.totalActiveTimeMs ?? 0,
          ),
          longestContinuousMs: Math.max(
            deviceTotals?.longestContinuousMs ?? 0,
            storedSessionMetrics?.longestContinuousMs ?? 0,
          ),
          maxConcurrentSessions: Math.max(
            deviceTotals?.maxConcurrentSessions ?? 0,
            storedSessionMetrics?.maxConcurrentSessions ?? 0,
          ),
          sessionCount: Math.max(
            deviceTotals?.sessionCount ?? 0,
            storedSessionMetrics?.sessionCount ?? 0,
          ),
          mcpServers: mcpServers && mcpServers.length > 0 ? mcpServers : null,
          updatedAt: new Date(),
        })
        .where(eq(submissions.id, submissionId));

      // Register durable post-commit work before the transaction exposes its
      // daily rows. If this invocation is interrupted after commit, a later
      // enabled submit replays this row rather than leaving a stranded counter.
      if (ratchetCensusEnabled && ratchetCensusBuckets.length > 0) {
        await tx.execute(sql`
          INSERT INTO ratchet_census_work (submission_id, submitted_device_id, buckets)
          VALUES (
            ${submissionId}::uuid,
            ${submittedDevice.id}::uuid,
            ${JSON.stringify(ratchetCensusBuckets)}::jsonb
          )
        `);
      }

      return {
        submissionId,
        isNewSubmission,
        metrics: {
          totalTokens: aggregates.totalTokens,
          totalCost: parseFloat(aggregates.totalCost),
          dateRange: {
            start: aggregates.dateStart,
            end: aggregates.dateEnd,
          },
          activeDays: aggregates.activeDays,
          clients: Array.from(allClients),
        },
      };
    });

    // Phase 1 / 1.5 census. Deliberately after the transaction commits, and
    // deliberately unable to affect the response below.
    await recordRatchetCensus({
      userId: tokenRecord.userId,
      submissionId: result.submissionId,
      servedTokens: result.metrics.totalTokens,
      servedCost: result.metrics.totalCost,
      enabled: ratchetCensusEnabled,
    });

    const usernameCacheKey = normalizeUsernameCacheKey(tokenRecord.username);
    try {
      revalidateTag("leaderboard", "max");
      revalidateTag(`user:${usernameCacheKey}`, "max");
      revalidateTag("user-rank", "max");
      revalidateTag(`user-rank:${usernameCacheKey}`, "max");
    } catch (e) {
      console.error("Public cache invalidation failed:", e);
    }

    // Re-warm the default leaderboard view in the background so the first
    // visitor after the invalidation gets a cache hit instead of paying the
    // multi-second cold aggregation query.
    try {
      after(() =>
        getLeaderboardData().catch((e) => {
          console.error("Leaderboard cache warmup failed:", e);
        }),
      );
    } catch {
      // `after` throws outside a request scope (e.g. direct handler calls in
      // tests) — the warmup is best-effort, so skip it there.
    }

    try {
      await revalidateUserGroupLeaderboards(tokenRecord.userId);
    } catch (e) {
      console.error("Group leaderboard cache invalidation failed:", e);
    }

    try {
      revalidateUsernamePaths(tokenRecord.username);
    } catch (e) {
      console.error("Username path revalidation failed:", e);
    }

    return NextResponse.json({
      success: true,
      submissionId: result.submissionId,
      username: tokenRecord.username,
      metrics: result.metrics,
      mode: result.isNewSubmission ? "create" : "merge",
      warnings: warnings.length > 0 ? warnings : undefined,
    });
  } catch (error) {
    console.error("Submit error:", error);
    return NextResponse.json(
      { error: "Internal server error" },
      { status: 500 }
    );
  }
}
