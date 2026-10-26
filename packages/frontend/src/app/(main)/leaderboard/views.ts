/**
 * View routing for /leaderboard. Pure module (no "use client") so the server
 * page, the client view selector and tests share one implementation.
 *
 * Views:
 * - humans (default): the existing people ranking. `?view=users` is the old
 *   name and still resolves here, so shared links keep working.
 * - models / agents: explore rankings with ?period=7d|30d|all and ?sort=users|tokens.
 * - groups: the group directory.
 */
import { EXPLORE_PERIODS, EXPLORE_SORTS } from "@/components/explore/format";

export type LeaderboardView = "humans" | "models" | "agents" | "groups";

export const LEADERBOARD_VIEWS: readonly LeaderboardView[] = ["humans", "models", "agents", "groups"];

export const LEADERBOARD_VIEW_LABELS: Record<LeaderboardView, string> = {
  humans: "Humans",
  models: "Models",
  agents: "Agents",
  groups: "Groups",
};

export type LeaderboardSearchParams = Record<string, string | string[] | undefined>;

export function resolveLeaderboardView(raw: string | string[] | undefined): LeaderboardView {
  const value = Array.isArray(raw) ? raw[0] : raw;
  if (value === "models" || value === "agents" || value === "groups") return value;
  // "users" is the legacy name of the humans view.
  return "humans";
}

/** Filters owned by the humans ranking (and round-tripped through groups). */
const HUMAN_KEYS = new Set(["period", "sortBy", "from", "to", "search"]);
const HUMAN_PERIODS = new Set(["all", "month", "last-month", "week", "custom"]);

function appendParam(params: URLSearchParams, key: string, value: string | string[]) {
  if (Array.isArray(value)) {
    for (const item of value) params.append(key, item);
  } else {
    params.set(key, value);
  }
}

function first(value: string | string[] | undefined): string | undefined {
  return Array.isArray(value) ? value[0] : value;
}

/**
 * Build the link for a view tab, carrying only the filters the destination
 * understands. Pagination is always dropped.
 *
 * - humans/groups keep the humans filters (groups ignores them, but keeping
 *   them makes Humans -> Groups -> Humans a round trip). from/to only travel
 *   with period=custom so they do not prime stale date inputs.
 * - models/agents keep period when it is 7d/30d/all, and sort.
 * - A humans link has no view param: /leaderboard is the humans view.
 */
export function buildLeaderboardViewHref(searchParams: LeaderboardSearchParams, view: LeaderboardView): string {
  const params = new URLSearchParams();
  const period = first(searchParams.period);

  if (view === "humans" || view === "groups") {
    for (const [key, value] of Object.entries(searchParams)) {
      if (value === undefined || !HUMAN_KEYS.has(key)) continue;
      if (key === "period" && (!period || !HUMAN_PERIODS.has(period))) continue;
      if ((key === "from" || key === "to") && period !== "custom") continue;
      appendParam(params, key, value);
    }
  } else {
    if (period && (EXPLORE_PERIODS as readonly string[]).includes(period)) params.set("period", period);
    const sort = first(searchParams.sort);
    if (sort && (EXPLORE_SORTS as readonly string[]).includes(sort)) params.set("sort", sort);
  }

  if (view !== "humans") params.set("view", view);
  const query = params.toString();
  return query ? `/leaderboard?${query}` : "/leaderboard";
}

/**
 * Link for an explore control (period or sort) inside the models/agents view.
 * Default values are left out so the canonical form stays short.
 */
export function buildExploreHref(
  pathname: string,
  current: { view?: LeaderboardView; period: string; sort?: string },
  change: { period?: string; sort?: string },
  defaults: { period: string; sort?: string },
): string {
  const params = new URLSearchParams();
  if (current.view && current.view !== "humans") params.set("view", current.view);
  const period = change.period ?? current.period;
  const sort = change.sort ?? current.sort;
  if (period !== defaults.period) params.set("period", period);
  if (sort && sort !== defaults.sort) params.set("sort", sort);
  const query = params.toString();
  return query ? `${pathname}?${query}` : pathname;
}
