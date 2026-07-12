export type LeaderboardSortBy = 'tokens' | 'cost';

export const SORT_BY_COOKIE_NAME = "leaderboard-sort-by";
export const VALID_SORT_BY: LeaderboardSortBy[] = ['tokens', 'cost'];

export function isValidSortBy(value: unknown): value is LeaderboardSortBy {
  return typeof value === 'string' && VALID_SORT_BY.includes(value as LeaderboardSortBy);
}
