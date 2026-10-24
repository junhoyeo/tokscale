import { Suspense } from "react";
import type { Metadata } from "next";
import { cookies } from "next/headers";
import { Navigation } from "@/components/layout/Navigation";
import { PublicFooter } from "@/components/layout/PublicFooter";
import { LeaderboardSkeleton } from "@/components/Skeleton";
import { getLeaderboardData } from "@/lib/leaderboard/getLeaderboard";
import type { LeaderboardData, Period, SortBy } from "@/lib/leaderboard/types";
import { getSession } from "@/lib/auth/session";
import {
  SORT_BY_COOKIE_NAME,
  resolveSortByParam,
} from "@/lib/leaderboard/constants";
import { parseCustomDateRange } from "@/lib/leaderboard/dateRange";
import { listPublicGroups, listUserGroups } from "@/lib/groups/queries";
import { leaderboardUrl } from "@/lib/seo/urls";
import { exploreLeaderboardUrl } from "@/lib/seo/explore";
import { getAgentLeaderboard, getModelLeaderboard } from "@/lib/explore";
import {
  DEFAULT_EXPLORE_PERIOD,
  DEFAULT_EXPLORE_SORT,
  EXPLORE_PERIODS,
  EXPLORE_SORTS,
  periodShortLabel,
  resolveExplorePeriod,
  resolveExploreSort,
  sortLabel,
} from "@/components/explore/format";
import { ExploreRanking } from "@/components/explore/ExploreRanking";
import LeaderboardClient from "./LeaderboardClient";
import GroupsBrowser from "./GroupsBrowser";
import ViewSelector from "./ViewSelector";
import { buildExploreHref, resolveLeaderboardView, type LeaderboardView } from "./views";
import { TOKSCALE_X_HANDLE } from "@/lib/community";

function isMissingDatabaseUrl(error: unknown): boolean {
  return error instanceof Error && error.message === "DATABASE_URL environment variable is not set";
}

const VALID_PERIODS: Period[] = ["all", "month", "last-month", "week", "custom"];

function createEmptyLeaderboardData(sortBy: SortBy): LeaderboardData {
  return {
    users: [],
    pagination: {
      page: 1,
      limit: 50,
      totalUsers: 0,
      totalPages: 0,
      hasNext: false,
      hasPrev: false,
    },
    stats: {
      totalTokens: 0,
      totalCost: 0,
      uniqueUsers: 0,
    },
    period: "all",
    sortBy,
  };
}

const EXPLORE_PAGE_SIZE = 50;

const VIEW_META: Record<LeaderboardView, { title: string; description: string }> = {
  humans: {
    title: "Leaderboard - Who Burns the Most AI Tokens | Tokscale",
    description:
      "Public ranking of developers by AI coding assistant token usage and estimated cost, across Claude Code, Cursor, Codex, Copilot, Gemini, and more.",
  },
  models: {
    title: "AI Model Leaderboard - Models Ranked by Active Users | Tokscale",
    description:
      "Which AI models developers use for coding: active users, adoption, tokens, and estimated cost per model, from public Tokscale submissions.",
  },
  agents: {
    title: "AI Coding Agent Leaderboard - Agents Ranked by Active Users | Tokscale",
    description:
      "Which coding agents developers use: active users, adoption, tokens, estimated cost, and top models per agent, from public Tokscale submissions.",
  },
  groups: {
    title: "Groups Leaderboard - Team AI Token Usage | Tokscale",
    description:
      "Browse public groups and compare combined AI coding assistant token usage and spend across teams, companies, and communities.",
  },
};

function canonicalForView(view: LeaderboardView): string {
  if (view === "models" || view === "agents") return exploreLeaderboardUrl(view);
  return leaderboardUrl(view === "groups" ? "groups" : "users");
}

interface PageProps {
  searchParams: Promise<{ [key: string]: string | string[] | undefined }>;
}

/**
 * Collapses the filter params onto one canonical URL per view.
 *
 * period, sort/sortBy, page, from/to and search are all views of the same
 * ranking, and `search` in particular spans an unbounded set of URLs that
 * would otherwise be crawled as distinct near-duplicates. `view` selects a
 * genuinely different page (humans, models, agents, groups), so the non-default
 * views canonicalize to themselves. The legacy `view=users` collapses onto the
 * bare humans URL.
 */
export async function generateMetadata({ searchParams }: PageProps): Promise<Metadata> {
  const params = await searchParams;
  const view = resolveLeaderboardView(params.view);
  const canonical = canonicalForView(view);
  const { title, description } = VIEW_META[view];

  return {
    title,
    description,
    alternates: { canonical },
    openGraph: {
      title,
      description,
      type: "website",
      url: canonical,
      siteName: "Tokscale",
    },
    twitter: {
      card: "summary_large_image",
      site: TOKSCALE_X_HANDLE,
      title,
      description,
    },
  };
}

export default function LeaderboardPage({ searchParams }: PageProps) {
  return (
    <div className="service-page-shell">
      <Navigation />

      <main className="service-main" id="main-content">
        <Suspense fallback={<LeaderboardSkeleton />}>
          <LeaderboardWithPreferences searchParams={searchParams} />
        </Suspense>
      </main>

      <PublicFooter />
    </div>
  );
}

