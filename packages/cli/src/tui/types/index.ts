import type { ColorPaletteName } from "../config/themes.js";

export type TabType = "overview" | "model" | "daily" | "stats";
export type SortType = "cost" | "tokens" | "date";
export type SourceType = "opencode" | "claude" | "codex" | "cursor" | "gemini" | "amp" | "droid" | "openclaw" | "pi";

export type { ColorPaletteName };

export interface ModelEntry {
  source: string;
  model: string;
  input: number;
  output: number;
  cacheWrite: number;
  cacheRead: number;
  reasoning: number;
  total: number;
  cost: number;
}

export interface DailyEntry {
  date: string;
  input: number;
  output: number;
  cacheRead: number;
  cacheWrite: number;
  total: number;
  cost: number;
}

export interface ContributionDay {
  date: string;
  cost: number;
  tokens: number;
  level: number;
}

export interface GridCell {
  date: string | null;
  level: number;
}

export interface TotalBreakdown {
  input: number;
  output: number;
  cacheWrite: number;
  cacheRead: number;
  reasoning: number;
  total: number;
  cost: number;
}

export interface Stats {
  favoriteModel: string;
  totalTokens: number;
  sessions: number;
  longestSession: string;
  currentStreak: number;
  longestStreak: number;
  activeDays: number;
  totalDays: number;
  peakHour: string;
}

export interface ModelWithPercentage {
  modelId: string;
  percentage: number;
  inputTokens: number;
  outputTokens: number;
  cacheReadTokens: number;
  cacheWriteTokens: number;
  totalTokens: number;
  cost: number;
}

export interface ChartModelData {
  modelId: string;
  tokens: number;
  color: string;
}

export interface ChartDataPoint {
  date: string;
  models: ChartModelData[];
  total: number;
}

export interface TUIData {
  modelEntries: ModelEntry[];
  dailyEntries: DailyEntry[];
  contributions: ContributionDay[];
  contributionGrid: GridCell[][];
  stats: Stats;
  totalCost: number;
  totals: TotalBreakdown;
  modelCount: number;
  chartData: ChartDataPoint[];
  topModels: ModelWithPercentage[];
  dailyBreakdowns: Map<string, DailyModelBreakdown>;
}

export interface TUISettings {
  colorPalette: string;
}

export type LoadingPhase = 
  | "idle"
  | "parsing-sources"
  | "loading-pricing"
  | "finalizing-report"
  | "complete";

export interface DailyModelBreakdown {
  date: string;
  cost: number;
  totalTokens: number;
  models: Array<{
    modelId: string;
    source: string;
    tokens: {
      input: number;
      output: number;
      cacheRead: number;
      cacheWrite: number;
      reasoning: number;
    };
    cost: number;
    messages: number;
  }>;
}

export interface TUIOptions {
  initialTab?: TabType;
  enabledSources?: SourceType[];
  sortBy?: SortType;
  sortDesc?: boolean;
  since?: string;
  until?: string;
  year?: string;
  sinceTs?: number;
  untilTs?: number;
  colorPalette?: ColorPaletteName;
}



export const LAYOUT = {
  HEADER_HEIGHT: 1,
  FOOTER_HEIGHT: 3,
  MIN_CONTENT_HEIGHT: 12,
  CHART_HEIGHT_RATIO: 0.35,
  MIN_CHART_HEIGHT: 5,
  MIN_LIST_HEIGHT: 4,
  CHART_AXIS_WIDTH: 8,
  MIN_CHART_WIDTH: 20,
  MAX_VISIBLE_BARS: 52,
} as const;

export const SOURCE_LABELS: Record<SourceType, string> = {
  opencode: "OC",
  claude: "CC",
  codex: "CX",
  cursor: "CR",
  gemini: "GM",
  amp: "AM",
  droid: "DR",
  openclaw: "CL",
  pi: "PI",
} as const;

export const TABS: readonly TabType[] = ["overview", "model", "daily", "stats"] as const;
export const ALL_SOURCES: readonly SourceType[] = ["opencode", "claude", "codex", "cursor", "gemini", "amp", "droid", "openclaw", "pi"] as const;
