/**
 * Submission Validation (Level 1)
 * - Mathematical consistency (no negatives, totals match)
 * - Server-side sanity caps for self-reported usage
 * - No future dates
 * - Required fields present
 */

import { createHash } from "node:crypto";
import { z } from "zod";
import { SUPPORTED_CLIENT_TYPES } from "../types";

// ============================================================================
// SCHEMAS
// ============================================================================

const COST_RELATIVE_TOLERANCE = 0.01;
const COST_ABSOLUTE_TOLERANCE = 0.1;
// Sub-cent epsilon for "is there real cost left after subtracting the
// Cursor-legacy carve-out?" comparisons. Strict `> 0` would falsely reject
// an all-legacy submission when floating-point summation leaves a tiny
// residue (e.g. 0.1 + 0.2 === 0.30000000000000004, IEEE 0.3 ≈
// 0.299999999999999988, diff ≈ 5.5e-17). 1e-6 is well below any real LLM
// charge, so legitimate non-legacy cost will still trip the check.
const LEGACY_COST_FLOAT_EPSILON = 1e-6;
const TOKEN_RELATIVE_TOLERANCE = 0.01;
const TOKEN_ABSOLUTE_TOLERANCE = 100;

const NonNegativeIntegerSchema = z.number().finite().int().min(0).max(Number.MAX_SAFE_INTEGER);
const NonNegativeNumberSchema = z.number().finite().min(0);

const TokenBreakdownSchema = z.object({
  input: NonNegativeIntegerSchema,
  output: NonNegativeIntegerSchema,
  cacheRead: NonNegativeIntegerSchema,
  cacheWrite: NonNegativeIntegerSchema,
  reasoning: NonNegativeIntegerSchema,
});

const ClientContributionProvenanceSchema = z.object({
  schemaVersion: NonNegativeIntegerSchema.min(1),
  messageCount: NonNegativeIntegerSchema,
  modelCount: NonNegativeIntegerSchema,
});

const CcMirrorSourceSchema = z.string().regex(
  /^cc-mirror\/[a-z0-9][a-z0-9._-]{0,95}$/,
  "Invalid cc-mirror variant client id"
);
const SourceSchema = z.union([z.enum(SUPPORTED_CLIENT_TYPES), CcMirrorSourceSchema]);

const ClientContributionSchema = z.object({
  client: SourceSchema,
  modelId: z.string().min(1),
  providerId: z.string().optional(),
  tokens: TokenBreakdownSchema,
  cost: NonNegativeNumberSchema,
  messages: NonNegativeIntegerSchema,
  provenance: ClientContributionProvenanceSchema.optional(),
});

const DailyContributionSchema = z.object({
  date: z.string().regex(/^\d{4}-\d{2}-\d{2}$/),
  timestampMs: z.number().int().min(1e12).max(Number.MAX_SAFE_INTEGER).optional(),
  activeTimeMs: z.number().int().min(0).optional(),
  totals: z.object({
    tokens: NonNegativeIntegerSchema,
    cost: NonNegativeNumberSchema,
    messages: NonNegativeIntegerSchema,
  }),
  intensity: NonNegativeIntegerSchema.max(4),
  tokenBreakdown: TokenBreakdownSchema,
  clients: z.array(ClientContributionSchema),
});

const YearSummarySchema = z.object({
  year: z.string().regex(/^\d{4}$/),
  totalTokens: NonNegativeIntegerSchema,
  totalCost: NonNegativeNumberSchema,
  range: z.object({
    start: z.string().regex(/^\d{4}-\d{2}-\d{2}$/),
    end: z.string().regex(/^\d{4}-\d{2}-\d{2}$/),
  }),
});

const DataSummarySchema = z.object({
  totalTokens: NonNegativeIntegerSchema,
  totalCost: NonNegativeNumberSchema,
  totalDays: NonNegativeIntegerSchema,
  activeDays: NonNegativeIntegerSchema,
  averagePerDay: NonNegativeNumberSchema,
  maxCostInSingleDay: NonNegativeNumberSchema,
  clients: z.array(SourceSchema),
  models: z.array(z.string()),
});

