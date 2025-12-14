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

const SourceContributionSchema = z.object({
  source: z.enum(["opencode", "claude", "codex", "gemini", "cursor"]),
  modelId: z.string().min(1),
  providerId: z.string().optional(),
  tokens: TokenBreakdownSchema,
  cost: z.number().min(0),
  messages: z.number().int().min(0),
});

const DailyContributionSchema = z.object({
  date: z.string().regex(/^\d{4}-\d{2}-\d{2}$/),
  totals: z.object({
    tokens: z.number().int().min(0),
    cost: z.number().min(0),
    messages: z.number().int().min(0),
  }),
  intensity: z.number().int().min(0).max(4),
  tokenBreakdown: TokenBreakdownSchema,
  sources: z.array(SourceContributionSchema),
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
  sources: z.array(z.enum(["opencode", "claude", "codex", "gemini", "cursor"])),
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

export const SubmissionDataSchema = z.object({
  meta: ExportMetaSchema,
  summary: DataSummarySchema,
  years: z.array(YearSummarySchema),
  contributions: z.array(DailyContributionSchema),
});

export type SubmissionData = z.infer<typeof SubmissionDataSchema>;

// ============================================================================
// VALIDATION FUNCTIONS
// ============================================================================

export interface ValidationResult {
  valid: boolean;
  errors: string[];
  warnings: string[];
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
      errors: parseResult.error.errors.map(
        (e) => `${e.path.join(".")}: ${e.message}`
      ),
      warnings: [],
    };
  }

  const submission = parseResult.data;

  // Step 2: No future dates
  const today = new Date();
  today.setHours(23, 59, 59, 999);
  const todayStr = today.toISOString().split("T")[0];

  if (submission.meta.dateRange.end > todayStr) {
    errors.push(`Date range extends into the future: ${submission.meta.dateRange.end}`);
  }

  for (const day of submission.contributions) {
    if (day.date > todayStr) {
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
    // Check sources sum to day totals
    if (day.sources.length > 0) {
      const sourcesTokenSum = day.sources.reduce((sum, s) => {
        const t = s.tokens;
        return sum + t.input + t.output + t.cacheRead + t.cacheWrite + t.reasoning;
      }, 0);

      // Allow some tolerance
      if (Math.abs(sourcesTokenSum - day.totals.tokens) > day.totals.tokens * 0.05 && day.totals.tokens > 100) {
        warnings.push(
          `Day ${day.date}: source tokens (${sourcesTokenSum}) don't match total (${day.totals.tokens})`
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
  };
}

/**
 * Generate a hash for the submission data (for deduplication)
 */
export function generateSubmissionHash(data: SubmissionData): string {
  const content = JSON.stringify({
    dateRange: data.meta.dateRange,
    totalTokens: data.summary.totalTokens,
    totalCost: data.summary.totalCost,
    activeDays: data.summary.activeDays,
    // Include first and last contribution dates for uniqueness
    firstDay: data.contributions[0]?.date,
    lastDay: data.contributions[data.contributions.length - 1]?.date,
  });

  // Simple hash - in production you might want crypto.subtle
  let hash = 0;
  for (let i = 0; i < content.length; i++) {
    const char = content.charCodeAt(i);
    hash = (hash << 5) - hash + char;
    hash = hash & hash;
  }

  return Math.abs(hash).toString(16).padStart(16, "0");
}

/**
 * Extract summary metrics from submission data
 */
export function extractMetrics(data: SubmissionData) {
  return {
    totalTokens: data.summary.totalTokens,
    totalCost: data.summary.totalCost,
    inputTokens: data.contributions.reduce(
      (sum, d) => sum + d.tokenBreakdown.input,
      0
    ),
    outputTokens: data.contributions.reduce(
      (sum, d) => sum + d.tokenBreakdown.output,
      0
    ),
    cacheCreationTokens: data.contributions.reduce(
      (sum, d) => sum + d.tokenBreakdown.cacheWrite,
      0
    ),
    cacheReadTokens: data.contributions.reduce(
      (sum, d) => sum + d.tokenBreakdown.cacheRead,
      0
    ),
    dateStart: data.meta.dateRange.start,
    dateEnd: data.meta.dateRange.end,
    sourcesUsed: data.summary.sources,
    modelsUsed: data.summary.models,
  };
}
