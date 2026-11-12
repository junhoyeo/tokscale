import type { Metadata } from "next";
import { Navigation } from "@/components/layout/Navigation";
import { HomePage } from "@/components/home/HomePage";
import { getCommunitySnapshot, getFeaturedHuman, getModelLeaderboard } from "@/lib/explore";
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

/** The home page features this account. It is pinned by hand: change the username to feature someone else. */
const FEATURED_HUMAN_USERNAME = "Dicklesworthstone";

export default async function Page() {
  const [snapshot, featured, models, capital] = await Promise.all([
    getCommunitySnapshot("30d").catch(() => null),
    getFeaturedHuman(FEATURED_HUMAN_USERNAME, "30d").catch(() => null),
    getModelLeaderboard({ period: "30d", sort: "users", limit: 2 }).catch(() => null),
    getHomeCapital().catch(() => null),
  ]);

  return (
    <>
      <Navigation />
      <HomePage
        snapshot={snapshot}
        featuredHuman={featured?.human ?? null}
        humansCoverage={featured?.coverage ?? null}
        models={models?.rows ?? []}
        modelsCoverage={models?.coverage ?? null}
        capital={capital}
      />
    </>
  );
}
