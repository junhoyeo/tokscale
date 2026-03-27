import type { SubmissionFreshness } from "@/lib/submissionFreshness";

export type Period = "all" | "month" | "week";
export type SortBy = "tokens" | "cost";

export interface LeaderboardUser {
  rank: number;
  userId: string;
  username: string;
  displayName: string | null;
  avatarUrl: string | null;
  totalTokens: number;
  totalCost: number;
  submissionCount: number | null;
  lastSubmission: string;
  submissionFreshness: SubmissionFreshness | null;
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
    totalSubmissions: number | null;
    uniqueUsers: number;
  };
  period: Period;
  sortBy: SortBy;
}
