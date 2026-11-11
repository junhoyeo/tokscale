import type { Metadata } from "next";
import { Navigation } from "@/components/layout/Navigation";
import { HomePage } from "@/components/home/HomePage";
import { getCommunitySnapshot, getHumansLeaderboard, getModelLeaderboard } from "@/lib/explore";
import { getHomeCapital } from "@/lib/capital/ledger/home";
import { homeUrl } from "@/lib/seo/urls";

// Declared per-page rather than on the root layout: `alternates.canonical` is
// inherited by every nested route, so putting it in app/layout.tsx would point
// every page on the site at the home page.
export const metadata: Metadata = {
  alternates: {
    canonical: homeUrl(),
  },
};

export default async function Page() {
  const [snapshot, humans, models, capital] = await Promise.all([
    getCommunitySnapshot("30d").catch(() => null),
    getHumansLeaderboard({ period: "30d", limit: 1 }).catch(() => null),
    getModelLeaderboard({ period: "30d", sort: "users", limit: 2 }).catch(() => null),
    getHomeCapital().catch(() => null),
  ]);

  return (
    <>
      <Navigation />
      <HomePage
        snapshot={snapshot}
        featuredHuman={humans?.rows[0] ?? null}
        humansCoverage={humans?.coverage ?? null}
        models={models?.rows ?? []}
        modelsCoverage={models?.coverage ?? null}
        capital={capital}
      />
    </>
  );
}