const ExportMetaSchema = z.object({
  generatedAt: z.string(),
  version: z.string(),
  dateRange: z.object({
    start: z.string().regex(/^\d{4}-\d{2}-\d{2}$/),
    end: z.string().regex(/^\d{4}-\d{2}-\d{2}$/),
  }),
});

const SubmitDeviceSchema = z.object({
  id: z.string().trim().min(1).max(96).regex(/^[A-Za-z0-9._:-]+$/),
  name: z.string().trim().min(1).max(120).optional(),
});

const LEGACY_CLIENT_ALIASES: Record<string, string> = {
  kilocode: "kilo",
};

function normalizeLegacyClientId(id: unknown): unknown {
  if (typeof id === "string" && id in LEGACY_CLIENT_ALIASES) {
    return LEGACY_CLIENT_ALIASES[id];
  }
  return id;
}

/**
 * Normalize legacy payloads:
 * - "sources"/"source" → "clients"/"client" key renames
 * - "kilocode" → "kilo" client ID alias
 * This ensures older CLI versions can still submit data.
 */
function normalizeLegacySources(data: unknown): unknown {
  if (!data || typeof data !== "object") return data;
  const d = { ...(data as Record<string, unknown>) };

  if (d.summary && typeof d.summary === "object") {
    const summary = { ...(d.summary as Record<string, unknown>) };
    if ("sources" in summary && !("clients" in summary)) {
      summary.clients = summary.sources;
      delete summary.sources;
    }
    if (Array.isArray(summary.clients)) {
      summary.clients = summary.clients.map(normalizeLegacyClientId);
    }
    d.summary = summary;
  }

  if (Array.isArray(d.contributions)) {
    d.contributions = (d.contributions as Record<string, unknown>[]).map((c) => {
      if (!c || typeof c !== "object") return c;
      const contrib = { ...c };
      if ("sources" in contrib && !("clients" in contrib)) {
        const items = Array.isArray(contrib.sources) ? contrib.sources : [];
        contrib.clients = (items as Record<string, unknown>[]).map((s) => {
          if (s && typeof s === "object" && "source" in s && !("client" in s)) {
            const { source, ...rest } = s;
            return { client: normalizeLegacyClientId(source), ...rest };
          }
          return s;
        });
        delete contrib.sources;
      }
      if (Array.isArray(contrib.clients)) {
        contrib.clients = (contrib.clients as Record<string, unknown>[]).map((cl) => {
          if (cl && typeof cl === "object" && "client" in cl) {
            return { ...cl, client: normalizeLegacyClientId(cl.client) };
          }
          return cl;
        });
      }
      return contrib;
    });
  }

  return d;
}

const TimeMetricsSchema = z.object({
  totalActiveTimeMs: z.number().int().min(0),
  longestContinuousMs: z.number().int().min(0),
  maxConcurrentSessions: z.number().int().min(0),
  sessionCount: z.number().int().min(0),
});

const SubmissionDataSchema = z.preprocess(normalizeLegacySources, z.object({
  meta: ExportMetaSchema,
  device: SubmitDeviceSchema.optional(),
  summary: DataSummarySchema,
  years: z.array(YearSummarySchema),
  contributions: z.array(DailyContributionSchema),
  timeMetrics: TimeMetricsSchema.optional(),
}));

export type SubmissionData = z.infer<typeof SubmissionDataSchema>;

type TokenBreakdown = SubmissionData["contributions"][number]["tokenBreakdown"];

// ============================================================================
// VALIDATION FUNCTIONS
// ============================================================================

export interface ValidationResult {
  valid: boolean;
  errors: string[];
  warnings: string[];
  data?: SubmissionData;
}

function tokenTotal(tokens: TokenBreakdown): number {
  return tokens.input + tokens.output + tokens.cacheRead + tokens.cacheWrite + tokens.reasoning;
}

function exceedsTolerance(
  actual: number,
  expected: number,
  relativeTolerance: number,
  absoluteTolerance: number
): boolean {
  return Math.abs(actual - expected) > Math.max(Math.abs(expected) * relativeTolerance, absoluteTolerance);
}

