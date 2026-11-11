/**
 * Token mix per model for "price at real agent usage". Reads the same facts
 * as the explore rollups (daily_breakdown.source_breakdown, accounts with
 * users.leaderboard_hidden = true excluded) and sums the five token buckets
 * per canonical model key in the current window.
 *
 * Server only. Without DATABASE_URL there is no usage data: callers get null
 * and show the estimate as unavailable instead of inventing a mix.
 */
import { isIdentifiedModelKey, modelKey } from "@/lib/explore/modelKey";
import type { UsageFact } from "@/lib/explore/rollup";
import type { ExplorePeriod } from "@/lib/explore/types";
import type { ExploreWindow } from "@/lib/explore/window";
import type { ModelMixes } from "./dashboard";
import { addMix, emptyMix } from "./price";

export interface ModelMixRollup extends ModelMixes {
  period: ExplorePeriod;
  start: string;
  end: string;
}

/** Sum current-window facts into per-model and community mixes. Pure. */
export function foldModelMixes(window: ExploreWindow, facts: readonly UsageFact[]): ModelMixRollup {
  const community = emptyMix();
  const byModel: Record<string, ReturnType<typeof emptyMix>> = Object.create(null);
  const keys = new Map<string, string>();
  for (const fact of facts) {
    if (!fact.current || fact.tokens <= 0) continue;
    let key = keys.get(fact.model);
    if (key === undefined) {
      key = modelKey(fact.model);
      keys.set(fact.model, key);
    }
    if (!isIdentifiedModelKey(key)) continue;
    addMix(community, fact);
    addMix((byModel[key] ??= emptyMix()), fact);
  }
  return { period: window.period, start: window.start, end: window.end, community, byModel };
}

export async function computeModelMixes(period: ExplorePeriod, now: Date = new Date()): Promise<ModelMixRollup> {
  const { computeFacts, computeWindow } = await import("@/lib/explore/db");
  const window = await computeWindow(period, now);
  return foldModelMixes(window, await computeFacts(window));
}

export async function getModelMixes(period: ExplorePeriod): Promise<ModelMixRollup | null> {
  if (!process.env.DATABASE_URL) return null;
  const { getPeriodModelMixes } = await import("@/lib/explore/db");
  return getPeriodModelMixes(period);
}

/** Shared 30-day counts for benchmark pages; no all-time leaderboard load. */
export async function getBenchmarkModelUsage() {
  if (!process.env.DATABASE_URL) return null;
  const { getModelUsageCounts } = await import("@/lib/explore/db");
  return getModelUsageCounts();
}
