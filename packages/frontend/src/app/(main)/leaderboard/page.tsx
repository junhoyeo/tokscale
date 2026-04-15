import { Suspense } from "react";
import { cookies } from "next/headers";
import { Navigation } from "@/components/layout/Navigation";
import { Footer } from "@/components/layout/Footer";
import { BlackholeHero } from "@/components/BlackholeHero";
import { LeaderboardSkeleton } from "@/components/Skeleton";
import { getLeaderboardData, getUserRank } from "@/lib/leaderboard/getLeaderboard";
import type { LeaderboardData, SortBy } from "@/lib/leaderboard/types";
import { getSession } from "@/lib/auth/session";
import { SORT_BY_COOKIE_NAME, isValidSortBy } from "@/lib/leaderboard/constants";

function isMissingDatabaseUrl(error: unknown): boolean {
  return error instanceof Error && error.message === "DATABASE_URL environment variable is not set";
}
import LeaderboardClient from "./LeaderboardClient";

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

export default function LeaderboardPage() {
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
          <LeaderboardWithPreferences />
        </Suspense>
      </main>

      <Footer />
    </div>
  );
}

async function LeaderboardWithPreferences() {
  const cookieStore = await cookies();
  const sortByCookie = cookieStore.get(SORT_BY_COOKIE_NAME)?.value;
  const sortBy: SortBy = isValidSortBy(sortByCookie) ? sortByCookie : "tokens";

  const [initialData, session] = await Promise.all([
    getLeaderboardData("all", 1, 50, sortBy).catch((error) => {
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
    ? await getUserRank(session.username, "all", sortBy).catch((error) => {
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
