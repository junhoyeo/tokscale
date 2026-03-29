/**
 * Submission Validation (Level 1)
 * - Mathematical consistency (no negatives, totals match)
 * - No future dates
 * - Required fields present
 */

import { z } from "zod";

// ============================================================================
// SCHEMAS
// ============================================================================

const TokenBreakdownSchema = z.object({
  input: z.number().int().min(0),
  output: z.number().int().min(0),
  cacheRead: z.number().int().min(0),
  cacheWrite: z.number().int().min(0),
  reasoning: z.number().int().min(0),
});

const SUPPORTED_SOURCES = [
  "opencode",
  "claude",
  "codex",
  "gemini",
  "cursor",
  "amp",
  "droid",
  "openclaw",
  "pi",
  "kimi",
  "qwen",
  "roocode",
  "kilo",
  "mux",
  "synthetic",
] as const;
const SourceSchema = z.enum(SUPPORTED_SOURCES);

const ClientContributionSchema = z.object({
  client: SourceSchema,
  modelId: z.string().min(1),
  providerId: z.string().optional(),
  tokens: TokenBreakdownSchema,
  cost: z.number().min(0),
  messages: z.number().int().min(0),
});

const DailyContributionSchema = z.object({
  date: z.string().regex(/^\d{4}-\d{2}-\d{2}$/),
  timestampMs: z.number().int().min(1e12).max(Number.MAX_SAFE_INTEGER).optional(),
  totals: z.object({
    tokens: z.number().int().min(0),
    cost: z.number().min(0),
    messages: z.number().int().min(0),
  }),
  intensity: z.number().int().min(0).max(4),
  tokenBreakdown: TokenBreakdownSchema,
  clients: z.array(ClientContributionSchema),
});

const YearSummarySchema = z.object({
  year: z.string().regex(/^\d{4}$/),
  totalTokens: z.number().int().min(0),
  totalCost: z.number().min(0),
  range: z.object({
    start: z.string().regex(/^\d{4}-\d{2}-\d{2}$/),
    end: z.string().regex(/^\d{4}-\d{2}-\d{2}$/),
  }),
});

const DataSummarySchema = z.object({
  totalTokens: z.number().int().min(0),
  totalCost: z.number().min(0),
  totalDays: z.number().int().min(0),
  activeDays: z.number().int().min(0),
  averagePerDay: z.number().min(0),
  maxCostInSingleDay: z.number().min(0),
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

const SubmissionDataSchema = z.preprocess(normalizeLegacySources, z.object({
  meta: ExportMetaSchema,
  summary: DataSummarySchema,
  years: z.array(YearSummarySchema),
  contributions: z.array(DailyContributionSchema),
}));

export type SubmissionData = z.infer<typeof SubmissionDataSchema>;

// ============================================================================
// VALIDATION FUNCTIONS
// ============================================================================

export interface ValidationResult {
  valid: boolean;
  errors: string[];
  warnings: string[];
  data?: SubmissionData;
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

  // Step 3: Mathematical consistency checks

  // 3a. Summary totals should match sum of contributions
  const calculatedTotalTokens = submission.contributions.reduce(
    (sum, day) => sum + day.totals.tokens,
    0
  );
  const calculatedTotalCost = submission.contributions.reduce(
    (sum, day) => sum + day.totals.cost,
    0
  );

  // Allow 1% tolerance for floating point
  const tokenDiff = Math.abs(calculatedTotalTokens - submission.summary.totalTokens);
  const costDiff = Math.abs(calculatedTotalCost - submission.summary.totalCost);

  if (tokenDiff > submission.summary.totalTokens * 0.01 && tokenDiff > 100) {
    errors.push(
      `Token total mismatch: summary=${submission.summary.totalTokens}, calculated=${calculatedTotalTokens}`
    );
  }

  if (costDiff > submission.summary.totalCost * 0.01 && costDiff > 0.1) {
    warnings.push(
      `Cost total minor mismatch: summary=${submission.summary.totalCost.toFixed(2)}, calculated=${calculatedTotalCost.toFixed(2)}`
    );
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
    // Check clients sum to day totals
    if (day.clients.length > 0) {
      const clientsTokenSum = day.clients.reduce((sum, c) => {
        const t = c.tokens;
        return sum + t.input + t.output + t.cacheRead + t.cacheWrite + t.reasoning;
      }, 0);

      // Allow some tolerance
      if (Math.abs(clientsTokenSum - day.totals.tokens) > day.totals.tokens * 0.05 && day.totals.tokens > 100) {
        warnings.push(
          `Day ${day.date}: client tokens (${clientsTokenSum}) don't match total (${day.totals.tokens})`
        );
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
 * Generate a hash for the submission data (for deduplication)
 * 
 * CHANGED for client-level merge:
 * - Hash is now based on clients + date range (not totals)
 * - Totals change after merge, so they can't be in the hash
 * - This hash identifies "what clients and dates are being submitted"
 */
export function generateSubmissionHash(data: SubmissionData): string {
  // Sort contributions by date to ensure deterministic hash
  const sortedDates = data.contributions
    .map(c => c.date)
    .sort();

  const content = JSON.stringify({
    // What clients are being submitted
    clients: data.summary.clients.slice().sort(),
    // Date range of this submission
    dateRange: data.meta.dateRange,
    // Number of days with data (for basic fingerprinting)
    daysCount: data.contributions.length,
    // First and last dates FROM SORTED LIST
    firstDay: sortedDates[0],
    lastDay: sortedDates[sortedDates.length - 1],
  });

  // Simple synchronous hash (djb2 algorithm)
  let hash = 5381;
  for (let i = 0; i < content.length; i++) {
    const char = content.charCodeAt(i);
    hash = ((hash << 5) + hash) + char; // hash * 33 + char
    hash = hash & hash; // Convert to 32-bit integer
  }

  return Math.abs(hash).toString(16).padStart(16, "0");
}
