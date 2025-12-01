/**
 * Frontend type definitions for contribution graph
 * Based on CLI's graph-types.ts with frontend-specific additions
 * Note: intensity is calculated based on COST ($), not tokens
 */

/**
 * Valid source identifiers
 */
export type SourceType = "opencode" | "claude" | "codex" | "gemini";

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

// =====================================
// Frontend-specific types
// =====================================

/**
 * Available color palette names for graph contribution cells
 */
export type ColorPaletteName =
  | "green"
  | "halloween"
  | "teal"
  | "blue"
  | "pink"
  | "purple"
  | "orange"
  | "monochrome"
  | "YlGnBu";

/**
 * Graph color palette - only for contribution cell colors
 */
export interface GraphColorPalette {
  name: string;
  grade0: string;
  grade1: string;
  grade2: string;
  grade3: string;
  grade4: string;
}

// Legacy type aliases for backward compatibility
export type ThemeName = ColorPaletteName;
export type Theme = GraphColorPalette;

/**
 * Graph view mode
 */
export type ViewMode = "2d" | "3d";

/**
 * Tooltip position
 */
export interface TooltipPosition {
  x: number;
  y: number;
}

/**
 * Graph container state
 */
export interface GraphState {
  view: ViewMode;
  colorPalette: ColorPaletteName;
  selectedYear: string | null;
  hoveredDay: DailyContribution | null;
  selectedDay: DailyContribution | null;
  tooltipPosition: TooltipPosition | null;
  sourceFilter: SourceType[];
  modelFilter: string[];
}

/**
 * Week data for grid rendering (53 weeks x 7 days)
 */
export interface WeekData {
  weekIndex: number;
  days: (DailyContribution | null)[];
}

/**
 * Cell hit test result for mouse interaction
 */
export interface CellHitResult {
  row: number;
  col: number;
  day: DailyContribution | null;
}
