import { Navigation } from "@/components/layout/Navigation";
import { LandingPage } from "@/components/landing/LandingPage";
import { getStargazersCount } from "@/lib/github";
import { getLeaderboardData, type LeaderboardData } from "@/lib/leaderboard/getLeaderboard";

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
      totalSubmissions: null,
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
