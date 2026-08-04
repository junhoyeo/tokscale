import type { Metadata } from "next";
import { Navigation } from "@/components/layout/Navigation";
import { LandingPage } from "@/components/landing/LandingPage";
import { getStargazersCount } from "@/lib/github";
import { getLeaderboardData, type LeaderboardData } from "@/lib/leaderboard/getLeaderboard";
import { homeUrl } from "@/lib/seo/urls";

// This page queries the leaderboard. Keep it runtime-rendered so the
// self-hosted image can build without a database while static routes remain
// pre-rendered for the hosted site.
export const dynamic = "force-dynamic";

// Declared per-page rather than on the root layout: `alternates.canonical` is
// inherited by every nested route, so putting it in app/layout.tsx would point
// every page on the site at the home page.
export const metadata: Metadata = {
  alternates: {
    canonical: homeUrl(),
  },
};

function createEmptyLeaderboardData(sortBy: "tokens" | "cost"): LeaderboardData {
  return {
    users: [],
    pagination: {
      page: 1,
      limit: 5,
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

export default async function HomePage() {
  const [stargazersCount, topUsersByCost, topUsersByTokens] = await Promise.all([
    getStargazersCount("junhoyeo/tokscale"),
    getLeaderboardData("all", 1, 5, "cost").catch(() => createEmptyLeaderboardData("cost")),
    getLeaderboardData("all", 1, 5, "tokens").catch(() => createEmptyLeaderboardData("tokens")),
  ]);

  return (
    <>
      <Navigation />
      <LandingPage
        stargazersCount={stargazersCount}
        topUsersByCost={topUsersByCost.users}
        topUsersByTokens={topUsersByTokens.users}
      />
    </>
  );
}
