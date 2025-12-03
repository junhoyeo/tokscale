/**
 * Type definitions for contribution graph data
 * Note: intensity is calculated based on COST ($), not tokens
 */

/**
 * Valid source identifiers
 */
export type SourceType = "opencode" | "claude" | "codex" | "gemini" | "cursor";

/**
 * Token breakdown by category
 */
export interface TokenBreakdown {
  input: number;
  output: number;
  cacheRead: number;
  cacheWrite: number;
  reasoning: number;
}

/**
 * Per-source contribution for a single day
 */
export interface SourceContribution {
  /** Source identifier */
  source: SourceType;

  /** Exact model ID as reported by the source */
  modelId: string;

  /** Provider ID if available */
  providerId?: string;

  /** Token counts */
  tokens: TokenBreakdown;

  /** Calculated cost for this source/model combination */
  cost: number;

  /** Number of messages/requests */
  messages: number;
}

/**
 * Daily contribution entry with full granularity
 */
export interface DailyContribution {
  /** ISO date string (YYYY-MM-DD) */
  date: string;

  /** Aggregated totals for the day */
  totals: {
    /** Total tokens (input + output + cache) */
    tokens: number;
    /** Total cost in USD */
    cost: number;
    /** Number of messages/requests */
    messages: number;
  };

  /**
   * Calculated intensity grade (0-4)
   * Based on COST, not tokens
   * 0 = no activity, 4 = highest cost relative to max
   */
  intensity: 0 | 1 | 2 | 3 | 4;

  /** Token breakdown by category (aggregated across all sources) */
  tokenBreakdown: TokenBreakdown;

  /** Per-source breakdown with model information */
  sources: SourceContribution[];
}

/**
 * Year-level summary
 */
export interface YearSummary {
  /** Year as string (e.g., "2024") */
  year: string;

  /** Total tokens for the year */
  totalTokens: number;

  /** Total cost for the year */
  totalCost: number;

  /** Date range for this year's data */
  range: {
    start: string;
    end: string;
  };
}

/**
 * Summary statistics
 */
export interface DataSummary {
  /** Total tokens across all time */
  totalTokens: number;

  /** Total cost across all time */
  totalCost: number;

  /** Total number of days in date range */
  totalDays: number;

  /** Number of days with activity */
  activeDays: number;

  /** Average cost per day (based on active days) */
  averagePerDay: number;

  /** Maximum cost in a single day (used for intensity calculation) */
  maxCostInSingleDay: number;

  /** All sources present in the data */
  sources: SourceType[];

  /** All unique model IDs across all sources */
  models: string[];
}

/**
 * Metadata about the export
 */
export interface ExportMeta {
  /** ISO timestamp of when the data was generated */
  generatedAt: string;

  /** CLI version that generated this data */
  version: string;

  /** Date range of the data */
  dateRange: {
    start: string;
    end: string;
  };
}

/**
 * Root data structure exported by CLI
 * This is the complete JSON schema for contribution graph data
 */
export interface TokenContributionData {
  /** Metadata about the export */
  meta: ExportMeta;

  /** Summary statistics */
  summary: DataSummary;

  /** Year-by-year breakdown for multi-year views */
  years: YearSummary[];

  /** Daily contribution data - the core dataset */
  contributions: DailyContribution[];
}

/**
 * Options for graph data generation
 */
export interface GraphOptions {
  /** Filter to specific sources */
  sources?: SourceType[];

  /** Start date filter (ISO format) */
  since?: string;

  /** End date filter (ISO format) */
  until?: string;

  /** Filter to specific year */
  year?: string;
}

/**
 * Unified message format for aggregation
 * Used internally to normalize data from different sources
 */
export interface UnifiedMessage {
  source: SourceType;
  modelId: string;
  providerId?: string;
  timestamp: number; // Unix milliseconds
  tokens: TokenBreakdown;
  cost: number;
}
