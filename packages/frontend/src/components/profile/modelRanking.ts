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

function compareCodePoint(left: string, right: string): number {
  return left < right ? -1 : left > right ? 1 : 0;
}

function computeOverflowSafeShares(values: readonly number[]): Array<number | null> {
  let maxValue = 0;
  for (const value of values) {
    if (Number.isFinite(value) && value > maxValue) {
      maxValue = value;
    }
  }

  if (!(maxValue > 0) || !Number.isFinite(maxValue)) {
    return values.map(() => null);
  }

  const normalized = values.map((value) =>
    Number.isFinite(value) && value > 0 ? value / maxValue : 0,
  );
  const normalizedTotal = normalized.reduce((sum, value) => sum + value, 0);

  if (!Number.isFinite(normalizedTotal) || !(normalizedTotal > 0)) {
    return values.map(() => null);
  }

  return normalized.map((scaled) => {
    const share = (scaled / normalizedTotal) * 100;
    return Number.isFinite(share) ? share : null;
  });
}

export function rankModelUsage(
  modelUsage: readonly ModelUsage[] | undefined,
  rankBy: ModelRankBy,
): RankedModelUsage[] {
  const sorted = (modelUsage ?? [])
    .filter((item) => item.model !== "<synthetic>")
    .map((item) => ({
      ...item,
      tokens: finiteNonnegative(item.tokens),
      cost: finiteNonnegative(item.cost),
      percentage: finiteNonnegative(item.percentage),
    }))
    .sort((left, right) => {
      if (rankBy === "tokens") {
        return (
          right.tokens - left.tokens ||
          right.cost - left.cost ||
          compareCodePoint(left.model, right.model)
        );
      }
      return (
        right.cost - left.cost ||
        right.tokens - left.tokens ||
        compareCodePoint(left.model, right.model)
      );
    });

  const shares = computeOverflowSafeShares(
    sorted.map((item) => (rankBy === "tokens" ? item.tokens : item.cost)),
  );

  return sorted.map((item, index) => ({
    ...item,
    share: shares[index] ?? null,
  }));
}

export function getFavoriteModel(
  modelUsage: readonly ModelUsage[] | undefined,
  rankBy: ModelRankBy,
): RankedModelUsage | null {
  return rankModelUsage(modelUsage, rankBy)[0] ?? null;
}
