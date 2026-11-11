import type { Metadata } from "next";
import { Navigation } from "@/components/layout/Navigation";
import { listSupportedAgents } from "@/components/home/agents";
import { getLeaderboardData } from "@/lib/leaderboard/getLeaderboard";
import { getPublicOrigin } from "@/lib/seo/urls";
import { TrackerPageClient, type TrackerStats } from "./TrackerPageClient";

const TITLE = "Tracker: AI usage from your terminal | Tokscale";
const DESCRIPTION =
  "Run bunx tokscale@latest to see tokens, models, and estimated cost from your coding agents' local logs. Submit to get a public profile and README card.";

export const metadata: Metadata = {
  title: TITLE,
  description: DESCRIPTION,
  alternates: { canonical: `${getPublicOrigin()}/tracker` },
  openGraph: {
    title: TITLE,
    description: DESCRIPTION,
    url: `${getPublicOrigin()}/tracker`,
    siteName: "Tokscale",
    type: "website",
  },
};

export default async function TrackerPage() {
  // All-time totals, the same numbers as the leaderboard's "Ranked users" and "Tokens".
  const board = await getLeaderboardData("all", 1, 1, "tokens").catch(() => null);
  const stats: TrackerStats | null = board
    ? { rankedUsers: board.stats.uniqueUsers, tokens: board.stats.totalTokens }
    : null;

  return (
    <>
      <Navigation />
      <TrackerPageClient agents={listSupportedAgents()} stats={stats} />
    </>
  );
}