// Cursor legacy carve-out: Cursor's pre-2025-05 usage exports include rows
// whose model id is `premium-tool-call`. These rows are billed per tool
// invocation and carry no token attribution at all (input/output/cache
// columns are empty in the CSV). They legitimately have cost > 0 with every
// token field equal to 0, so the "Cost submitted without tokens" sanity
// check must skip them — otherwise any user with historical Cursor data
// (even a few cents' worth) is permanently locked out of submitting.
const CURSOR_LEGACY_TOKENLESS_MODELS: ReadonlySet<string> = new Set([
  "premium-tool-call",
]);

type ClientLike = {
  client: string;
  modelId: string;
  providerId?: string;
  tokens: TokenBreakdown;
  cost: number;
};

function isLegacyTokenlessCursorClient(client: ClientLike): boolean {
  return (
    client.client === "cursor" &&
    CURSOR_LEGACY_TOKENLESS_MODELS.has(client.modelId) &&
    tokenTotal(client.tokens) === 0
  );
}

function formatTokenBreakdown(tokens: TokenBreakdown): string {
  return (
    `input=${tokens.input}, output=${tokens.output}, ` +
    `cacheRead=${tokens.cacheRead}, cacheWrite=${tokens.cacheWrite}, ` +
    `reasoning=${tokens.reasoning}`
  );
}

function formatCost(cost: number): string {
  return `$${cost.toFixed(4)}`;
}

function describeTokenlessOffenders(clients: ClientLike[]): string {
  return clients
    .filter((c) => c.cost > 0 && tokenTotal(c.tokens) === 0)
    .map((c) => {
      const provider = c.providerId ? ` (provider=${c.providerId})` : "";
      return `${c.client}/${c.modelId}${provider} cost=${formatCost(c.cost)}`;
    })
    .join("; ");
}

/**
 * Validate submission data (Level 1 validation)
 */
