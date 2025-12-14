import type { ColorPaletteName } from "../config/themes.js";

export type TabType = "overview" | "model" | "daily" | "stats";
export type SortType = "cost" | "tokens";
export type SourceType = "opencode" | "claude" | "codex" | "cursor" | "gemini";

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
  cache: number;
  total: number;
  cost: number;
}

export interface ContributionDay {
  date: string;
  cost: number;
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
}

export interface TUISettings {
  colorPalette: string;
}

export interface TUIOptions {
  initialTab?: TabType;
  enabledSources?: SourceType[];
  sortBy?: SortType;
  sortDesc?: boolean;
  since?: string;
  until?: string;
  year?: string;
  colorPalette?: ColorPaletteName;
}



export const LAYOUT = {
  HEADER_HEIGHT: 2,
  FOOTER_HEIGHT: 4,
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
} as const;

export const TABS: readonly TabType[] = ["overview", "model", "daily", "stats"] as const;
export const ALL_SOURCES: readonly SourceType[] = ["opencode", "claude", "codex", "cursor", "gemini"] as const;
