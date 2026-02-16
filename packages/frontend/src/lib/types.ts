export type SourceType = "opencode" | "claude" | "codex" | "gemini" | "cursor" | "amp" | "droid" | "openclaw";

export interface TokenBreakdown {
  input: number;
  output: number;
  cacheRead: number;
  cacheWrite: number;
  reasoning: number;
}

/**
 * Model-level usage data (used in database storage format)
 * When data comes from the database, sources are grouped with nested models
 */
export interface ModelBreakdownData {
  tokens: number;
  cost: number;
  input: number;
  output: number;
  cacheRead: number;
  cacheWrite: number;
  reasoning: number;
  messages: number;
}

/**
 * Per-source contribution
 * 
 * Two formats exist:
 * 1. CLI format: Each source/model combo is a separate entry (modelId set, no models field)
 * 2. Database format: Sources grouped with nested models (models field populated)
 */
export interface SourceContribution {
  source: SourceType;
  modelId: string;
  providerId?: string;
  tokens: TokenBreakdown;
  cost: number;
  messages: number;
  /** Present when data comes from database (grouped by source with nested models) */
  models?: Record<string, ModelBreakdownData>;
}

export interface DailyContribution {
  date: string;
  timestampMs?: number | null;
  totals: {
    tokens: number;
    cost: number;
    messages: number;
  };
  intensity: 0 | 1 | 2 | 3 | 4;
  tokenBreakdown: TokenBreakdown;
  sources: SourceContribution[];
}

export interface YearSummary {
  year: string;
  totalTokens: number;
  totalCost: number;
  range: {
    start: string;
    end: string;
  };
}

export interface DataSummary {
  totalTokens: number;
  totalCost: number;
  totalDays: number;
  activeDays: number;
  averagePerDay: number;
  maxCostInSingleDay: number;
  sources: SourceType[];
  models: string[];
}

export interface ExportMeta {
  generatedAt: string;
  version: string;
  dateRange: {
    start: string;
    end: string;
  };
}

export interface TokenContributionData {
  meta: ExportMeta;
  summary: DataSummary;
  years: YearSummary[];
  contributions: DailyContribution[];
}

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

export interface GraphColorPalette {
  name: string;
  grade0: string;
  grade1: string;
  grade2: string;
  grade3: string;
  grade4: string;
}

export type ViewMode = "2d" | "3d";

export interface TooltipPosition {
  x: number;
  y: number;
}

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

export interface WeekData {
  weekIndex: number;
  days: (DailyContribution | null)[];
}

export interface CellHitResult {
  row: number;
  col: number;
  day: DailyContribution | null;
}
