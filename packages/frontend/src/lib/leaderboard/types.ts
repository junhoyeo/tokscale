export type Period = "all" | "month" | "last-month" | "week" | "custom";
export type SortBy = "tokens" | "cost" | "time";

export interface LeaderboardUser {
  rank: number;
  userId: string;
  username: string;
  displayName: string | null;
  avatarUrl: string | null;
  totalTokens: number;
  totalCost: number;
  totalActiveTimeMs: number | null;
}

export interface LeaderboardData {
  users: LeaderboardUser[];
  pagination: {
    page: number;
    limit: number;
    totalUsers: number;
    totalPages: number;
    hasNext: boolean;
    hasPrev: boolean;
  };
  stats: {
    totalTokens: number;
    totalCost: number;
    totalActiveTimeMs: number | null;
    uniqueUsers: number;
  };
  period: Period;
  sortBy: SortBy;
}
