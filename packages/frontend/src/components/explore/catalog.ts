/**
 * Pure helpers for the model catalog (/models/all): grouping by lab, search
 * matching, and collapsing each lab's long tail.
 */
import type { ExploreSort, ModelUsageRow } from "@/lib/explore";

export const UNKNOWN_LAB_KEY = "unknown";
export const UNKNOWN_LAB_NAME = "Lab unknown";

/** Models listed per lab before the rest collapse behind a "Show N more" button. */
export const LAB_PREVIEW_SIZE = 5;
/** A lab with at most this many models past the preview lists them all ("Show 1 more" helps nobody). */
export const LAB_PREVIEW_SLACK = 2;

/** The fields the catalog reads from a usage row. */
export type CatalogModel = Pick<
  ModelUsageRow,
  "key" | "name" | "lab" | "labSlug" | "logoUrl" | "activeUsers" | "adoption" | "tokens" | "estimatedCost"
>;

export interface LabGroup<T extends CatalogModel = ModelUsageRow> {
  /** labSlug, or UNKNOWN_LAB_KEY when the lab is not identified. */
  key: string;
  name: string;
  /** Slug for /capital/companies/[slug], or null. */
  slug: string | null;
  models: T[];
}

/** Keep only what the catalog renders, so the client payload stays small. */
export function toCatalogModel(row: CatalogModel): CatalogModel {
  return {
    key: row.key,
    name: row.name,
    lab: row.lab,
    labSlug: row.labSlug,
    logoUrl: row.logoUrl,
    activeUsers: row.activeUsers,
    adoption: row.adoption,
    tokens: row.tokens,
    estimatedCost: row.estimatedCost,
  };
}

function metric(row: CatalogModel, sort: ExploreSort): number {
  return sort === "tokens" ? row.tokens : row.activeUsers;
}

/**
 * Group models by lab. Models inside a lab follow the sort; labs are ordered
 * by their strongest model on the same metric, with the unknown lab last.
 */
export function groupModelsByLab<T extends CatalogModel>(rows: readonly T[], sort: ExploreSort): LabGroup<T>[] {
  const groups = new Map<string, LabGroup<T>>();
  for (const row of rows) {
    const key = row.labSlug ?? (row.lab ? row.lab.toLowerCase().replace(/[^a-z0-9]+/g, "-") : UNKNOWN_LAB_KEY);
    const existing = groups.get(key);
    if (existing) {
      existing.models.push(row);
    } else {
      groups.set(key, { key, name: row.lab ?? UNKNOWN_LAB_NAME, slug: row.labSlug, models: [row] });
    }
  }

  const list = [...groups.values()];
  for (const group of list) {
    group.models.sort((a, b) => metric(b, sort) - metric(a, sort) || a.name.localeCompare(b.name));
  }
  return list.sort((a, b) => {
    if (a.key === UNKNOWN_LAB_KEY) return 1;
    if (b.key === UNKNOWN_LAB_KEY) return -1;
    return metric(b.models[0], sort) - metric(a.models[0], sort) || a.name.localeCompare(b.name);
  });
}

/** Case-insensitive match on model name, key and lab. Every word must match. */
export function matchesModelQuery(row: Pick<ModelUsageRow, "name" | "key" | "lab">, query: string): boolean {
  const words = query.trim().toLowerCase().split(/\s+/).filter(Boolean);
  if (words.length === 0) return true;
  const haystack = `${row.name} ${row.key} ${row.lab ?? UNKNOWN_LAB_NAME}`.toLowerCase();
  return words.every((word) => haystack.includes(word));
}

/** A search or a single lab narrows the list; then every match is listed and nothing collapses. */
export function isCatalogFiltered(query: string, lab: string): boolean {
  return query.trim().length > 0 || lab !== "all";
}

export interface LabPreview<T> {
  shown: T[];
  /** Models behind the "Show N more" button; 0 when the whole lab is listed. */
  hidden: number;
}

/**
 * Split a lab's sorted models into the listed head and the collapsed long
 * tail. Nothing collapses when `collapse` is false or the tail is short.
 */
export function previewLabModels<T>(
  models: readonly T[],
  collapse: boolean,
  size: number = LAB_PREVIEW_SIZE,
  slack: number = LAB_PREVIEW_SLACK,
): LabPreview<T> {
  if (!collapse || models.length <= size + slack) return { shown: [...models], hidden: 0 };
  return { shown: models.slice(0, size), hidden: models.length - size };
}
