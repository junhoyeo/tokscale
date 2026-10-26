import { CATEGORIES, ORG_STATUSES, type Category, type OrgStatus } from "./schema";

/**
 * URL state for the companies directory. Every filter, sort and view lives in the query
 * string so a view can be shared; the page canonicalizes to the bare URL.
 */
export const DIRECTORY_VIEWS = ["overview", "funding", "compute", "public", "models"] as const;
export type DirectoryView = (typeof DIRECTORY_VIEWS)[number];
export const DIRECTORY_SORTS = ["activity", "name", "valuation", "equity", "deals", "capex"] as const;
export type DirectorySort = (typeof DIRECTORY_SORTS)[number];
export const DIRECTORY_FLAGS = ["compute", "models", "rounds"] as const;
export type DirectoryFlag = (typeof DIRECTORY_FLAGS)[number];

export interface DirectoryParams {
  q: string;
  category: Category | null;
  country: string | null;
  status: OrgStatus | null;
  view: DirectoryView;
  sort: DirectorySort;
  has: DirectoryFlag[];
}

export const DEFAULT_DIRECTORY_PARAMS: DirectoryParams = {
  q: "",
  category: null,
  country: null,
  status: null,
  view: "overview",
  sort: "activity",
  has: [],
};

type RawParams = Record<string, string | string[] | undefined>;

const first = (value: string | string[] | undefined) => (Array.isArray(value) ? value[0] : value) ?? "";
const oneOf = <T extends string>(list: readonly T[], value: string): T | null => (list.includes(value as T) ? (value as T) : null);

/** Unknown values fall back to defaults instead of erroring: a shared link with a typo still renders. */
export function parseDirectoryParams(raw: RawParams): DirectoryParams {
  const has = (Array.isArray(raw.has) ? raw.has : (raw.has ?? "").split(","))
    .map((v) => oneOf(DIRECTORY_FLAGS, v.trim()))
    .filter((v): v is DirectoryFlag => v !== null);
  const country = first(raw.country).toUpperCase();
  return {
    q: first(raw.q).trim().slice(0, 80),
    category: oneOf(CATEGORIES, first(raw.category)),
    country: /^[A-Z]{2}$/.test(country) ? country : null,
    status: oneOf(ORG_STATUSES, first(raw.status)),
    view: oneOf(DIRECTORY_VIEWS, first(raw.view)) ?? "overview",
    sort: oneOf(DIRECTORY_SORTS, first(raw.sort)) ?? "activity",
    has: [...new Set(has)].sort(),
  };
}

/** Query string for a params object, omitting defaults. "" for the default view. */
export function directoryQuery(params: Partial<DirectoryParams>): string {
  const merged = { ...DEFAULT_DIRECTORY_PARAMS, ...params };
  const search = new URLSearchParams();
  if (merged.q) search.set("q", merged.q);
  if (merged.category) search.set("category", merged.category);
  if (merged.country) search.set("country", merged.country);
  if (merged.status) search.set("status", merged.status);
  if (merged.has.length > 0) search.set("has", [...merged.has].sort().join(","));
  if (merged.view !== "overview") search.set("view", merged.view);
  if (merged.sort !== "activity") search.set("sort", merged.sort);
  const text = search.toString();
  return text ? `?${text}` : "";
}

export interface DirectoryRowLike {
  name: string;
  aliases: readonly string[];
  legalNames: readonly string[];
  tickers: readonly string[];
  categories: readonly Category[];
  country: string | null;
  status: OrgStatus;
  lastActivity: string | null;
  valuationUsd: number | null;
  equityUsd: number | null;
  capexUsd: number | null;
  dealsCount: number;
  roundsCount: number;
  modelsCount: number | null;
}

const fold = (text: string) =>
  text
    .normalize("NFKD")
    .replace(/[\u0300-\u036f]/g, "")
    .toLowerCase()
    .replace(/[^a-z0-9]+/g, " ")
    .trim();

/** Case- and accent-insensitive match on names, legal names, aliases and tickers ("zhipu" finds Z.ai). */
export function matchesQuery(row: Pick<DirectoryRowLike, "name" | "aliases" | "legalNames" | "tickers">, q: string): boolean {
  const needle = fold(q);
  if (!needle) return true;
  const haystack = [row.name, ...row.aliases, ...row.legalNames, ...row.tickers].map(fold);
  return haystack.some((h) => h.includes(needle) || h.replace(/ /g, "").includes(needle.replace(/ /g, "")));
}

export function filterDirectory<T extends DirectoryRowLike>(rows: readonly T[], params: DirectoryParams): T[] {
  return rows.filter(
    (row) =>
      matchesQuery(row, params.q) &&
      (!params.category || row.categories.includes(params.category)) &&
      (!params.country || row.country === params.country) &&
      (!params.status || row.status === params.status) &&
      (!params.has.includes("compute") || row.dealsCount > 0) &&
      (!params.has.includes("rounds") || row.roundsCount > 0) &&
      (!params.has.includes("models") || (row.modelsCount ?? 0) > 0)
  );
}

/** Sorts never invent values: rows without the sort value go last, then by name. */
export function sortDirectory<T extends DirectoryRowLike>(rows: readonly T[], sort: DirectorySort): T[] {
  const key = (row: T): number | string | null => {
    switch (sort) {
      case "name":
        return null;
      case "activity":
        return row.lastActivity;
      case "valuation":
        return row.valuationUsd;
      case "equity":
        return row.equityUsd;
      case "deals":
        return row.dealsCount > 0 ? row.dealsCount : null;
      case "capex":
        return row.capexUsd;
    }
  };
  return [...rows].sort((a, b) => {
    const ka = key(a);
    const kb = key(b);
    if (ka !== kb) {
      if (ka === null) return 1;
      if (kb === null) return -1;
      if (typeof ka === "number" && typeof kb === "number") return kb - ka;
      return String(kb).localeCompare(String(ka));
    }
    return a.name.localeCompare(b.name);
  });
}

/** Preset views shown as links ("saved views" without an account). */
export const DIRECTORY_PRESETS: ReadonlyArray<{ label: string; params: Partial<DirectoryParams> }> = [
  { label: "AI labs by valuation", params: { category: "ai-lab", sort: "valuation", view: "funding" } },
  { label: "Neoclouds by contracts", params: { category: "neocloud", view: "compute", sort: "deals" } },
  { label: "Chinese labs", params: { category: "ai-lab", country: "CN" } },
  { label: "Public AI infrastructure", params: { status: "public", view: "public", sort: "capex" } },
  { label: "Labs with models on Tokscale", params: { has: ["models"], view: "models" } },
];
