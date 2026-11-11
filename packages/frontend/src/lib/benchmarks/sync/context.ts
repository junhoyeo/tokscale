/**
 * Shared state for one sync run: the keys a result may map to, the alias
 * rules from the last snapshot, and what this run decided. Pure: the script
 * in scripts/benchmarks/sync.ts does all file and network work.
 */
import { aliasRuleId, resolveModelKey, type AliasRule, type ExternalModelRef, type Resolution } from "../mapping";

export type AliasSource = AliasRule["source"];

export interface UnmappedModel {
  source: string;
  externalModel: string;
  name: string | null;
  benchmarks: Set<string>;
  newestDate: string | null;
}

export interface SyncContext {
  /** Keys a result may map to: Tokscale usage keys plus models.dev keys. */
  knownKeys: ReadonlySet<string>;
  /** Alias rules from the previous aliases.json, by aliasRuleId. */
  aliases: ReadonlyMap<string, AliasRule>;
  /** Alias decisions made in this run, by aliasRuleId. */
  decisions: Map<string, AliasRule>;
  unmapped: Map<string, UnmappedModel>;
}

export function createSyncContext(knownKeys: ReadonlySet<string>, rules: readonly AliasRule[]): SyncContext {
  const aliases = new Map<string, AliasRule>();
  for (const rule of rules) aliases.set(aliasRuleId(rule.source, rule.externalId), rule);
  return { knownKeys, aliases, decisions: new Map(), unmapped: new Map() };
}

/**
 * Resolve an external model and record the decision: mapped ids become alias
 * rules (keeping a previous reviewed flag), unmapped ids go to the report.
 */
export function mapExternalModel(
  ctx: SyncContext,
  source: AliasSource,
  ref: ExternalModelRef,
  usage: { benchmark: string; name: string | null; date: string | null },
  prefer: "ids" | "names" = "ids",
): Resolution {
  const resolution = resolveModelKey(source, ref, ctx.knownKeys, ctx.aliases, prefer);
  const id = aliasRuleId(source, ref.externalId);
  if (resolution.key && resolution.method) {
    const previous = ctx.aliases.get(id);
    ctx.decisions.set(id, {
      source,
      externalId: ref.externalId,
      modelKey: resolution.key,
      method: resolution.method,
      reviewed: resolution.reviewed,
      ...(previous?.note ? { note: previous.note } : {}),
    });
    return resolution;
  }
  const manualNoMatch = ctx.aliases.get(id);
  if (manualNoMatch && manualNoMatch.method === "manual" && manualNoMatch.modelKey === null) {
    ctx.decisions.set(id, manualNoMatch);
  }
  const entry = ctx.unmapped.get(id) ?? {
    source,
    externalModel: ref.externalId,
    name: usage.name,
    benchmarks: new Set<string>(),
    newestDate: null,
  };
  entry.benchmarks.add(usage.benchmark);
  if (usage.date && (entry.newestDate === null || usage.date > entry.newestDate)) entry.newestDate = usage.date;
  ctx.unmapped.set(id, entry);
  return resolution;
}

/** Alias rules to write: this run's decisions plus manual rules that did not come up. */
export function aliasRulesForWrite(ctx: SyncContext): AliasRule[] {
  const out = new Map(ctx.decisions);
  for (const [id, rule] of ctx.aliases) {
    if (rule.method === "manual" && !out.has(id)) out.set(id, rule);
  }
  return [...out.values()].sort(
    (a, b) => (a.source < b.source ? -1 : a.source > b.source ? 1 : a.externalId < b.externalId ? -1 : a.externalId > b.externalId ? 1 : 0),
  );
}
