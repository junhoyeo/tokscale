import type { ModelUsage } from "./types";

export type ModelRankBy = "cost" | "tokens";

export const DEFAULT_MODEL_RANK_BY: ModelRankBy = "cost";

export const MODEL_RANK_BY_STORAGE_KEY = "tokscale:profile-model-rank-by";

export interface RankedModelUsage extends ModelUsage {
  /** Share (0-100) for the active ranking metric, or null when the metric total is zero. */
  share: number | null;
}

export function isModelRankBy(value: unknown): value is ModelRankBy {
  return value === "cost" || value === "tokens";
}

function finiteNonnegative(value: number): number {
  return Number.isFinite(value) ? Math.max(0, value) : 0;
}

export function rankModelUsage(
  modelUsage: readonly ModelUsage[] | undefined,
  rankBy: ModelRankBy,
): RankedModelUsage[] {
  const filtered = (modelUsage ?? [])
    .filter((item) => item.model !== "<synthetic>")
    .map((item) => ({
      ...item,
      tokens: finiteNonnegative(item.tokens),
      cost: finiteNonnegative(item.cost),
      percentage: finiteNonnegative(item.percentage),
    }));

  const totalCost = filtered.reduce((sum, item) => sum + item.cost, 0);
  const totalTokens = filtered.reduce((sum, item) => sum + item.tokens, 0);

  return filtered
    .sort((left, right) => {
      if (rankBy === "tokens") {
        return (
          right.tokens - left.tokens ||
          right.cost - left.cost ||
          left.model.localeCompare(right.model)
        );
      }
      return (
        right.cost - left.cost ||
        right.tokens - left.tokens ||
        left.model.localeCompare(right.model)
      );
    })
    .map((item) => ({
      ...item,
      share:
        rankBy === "tokens"
          ? totalTokens > 0
            ? (item.tokens / totalTokens) * 100
            : null
          : totalCost > 0
            ? (item.cost / totalCost) * 100
            : null,
    }));
}

export function getFavoriteModel(
  modelUsage: readonly ModelUsage[] | undefined,
  rankBy: ModelRankBy,
): RankedModelUsage | null {
  return rankModelUsage(modelUsage, rankBy)[0] ?? null;
}
