/**
 * Report and file formatting for the sync. Pure functions only.
 */
import type { CatalogSnapshot, SourceSnapshot, SyncReport } from "../schema";
import type { AliasRule } from "../mapping";
import type { SyncContext } from "./context";

export interface SourceStatus {
  source: SyncReport["sources"][number]["source"];
  ok: boolean;
  error: string | null;
}

export function buildReport(
  ctx: SyncContext,
  snapshots: readonly SourceSnapshot[],
  catalog: CatalogSnapshot | null,
  statuses: readonly SourceStatus[],
  rules: readonly AliasRule[],
  generatedAt: string,
): SyncReport {
  const sources: SyncReport["sources"] = statuses.map((status) => {
    if (status.source === "models.dev") {
      const count = catalog?.models.length ?? 0;
      return { ...status, results: count, mappedResults: count, externalModels: count, mappedModels: count };
    }
    const snapshot = snapshots.find((s) => s.source === status.source);
    const results = snapshot?.benchmarks.flatMap((b) => b.results) ?? [];
    const models = new Map<string, boolean>();
    for (const r of results) models.set(r.externalModel, (models.get(r.externalModel) ?? false) || r.modelKey !== null);
    return {
      ...status,
      results: results.length,
      mappedResults: results.filter((r) => r.modelKey !== null).length,
      externalModels: models.size,
      mappedModels: [...models.values()].filter(Boolean).length,
    };
  });

  const methods = { exact: 0, normalized: 0, manual: 0, unreviewed: 0 };
  for (const rule of rules) {
    if (rule.modelKey === null) continue;
    methods[rule.method] += 1;
    if (!rule.reviewed) methods.unreviewed += 1;
  }

  const unmapped = [...ctx.unmapped.values()]
    .map((u) => ({
      source: u.source as SyncReport["unmapped"][number]["source"],
      externalModel: u.externalModel,
      name: u.name,
      benchmarks: [...u.benchmarks].sort(),
      newestDate: u.newestDate,
    }))
    .sort((a, b) => (b.newestDate ?? "").localeCompare(a.newestDate ?? "") || a.externalModel.localeCompare(b.externalModel));

  return { generatedAt, sources, methods, unmapped };
}

/**
 * JSON with one array element per line for large arrays of flat objects
 * (results, models, aliases), so diffs between snapshots stay readable.
 */
export function formatSnapshotJson(value: unknown): string {
  return `${format(value, 0)}\n`;
}

function isFlatObject(value: unknown): boolean {
  return (
    typeof value === "object" &&
    value !== null &&
    !Array.isArray(value) &&
    Object.values(value).every((v) => v === null || typeof v !== "object" || (Array.isArray(v) && v.every((x) => typeof x !== "object")))
  );
}

function format(value: unknown, depth: number): string {
  const pad = "  ".repeat(depth);
  const inner = "  ".repeat(depth + 1);
  if (Array.isArray(value)) {
    if (value.length === 0) return "[]";
    const lines = value.map((item) => (isFlatObject(item) ? JSON.stringify(item) : format(item, depth + 1)));
    return `[\n${lines.map((line) => `${inner}${line}`).join(",\n")}\n${pad}]`;
  }
  if (typeof value === "object" && value !== null) {
    const entries = Object.entries(value).filter(([, v]) => v !== undefined);
    if (entries.length === 0) return "{}";
    const lines = entries.map(([k, v]) => `${inner}${JSON.stringify(k)}: ${format(v, depth + 1)}`);
    return `{\n${lines.join(",\n")}\n${pad}}`;
  }
  return JSON.stringify(value);
}
