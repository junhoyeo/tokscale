import { Suspense } from "react";
import { cookies } from "next/headers";
import { Navigation } from "@/components/layout/Navigation";
import { Footer } from "@/components/layout/Footer";
import { BlackholeHero } from "@/components/BlackholeHero";
import { LeaderboardSkeleton } from "@/components/Skeleton";
import { getLeaderboardData, getUserRank } from "@/lib/leaderboard/getLeaderboard";
import type { LeaderboardData, Period, SortBy } from "@/lib/leaderboard/types";
import { getSession } from "@/lib/auth/session";
import { SORT_BY_COOKIE_NAME, isValidSortBy } from "@/lib/leaderboard/constants";

function isMissingDatabaseUrl(error: unknown): boolean {
  return error instanceof Error && error.message === "DATABASE_URL environment variable is not set";
}
import LeaderboardClient from "./LeaderboardClient";

const VALID_PERIODS: Period[] = ["all", "month", "last-month", "week", "custom"];
const DATE_REGEX = /^\d{4}-\d{2}-\d{2}$/;

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
      totalSubmissions: null,
      uniqueUsers: 0,
    },
    period: "all",
    sortBy,
  };
}

interface PageProps {
  searchParams: Promise<{ [key: string]: string | string[] | undefined }>;
}

export default function LeaderboardPage({ searchParams }: PageProps) {
  return (
    <div
      style={{
        minHeight: "100vh",
        display: "flex",
        flexDirection: "column",
        backgroundColor: "var(--color-bg-default)",
      }}
    >
      <Navigation />

      <main className="main-container">
        <BlackholeHero />
        <Suspense fallback={<LeaderboardSkeleton />}>
          <LeaderboardWithPreferences searchParams={searchParams} />
        </Suspense>
      </main>

      <Footer />
    </div>
  );
}

async function LeaderboardWithPreferences({ searchParams: searchParamsPromise }: { searchParams: Promise<{ [key: string]: string | string[] | undefined }> }) {
  const [cookieStore, searchParams] = await Promise.all([cookies(), searchParamsPromise]);
  const sortByCookie = cookieStore.get(SORT_BY_COOKIE_NAME)?.value;

  const periodParam = typeof searchParams.period === "string" ? searchParams.period : null;
  const pageParam = typeof searchParams.page === "string" ? Math.max(1, Number(searchParams.page) || 1) : 1;
  const sortByParam = typeof searchParams.sortBy === "string" ? searchParams.sortBy : null;
  const fromParam = typeof searchParams.from === "string" ? searchParams.from : null;
  const toParam = typeof searchParams.to === "string" ? searchParams.to : null;

  const sortBy: SortBy = sortByParam && (sortByParam === "tokens" || sortByParam === "cost")
    ? sortByParam
    : isValidSortBy(sortByCookie) ? sortByCookie : "tokens";

  let period: Period = periodParam && VALID_PERIODS.includes(periodParam as Period)
    ? (periodParam as Period)
    : "all";

  let customFrom: string | undefined;
  let customTo: string | undefined;
  if (period === "custom") {
    if (fromParam && DATE_REGEX.test(fromParam) && toParam && DATE_REGEX.test(toParam)) {
      customFrom = fromParam;
      customTo = toParam;
    } else {
      period = "all";
    }
  }

  const [initialData, session] = await Promise.all([
    getLeaderboardData(period, pageParam, 50, sortBy, "", customFrom, customTo).catch((error) => {
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
    <LeaderboardClient
      initialData={initialData}
      currentUser={session}
      initialSortBy={sortBy}
      initialUserRank={initialUserRank}
    />
  );
}
