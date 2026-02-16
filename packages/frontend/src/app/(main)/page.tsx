import { Suspense } from "react";
import { cookies } from "next/headers";
import { Navigation } from "@/components/layout/Navigation";
import { Footer } from "@/components/layout/Footer";
import { BlackholeHero } from "@/components/BlackholeHero";
import { LeaderboardSkeleton } from "@/components/Skeleton";
import { getLeaderboardData, getUserRank, type SortBy } from "@/lib/leaderboard/getLeaderboard";
import { getSession } from "@/lib/auth/session";
import { SORT_BY_COOKIE_NAME, isValidSortBy } from "@/lib/leaderboard/constants";
import LeaderboardClient from "./LeaderboardClient";

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
    getLeaderboardData("all", 1, 50, sortBy),
    getSession(),
  ]);

  const initialUserRank = session
    ? await getUserRank(session.username, "all", sortBy)
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
