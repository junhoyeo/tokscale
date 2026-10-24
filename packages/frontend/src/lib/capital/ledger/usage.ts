import { getModelLeaderboard, type Coverage, type ModelUsageRow } from "@/lib/explore";

/**
 * Tokscale usage for AI-lab profiles. These numbers describe Tokscale users only (people who
 * submit their usage with the CLI), never the market. Pages label them "among Tokscale users".
 */
export interface UsageSnapshot {
  rows: ModelUsageRow[];
  coverage: Coverage;
  /** Tokens across every identified model in the period. */
  totalTokens: number;
}

export interface LabUsage {
  models: ModelUsageRow[];
  tokens: number;
  activeModels: number;
  /** Share of identified-model tokens (0..1); null when there are no tokens at all. */
  share: number | null;
  coverage: Coverage;
}

/** The 30-day model leaderboard, or null when the data source is unavailable (pages then say so). */
export async function loadUsageSnapshot(): Promise<UsageSnapshot | null> {
  try {
    const ranked = await getModelLeaderboard({ period: "30d", sort: "users", limit: 1000 });
    return { rows: ranked.rows, coverage: ranked.coverage, totalTokens: ranked.rows.reduce((sum, r) => sum + r.tokens, 0) };
  } catch {
    return null;
  }
}

/** Models of the given Tokscale labs, by active users. null when the org has no lab slugs. */
export function usageForLabs(snapshot: UsageSnapshot, labSlugs: readonly string[]): LabUsage | null {
  if (labSlugs.length === 0) return null;
  const models = snapshot.rows
    .filter((r) => r.labSlug !== null && labSlugs.includes(r.labSlug))
    .sort((a, b) => b.activeUsers - a.activeUsers || b.tokens - a.tokens || a.key.localeCompare(b.key));
  const tokens = models.reduce((sum, r) => sum + r.tokens, 0);
  return {
    models,
    tokens,
    activeModels: models.filter((m) => m.activeUsers > 0).length,
    share: snapshot.totalTokens > 0 ? tokens / snapshot.totalTokens : null,
    coverage: snapshot.coverage,
  };
}
