import { Suspense } from "react";
import { cookies } from "next/headers";
import { Navigation } from "@/components/layout/Navigation";
import { ServiceFooter } from "@/components/layout/ServiceFooter";
import { LeaderboardSkeleton } from "@/components/Skeleton";
import { getLeaderboardData, getUserRank } from "@/lib/leaderboard/getLeaderboard";
import type { LeaderboardData, Period, SortBy } from "@/lib/leaderboard/types";
import { getSession } from "@/lib/auth/session";
import { SORT_BY_COOKIE_NAME, isValidSortBy } from "@/lib/leaderboard/constants";
import { parseCustomDateRange } from "@/lib/leaderboard/dateRange";
import { listPublicGroups, listUserGroups } from "@/lib/groups/queries";
import LeaderboardClient from "./LeaderboardClient";
import GroupsBrowser from "./GroupsBrowser";
import ViewSelector, { type LeaderboardView } from "./ViewSelector";

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
      totalActiveTimeMs: null,
      uniqueUsers: 0,
    },
    period: "all",
    sortBy,
  };
}

function resolveView(raw: string | string[] | undefined): LeaderboardView {
  return raw === "groups" ? "groups" : "users";
}

interface PageProps {
  searchParams: Promise<{ [key: string]: string | string[] | undefined }>;
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

      <ServiceFooter />
    </div>
  );
}

async function LeaderboardWithPreferences({
  searchParams: searchParamsPromise,
}: {
  searchParams: Promise<{ [key: string]: string | string[] | undefined }>;
}) {
  const [cookieStore, searchParams] = await Promise.all([cookies(), searchParamsPromise]);
  const view = resolveView(searchParams.view);

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
    sortByParam && isValidSortBy(sortByParam)
      ? sortByParam
      : isValidSortBy(sortByCookie)
      ? sortByCookie
      : "tokens";

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

  const initialUserRank = session
    ? await getUserRank(session.username, period, sortBy, customFrom, customTo).catch((error) => {
        if (isMissingDatabaseUrl(error)) {
          return null;
        }
        throw error;
      })
    : null;

  return (
    <>
      <ViewSelector current="users" searchParams={searchParams} />
      <LeaderboardClient
        initialData={initialData}
        currentUser={session}
        initialSortBy={sortBy}
        initialUserRank={initialUserRank}
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