async function LeaderboardWithPreferences({
  searchParams: searchParamsPromise,
}: {
  searchParams: Promise<{ [key: string]: string | string[] | undefined }>;
}) {
  const [cookieStore, searchParams] = await Promise.all([cookies(), searchParamsPromise]);
  const view = resolveLeaderboardView(searchParams.view);

  if (view === "models" || view === "agents") {
    return (
      <>
        <ViewSelector current={view} searchParams={searchParams} />
        <ExploreView view={view} searchParams={searchParams} />
      </>
    );
  }

  if (view === "groups") {
    return (
      <>
        <ViewSelector current="groups" searchParams={searchParams} />
        <GroupsView />
      </>
    );
  }

  const sortByCookie = cookieStore.get(SORT_BY_COOKIE_NAME)?.value;
  const periodParam = typeof searchParams.period === "string" ? searchParams.period : null;
  const pageParam =
    typeof searchParams.page === "string" ? Math.max(1, Number(searchParams.page) || 1) : 1;
  const sortByParam = typeof searchParams.sortBy === "string" ? searchParams.sortBy : null;
  const fromParam = typeof searchParams.from === "string" ? searchParams.from : null;
  const toParam = typeof searchParams.to === "string" ? searchParams.to : null;
  const searchParam =
    typeof searchParams.search === "string" ? searchParams.search.trim() : "";

  const sortBy: SortBy =
    resolveSortByParam(sortByParam) ?? resolveSortByParam(sortByCookie) ?? "tokens";

  let period: Period =
    periodParam && VALID_PERIODS.includes(periodParam as Period)
      ? (periodParam as Period)
      : "all";

  const customDateRange =
    period === "custom" ? parseCustomDateRange(fromParam, toParam) : null;

  if (period === "custom" && !customDateRange) {
    period = "all";
  }

  const customFrom = customDateRange?.from;
  const customTo = customDateRange?.to;

  const [initialData, session] = await Promise.all([
    getLeaderboardData(period, pageParam, 50, sortBy, searchParam, customFrom, customTo).catch((error) => {
      if (isMissingDatabaseUrl(error)) {
        return createEmptyLeaderboardData(sortBy);
      }
      throw error;
    }),
    getSession().catch((error) => {
      if (isMissingDatabaseUrl(error)) {
        return null;
      }
      throw error;
    }),
  ]);

  return (
    <>
      <ViewSelector current="humans" searchParams={searchParams} />
      <LeaderboardClient
        initialData={initialData}
        currentUser={session}
        initialSortBy={sortBy}
      />
    </>
  );
}

async function GroupsView() {
  const emptyPagination = {
    page: 1,
    limit: 20,
    total: 0,
    totalPages: 0,
    hasNext: false,
    hasPrev: false,
  } as const;

  const session = await getSession().catch((error) => {
    if (isMissingDatabaseUrl(error)) return null;
    throw error;
  });

  const [publicGroups, myGroups] = await Promise.all([
    listPublicGroups(1, 20).catch((error) => {
      if (isMissingDatabaseUrl(error)) {
        return { groups: [], pagination: emptyPagination };
      }
      throw error;
    }),
    session
      ? listUserGroups(session.id, 1, 20).catch((error) => {
          if (isMissingDatabaseUrl(error)) {
            return { groups: [], pagination: emptyPagination };
          }
          throw error;
        })
      : Promise.resolve(null),
  ]);

  return (
    <GroupsBrowser
      currentUser={session}
      initialPublicGroups={publicGroups.groups}
      initialMyGroups={myGroups?.groups ?? []}
      initialPublicPagination={publicGroups.pagination}
      initialMyPagination={myGroups?.pagination ?? emptyPagination}
    />
  );
}

async function ExploreView({
  view,
  searchParams,
}: {
  view: "models" | "agents";
  searchParams: { [key: string]: string | string[] | undefined };
}) {
  const period = resolveExplorePeriod(searchParams.period);
  const sort = resolveExploreSort(searchParams.sort);
  const query = { period, sort, limit: EXPLORE_PAGE_SIZE };
  const current = { view, period, sort };
  const defaults = { period: DEFAULT_EXPLORE_PERIOD, sort: DEFAULT_EXPLORE_SORT };

  const periodOptions = EXPLORE_PERIODS.map((value) => ({
    href: buildExploreHref("/leaderboard", current, { period: value }, defaults),
    label: periodShortLabel(value),
    active: value === period,
  }));
  const sortOptions = EXPLORE_SORTS.map((value) => ({
    href: buildExploreHref("/leaderboard", current, { sort: value }, defaults),
    label: sortLabel(value),
    active: value === sort,
  }));

  if (view === "models") {
    const ranked = await getModelLeaderboard(query);
    return (
      <ExploreRanking
        kind="models"
        rows={ranked.rows}
        coverage={ranked.coverage}
        period={period}
        sort={sort}
        periodOptions={periodOptions}
        sortOptions={sortOptions}
        truncated={ranked.rows.length >= EXPLORE_PAGE_SIZE}
      />
    );
  }

  const ranked = await getAgentLeaderboard(query);
  return (
    <ExploreRanking
      kind="agents"
      rows={ranked.rows}
      coverage={ranked.coverage}
      period={period}
      sort={sort}
      periodOptions={periodOptions}
      sortOptions={sortOptions}
      truncated={ranked.rows.length >= EXPLORE_PAGE_SIZE}
    />
  );
}
