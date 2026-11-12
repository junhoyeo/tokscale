import type { Metadata } from "next";
import { Navigation } from "@/components/layout/Navigation";
import { HomePage } from "@/components/home/HomePage";
import { getCommunitySnapshot, getFeaturedHuman, getModelLeaderboard } from "@/lib/explore";
import { getHomeCapital } from "@/lib/capital/ledger/home";
import { isPrerenderedDeployment } from "@/lib/prerender";
import { homeUrl } from "@/lib/seo/urls";

// Declared per-page rather than on the root layout: `alternates.canonical` is
// inherited by every nested route, so putting it in app/layout.tsx would point
// every page on the site at the home page.
export const metadata: Metadata = {
  alternates: {
    canonical: homeUrl(),
  },
};

/** Where pages are prerendered, rebuild this one in the background as often as the explore rollups refresh. */
export const revalidate = 600;

/** The home page features this account. It is pinned by hand: change the username to feature someone else. */
const FEATURED_HUMAN_USERNAME = "Dicklesworthstone";

/**
 * A per-request render shows the sections it could load. A prerendered page is
 * published as a whole and kept for ten minutes, so there a failed load fails
 * the rebuild and the last complete page stays up.
 */
function section<T>(load: Promise<T>): Promise<T | null> {
  return isPrerenderedDeployment() ? load : load.catch(() => null);
}

export default async function Page() {
  const [snapshot, featured, models, capital] = await Promise.all([
    section(getCommunitySnapshot("30d")),
    section(getFeaturedHuman(FEATURED_HUMAN_USERNAME, "30d")),
    section(getModelLeaderboard({ period: "30d", sort: "users", limit: 2 })),
    section(getHomeCapital()),
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
