export interface ProfileUser {
  username: string;
  displayName: string | null;
  avatarUrl: string | null;
  rank: number | null;
  createdAt?: string | null;
}

export interface ProfileStatsData {
  totalTokens: number;
  totalCost: number;
  inputTokens: number;
  outputTokens: number;
  cacheReadTokens: number;
  cacheWriteTokens: number;
  reasoningTokens?: number;
  activeDays: number;
  submissionCount?: number;
  totalActiveTimeMs?: number;
  sessionCount?: number;
}

export interface ModelUsage {
  model: string;
  tokens: number;
  cost: number;
  percentage: number;
}