export function validateSubmission(data: unknown): ValidationResult {
  const errors: string[] = [];
  const warnings: string[] = [];

  // Step 1: Schema validation
  const parseResult = SubmissionDataSchema.safeParse(data);
  if (!parseResult.success) {
    return {
      valid: false,
      errors: parseResult.error.issues.map(
        (e: z.ZodIssue) => `${e.path.join(".")}: ${e.message}`
      ),
      warnings: [],
    };
  }

  const submission = parseResult.data;

  // Step 2: No future dates
  // CLI generates dates using local timezone (chrono::Local), server validates
  // against UTC. A 2-day buffer handles:
  //   1. Max timezone offset (UTC+14 = ~14 hours ahead)
  //   2. Date boundary edge cases from session aggregation
  //   3. Clock skew between client and server
  // Security note: allows submitting "tomorrow's" data, but trust model already
  // relies on self-reported data without cryptographic proof.
  // See: https://github.com/junhoyeo/tokscale/issues/318
  // See: https://github.com/junhoyeo/tokscale/issues/334
  const now = new Date();
  const maxDate = new Date(now.getTime() + 2 * 24 * 60 * 60 * 1000);
  const maxDateStr = maxDate.toISOString().split("T")[0];

  if (submission.meta.dateRange.end > maxDateStr) {
    errors.push(`Date range extends into the future: ${submission.meta.dateRange.end}`);
  }

  for (const day of submission.contributions) {
    if (day.date > maxDateStr) {
      errors.push(`Future date found in contributions: ${day.date}`);
    }
  }

  // Step 3: Mathematical consistency and sanity checks

  // 3a. Summary totals should match sum of contributions
  const calculatedTotalTokens = submission.contributions.reduce(
    (sum, day) => sum + day.totals.tokens,
    0
  );
  const calculatedTotalCost = submission.contributions.reduce(
    (sum, day) => sum + day.totals.cost,
    0
  );

  // Allow small tolerance for floating point and legacy rounding.
  const tokenDiff = Math.abs(calculatedTotalTokens - submission.summary.totalTokens);
  const costDiff = Math.abs(calculatedTotalCost - submission.summary.totalCost);

  if (tokenDiff > submission.summary.totalTokens * 0.01 && tokenDiff > 100) {
    errors.push(
      `Token total mismatch: summary=${submission.summary.totalTokens}, calculated=${calculatedTotalTokens}`
    );
  }

  if (costDiff > submission.summary.totalCost * 0.01 && costDiff > 0.1) {
    errors.push(
      `Cost total mismatch: summary=${submission.summary.totalCost.toFixed(2)}, calculated=${calculatedTotalCost.toFixed(2)}`
    );
  }

  if (submission.summary.totalCost > 0 && submission.summary.totalTokens === 0) {
    // Cursor legacy: subtract premium-tool-call cost before the tokenless-cost
    // detection so a legacy charge cannot trip the error.
    const allClients: ClientLike[] = submission.contributions.flatMap(
      (d) => d.clients
    );
    const legacyCost = allClients
      .filter(isLegacyTokenlessCursorClient)
      .reduce((sum, c) => sum + c.cost, 0);
    const checkableCost = Math.max(0, submission.summary.totalCost - legacyCost);

    if (checkableCost > LEGACY_COST_FLOAT_EPSILON) {
      const offenders = describeTokenlessOffenders(
        allClients.filter((c) => !isLegacyTokenlessCursorClient(c))
      );
      const detail =
        `cost=${formatCost(submission.summary.totalCost)}, total tokens=0` +
        (offenders ? `; offending clients: ${offenders}` : "");
      errors.push(`Submission summary: Cost submitted without tokens (${detail})`);
    }
  }

  // 3b. Active days should match
  const activeDays = submission.contributions.filter((d) => d.totals.tokens > 0).length;
  if (activeDays !== submission.summary.activeDays) {
    warnings.push(
      `Active days mismatch: summary=${submission.summary.activeDays}, calculated=${activeDays}`
    );
  }

  // 3c. Day token breakdown should sum to totals
  for (const day of submission.contributions) {
    {
      // Cursor legacy: subtract premium-tool-call cost before both sanity
      // caps so legacy charges that share a day with normal token-bearing
      // usage cannot inflate the cost-per-million ratio either.
      const legacyCost = day.clients
        .filter(isLegacyTokenlessCursorClient)
        .reduce((sum, c) => sum + c.cost, 0);
      const checkableCost = Math.max(0, day.totals.cost - legacyCost);

      if (day.totals.cost > 0 && day.totals.tokens === 0) {
        if (checkableCost > LEGACY_COST_FLOAT_EPSILON) {
          const offenders = describeTokenlessOffenders(
            day.clients.filter((c) => !isLegacyTokenlessCursorClient(c))
          );
          const detail =
            `cost=${formatCost(day.totals.cost)}, total tokens=0` +
            (offenders ? `; offending clients: ${offenders}` : "");
          errors.push(`Day ${day.date}: Cost submitted without tokens (${detail})`);
        }
      }
    }

    const dayBreakdownTokens = tokenTotal(day.tokenBreakdown);
    if (exceedsTolerance(dayBreakdownTokens, day.totals.tokens, TOKEN_RELATIVE_TOLERANCE, TOKEN_ABSOLUTE_TOLERANCE)) {
      errors.push(
        `Day ${day.date}: token breakdown (${dayBreakdownTokens}) does not match total (${day.totals.tokens})`
      );
    }

    // Check clients sum to day totals. The route persists client rows and then
    // recalculates totals from them, so mismatches must not remain warnings.
    if (day.clients.length > 0) {
      const clientsTokenSum = day.clients.reduce((sum, c) => {
        const t = c.tokens;
        return sum + tokenTotal(t);
      }, 0);
      const clientsCostSum = day.clients.reduce((sum, c) => sum + c.cost, 0);

      if (exceedsTolerance(clientsTokenSum, day.totals.tokens, TOKEN_RELATIVE_TOLERANCE, TOKEN_ABSOLUTE_TOLERANCE)) {
        errors.push(
          `Day ${day.date}: client tokens (${clientsTokenSum}) do not match total (${day.totals.tokens})`
        );
      }

      if (exceedsTolerance(clientsCostSum, day.totals.cost, COST_RELATIVE_TOLERANCE, COST_ABSOLUTE_TOLERANCE)) {
        errors.push(
          `Day ${day.date}: client cost (${clientsCostSum.toFixed(2)}) does not match total (${day.totals.cost.toFixed(2)})`
        );
      }
    }

    for (const client of day.clients) {
      const clientTokens = tokenTotal(client.tokens);

      if (client.cost > 0 && clientTokens === 0) {
        if (!isLegacyTokenlessCursorClient(client)) {
          const provider = client.providerId
            ? ` (provider=${client.providerId})`
            : "";
          errors.push(
            `Client ${client.client}/${client.modelId}${provider} on ${day.date}: ` +
              `Cost submitted without tokens ` +
              `(cost=${formatCost(client.cost)}, tokens={${formatTokenBreakdown(client.tokens)}})`
          );
        }
      }
    }
  }

  // 3d. Dates should be in order and within date range
  const sortedDates = [...submission.contributions].sort((a, b) =>
    a.date.localeCompare(b.date)
  );

  if (sortedDates.length > 0) {
    const firstDate = sortedDates[0].date;
    const lastDate = sortedDates[sortedDates.length - 1].date;

    if (firstDate < submission.meta.dateRange.start) {
      warnings.push(
        `Contribution date ${firstDate} is before dateRange.start ${submission.meta.dateRange.start}`
      );
    }

    if (lastDate > submission.meta.dateRange.end) {
      warnings.push(
        `Contribution date ${lastDate} is after dateRange.end ${submission.meta.dateRange.end}`
      );
    }
  }

  // 3e. No duplicate dates
  const dateSet = new Set<string>();
  for (const day of submission.contributions) {
    if (dateSet.has(day.date)) {
      errors.push(`Duplicate date found: ${day.date}`);
    }
    dateSet.add(day.date);
  }

  // 3f. Year summaries should be reasonable
  for (const year of submission.years) {
    const yearDays = submission.contributions.filter((d) =>
      d.date.startsWith(year.year)
    );
    const yearTokens = yearDays.reduce((sum, d) => sum + d.totals.tokens, 0);

    if (Math.abs(yearTokens - year.totalTokens) > year.totalTokens * 0.01 && yearTokens > 1000) {
      warnings.push(
        `Year ${year.year} token mismatch: summary=${year.totalTokens}, calculated=${yearTokens}`
      );
    }
  }

  return {
    valid: errors.length === 0,
    errors,
    warnings,
    data: errors.length === 0 ? submission : undefined,
  };
}

