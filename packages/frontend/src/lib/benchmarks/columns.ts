/**
 * Column groups and sorting for the /models dashboard, with their URL form
 * (`?cols=usage,coding&sort=terminal-bench-4`). Pure, so the server page,
 * the client table and tests share one implementation.
 */

export const COLUMN_GROUPS = ["usage", "capability", "coding", "webdev", "price", "specs"] as const;
export type ColumnGroup = (typeof COLUMN_GROUPS)[number];

export const COLUMN_GROUP_LABELS: Record<ColumnGroup, string> = {
  usage: "Usage",
  capability: "Capability",
  coding: "Agentic coding",
  webdev: "Web dev",
  price: "Price",
  specs: "Specs",
};

/**
 * No group selected means the overview: one headline column per group
 * (active users, ECI, Terminal-Bench 4.0, WebDev Arena, price at the
 * community token mix). Selecting groups shows their full column sets.
 */
export const DEFAULT_COLUMN_GROUPS: readonly ColumnGroup[] = [];

/** Headline columns of the overview, as sort keys. */
export const OVERVIEW_SORT_KEYS: readonly string[] = ["users", "epoch-capabilities-index", "terminal-bench-4", "webdev-arena", "price-community"];

/** Benchmarks shown in each score group, in column order. */
export const GROUP_BENCHMARKS: Record<"capability" | "coding" | "webdev", readonly string[]> = {
  capability: ["epoch-capabilities-index", "gpqa-diamond", "hle", "frontiermath-tiers-1-3", "simpleqa-verified"],
  coding: ["terminal-bench-4", "cursorbench", "deepswe", "frontierswe", "agent-arena"],
  webdev: ["webdev-arena"],
};

export const DASHBOARD_BENCHMARKS: readonly string[] = [
  ...GROUP_BENCHMARKS.capability,
  ...GROUP_BENCHMARKS.coding,
  ...GROUP_BENCHMARKS.webdev,
];

type RawParam = string | string[] | undefined | null;

function first(raw: RawParam): string {
  return (Array.isArray(raw) ? raw[0] : raw) ?? "";
}

function isGroup(value: string): value is ColumnGroup {
  return (COLUMN_GROUPS as readonly string[]).includes(value);
}

/** Parse `?cols=`. Unknown names are dropped; nothing valid means the overview. Order is canonical. */
export function parseColumnGroups(raw: RawParam): ColumnGroup[] {
  const wanted = new Set(
    first(raw)
      .split(",")
      .map((v) => v.trim().toLowerCase())
      .filter(isGroup),
  );
  return COLUMN_GROUPS.filter((g) => wanted.has(g));
}

function sameGroups(a: readonly ColumnGroup[], b: readonly ColumnGroup[]): boolean {
  return a.length === b.length && a.every((g, i) => g === b[i]);
}

/** Value for `?cols=`, or null for the overview. */
export function columnGroupsParam(groups: readonly ColumnGroup[]): string | null {
  const canonical = COLUMN_GROUPS.filter((g) => groups.includes(g));
  if (canonical.length === 0 || sameGroups(canonical, DEFAULT_COLUMN_GROUPS)) return null;
  return canonical.join(",");
}

/** Toggle one group. Turning off the last group returns to the overview. */
export function toggleColumnGroup(groups: readonly ColumnGroup[], group: ColumnGroup): ColumnGroup[] {
  if (groups.includes(group)) return groups.filter((g) => g !== group);
  return COLUMN_GROUPS.filter((g) => g === group || groups.includes(g));
}

/** Sort keys whose columns are visible for these groups. */
export function visibleSortKeys(groups: readonly ColumnGroup[]): string[] {
  if (groups.length === 0) return [...OVERVIEW_SORT_KEYS];
  return SORT_KEYS.filter((key) => groups.includes(groupForSortKey(key)));
}

/** Keep the sort when its column is visible; otherwise fall back to active users or the first visible column. */
export function effectiveSortKey(sort: string, groups: readonly ColumnGroup[]): string {
  const visible = visibleSortKeys(groups);
  if (visible.includes(sort)) return sort;
  return visible.includes(DEFAULT_SORT) ? DEFAULT_SORT : (visible[0] ?? DEFAULT_SORT);
}

/** Sortable columns. Scores and usage sort high-to-low; prices low-to-high. */
export const SORT_KEYS = [
  "users",
  "tokens",
  ...DASHBOARD_BENCHMARKS,
  "price-community",
  "price-own",
  "price-input",
  "context",
  "released",
] as const;
export type SortKey = string;

export const DEFAULT_SORT: SortKey = "users";

export function parseSortKey(raw: RawParam): SortKey {
  const value = first(raw).trim().toLowerCase();
  return (SORT_KEYS as readonly string[]).includes(value) ? value : DEFAULT_SORT;
}

export function sortAscending(key: SortKey): boolean {
  return key.startsWith("price-");
}

/** Which group a sort key belongs to, so a sort on a hidden column can fall back. */
export function groupForSortKey(key: SortKey): ColumnGroup {
  if (key === "users" || key === "tokens") return "usage";
  if (key.startsWith("price-")) return "price";
  if (key === "context" || key === "released") return "specs";
  if (GROUP_BENCHMARKS.capability.includes(key)) return "capability";
  if (GROUP_BENCHMARKS.webdev.includes(key)) return "webdev";
  return "coding";
}

/** Build the dashboard query string for groups, sort and period (defaults left out). */
export function dashboardQuery(state: { groups: readonly ColumnGroup[]; sort: SortKey; period: string; defaultPeriod: string }): string {
  const params = new URLSearchParams();
  if (state.period !== state.defaultPeriod) params.set("period", state.period);
  const cols = columnGroupsParam(state.groups);
  if (cols) params.set("cols", cols);
  if (state.sort !== DEFAULT_SORT) params.set("sort", state.sort);
  // Commas stay readable in ?cols=usage,coding.
  const query = params.toString().replace(/%2C/g, ",");
  return query ? `?${query}` : "";
}
