import type { Metadata } from "next";
import { Navigation } from "@/components/layout/Navigation";
import { PublicFooter } from "@/components/layout/PublicFooter";
import { getModelLeaderboard } from "@/lib/explore";
import { TOKSCALE_X_HANDLE } from "@/lib/community";
import { homeUrl } from "@/lib/seo/urls";
import { breadcrumbJsonLd, modelsIndexUrl, serializeJsonLd } from "@/lib/seo/explore";
import { benchmarkData } from "@/lib/benchmarks";
import { benchmarkInfo, sourceNotes, type BenchmarkInfo } from "@/lib/benchmarks/attribution";
import { DASHBOARD_BENCHMARKS, parseColumnGroups, parseSortKey } from "@/lib/benchmarks/columns";
import { buildDashboardRows, DASHBOARD_MIN_USERS, DASHBOARD_TOP_ECI, ECI_SLUG } from "@/lib/benchmarks/dashboard";
import { mixShares, type MixCounts } from "@/lib/benchmarks/price";
import { getModelMixes } from "@/lib/benchmarks/usage";
import { PageHeader, PageLead, PageTitle } from "@/components/explore/ExploreUI";
import { ModelsDashboard } from "@/components/explore/ModelsDashboard";
import { CapabilityCharts, type ChartModel } from "@/components/explore/CapabilityCharts";
import { SourcesFooter } from "@/components/explore/SourcesFooter";
import { periodLabel, resolveExplorePeriod } from "@/components/explore/format";

const TITLE = "AI Models for Coding - Usage, Benchmarks and Price | Tokscale";
const DESCRIPTION =
  "Which AI models builders use, how they score on Terminal-Bench, the Epoch Capabilities Index and WebDev Arena, and what they cost at real agent token mixes. Each score keeps its source and run conditions.";

export function generateMetadata(): Metadata {
  const canonical = modelsIndexUrl();
  return {
    title: TITLE,
    description: DESCRIPTION,
    alternates: { canonical },
    openGraph: { title: TITLE, description: DESCRIPTION, type: "website", url: canonical, siteName: "Tokscale" },
    twitter: { card: "summary_large_image", site: TOKSCALE_X_HANDLE, title: TITLE, description: DESCRIPTION },
  };
}

interface PageProps {
  searchParams: Promise<{ [key: string]: string | string[] | undefined }>;
}

export default async function ModelsPage({ searchParams }: PageProps) {
  const params = await searchParams;
  const period = resolveExplorePeriod(params.period);
  const groups = parseColumnGroups(params.cols);
  const sort = parseSortKey(params.sort);

  const [ranked, mixes] = await Promise.all([
    getModelLeaderboard({ period, limit: 1000 }),
    getModelMixes(period).catch((error: unknown) => {
      console.error("[models] token mixes failed:", error);
      return null;
    }),
  ]);
  const rows = buildDashboardRows(benchmarkData, ranked.rows, mixes);

  const benchmarks: Record<string, BenchmarkInfo> = {};
  for (const slug of DASHBOARD_BENCHMARKS) {
    const resolved = benchmarkData.get(slug);
    if (resolved) benchmarks[slug] = benchmarkInfo(resolved);
  }
  const ownMixes: Record<string, MixCounts> = {};
  for (const row of rows) {
    const mix = mixes?.byModel[row.key];
    if (mix) ownMixes[row.key] = mix;
  }
  const chartModels: ChartModel[] = rows.flatMap((row) => {
    const eci = row.scores[ECI_SLUG];
    if (!eci) return [];
    return [
      {
        key: row.key,
        name: row.name,
        logoUrl: row.logoUrl,
        eci: eci.value,
        activeUsers: row.usage?.activeUsers ?? null,
        price: row.price?.community?.perMillion ?? null,
      },
    ];
  });

  const jsonLd = breadcrumbJsonLd([
    { name: "Tokscale", url: homeUrl() },
    { name: "Models", url: modelsIndexUrl() },
  ]);

  return (
    <div className="service-page-shell">
      <Navigation />
      <main className="service-main" id="main-content">
        <script type="application/ld+json" dangerouslySetInnerHTML={{ __html: serializeJsonLd(jsonLd) }} />
        <PageHeader>
          <PageTitle>Models</PageTitle>
          <PageLead>
            What builders use, how models score, and what they cost. Usage comes from public Tokscale submissions. Scores come
            from Epoch AI, LMArena and Terminal-Bench, each with its own unit, date and run conditions. Prices are list prices
            from models.dev.
          </PageLead>
        </PageHeader>
        <ModelsDashboard
          rows={rows}
          coverage={ranked.coverage}
          period={period}
          initialGroups={groups}
          initialSort={sort}
          benchmarks={benchmarks}
          communityMix={mixes?.community ?? null}
          ownMixes={ownMixes}
          minUsers={DASHBOARD_MIN_USERS}
          topEci={DASHBOARD_TOP_ECI}
        />
        <div style={{ marginTop: 40 }}>
          <CapabilityCharts
            models={chartModels}
            periodLabel={periodLabel(period)}
            cacheReadShare={mixes ? mixShares(mixes.community).cacheRead : null}
          />
        </div>
        <div style={{ marginTop: 40 }}>
          <SourcesFooter notes={sourceNotes(benchmarkData, { slugs: DASHBOARD_BENCHMARKS, includeCatalog: true })} />
        </div>
      </main>
      <PublicFooter />
    </div>
  );
}