/**
 * Generate a content hash for the submitted usage payload.
 *
 * generatedAt is intentionally excluded so an idempotent resubmit of the same
 * usage data keeps the same hash even when the export timestamp changes.
 */
export function generateSubmissionHash(data: SubmissionData): string {
  const content = JSON.stringify({
    meta: {
      dateRange: data.meta.dateRange,
    },
    // Which durable device bucket is being replaced. Legacy no-device payloads
    // omit this key entirely (JSON.stringify skips undefined), preserving prior
    // hash behavior for clients that don't send device metadata.
    deviceId: data.device?.id,
    summary: {
      totalTokens: data.summary.totalTokens,
      totalCost: data.summary.totalCost,
      totalDays: data.summary.totalDays,
      activeDays: data.summary.activeDays,
      averagePerDay: data.summary.averagePerDay,
      maxCostInSingleDay: data.summary.maxCostInSingleDay,
      clients: data.summary.clients.slice().sort(),
      models: data.summary.models.slice().sort(),
    },
    years: data.years
      .map((year) => ({
        year: year.year,
        totalTokens: year.totalTokens,
        totalCost: year.totalCost,
        range: year.range,
      }))
      .sort((a, b) => a.year.localeCompare(b.year)),
    contributions: data.contributions
      .map((day) => ({
        date: day.date,
        timestampMs: day.timestampMs ?? null,
        totals: day.totals,
        intensity: day.intensity,
        tokenBreakdown: day.tokenBreakdown,
        clients: day.clients
          .map((client) => ({
            client: client.client,
            modelId: client.modelId,
            providerId: client.providerId ?? null,
            tokens: client.tokens,
            cost: client.cost,
            messages: client.messages,
            // provenance intentionally excluded: it is derived metadata that can
            // be back-filled without changing usage data, so it must not affect
            // idempotency. Hash ordering uses client/providerId/modelId only.
          }))
          .sort((a, b) =>
            `${a.client}\u0000${a.providerId ?? ""}\u0000${a.modelId}`.localeCompare(
              `${b.client}\u0000${b.providerId ?? ""}\u0000${b.modelId}`
            )
          ),
      }))
      .sort((a, b) => a.date.localeCompare(b.date)),
  });

  return createHash("sha256").update(content).digest("hex");
}
