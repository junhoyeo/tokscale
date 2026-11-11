import type { Metadata } from "next";
import { Navigation } from "@/components/layout/Navigation";
import { PublicFooter } from "@/components/layout/PublicFooter";
import { BenchmarkIndex } from "@/components/explore/BenchmarkIndex";
import { PageHeader, PageLead, PageTitle } from "@/components/explore/ExploreUI";
import { benchmarkData, benchmarksUrl } from "@/lib/benchmarks";
import { sourceNotes } from "@/lib/benchmarks/attribution";
import { benchmarkIndexSummary, buildBenchmarkIndex } from "@/lib/benchmarks/detail";
import { TOKSCALE_X_HANDLE } from "@/lib/community";
import { breadcrumbJsonLd, serializeJsonLd } from "@/lib/seo/explore";
import { homeUrl } from "@/lib/seo/urls";

const TITLE = "AI Coding Benchmarks - Terminal-Bench, ECI, WebDev Arena | Tokscale";
const DESCRIPTION =
  "The AI benchmarks Tokscale tracks, grouped by what they test: agentic coding, web development, agents, reasoning and knowledge. Each keeps its own unit, source and license. No combined score.";

export function generateMetadata(): Metadata {
  const canonical = benchmarksUrl();
  return {
    title: TITLE,
    description: DESCRIPTION,
    alternates: { canonical },
    openGraph: { title: TITLE, description: DESCRIPTION, type: "website", url: canonical, siteName: "Tokscale" },
    twitter: { card: "summary_large_image", site: TOKSCALE_X_HANDLE, title: TITLE, description: DESCRIPTION },
  };
}

export default function BenchmarksPage() {
  const jsonLd = breadcrumbJsonLd([
    { name: "Tokscale", url: homeUrl() },
    { name: "Benchmarks", url: benchmarksUrl() },
  ]);

  return (
    <div className="service-page-shell">
      <Navigation />
      <main className="service-main" id="main-content">
        <script type="application/ld+json" dangerouslySetInnerHTML={{ __html: serializeJsonLd(jsonLd) }} />
        <PageHeader>
          <PageTitle>Benchmarks</PageTitle>
          <PageLead>
            The benchmarks Tokscale tracks, grouped by what they test. Each one keeps its own unit, source and license.
            Tokscale does not combine them into one score.
          </PageLead>
        </PageHeader>
        <BenchmarkIndex
          groups={buildBenchmarkIndex(benchmarkData)}
          summary={benchmarkIndexSummary(benchmarkData)}
          sources={sourceNotes(benchmarkData)}
        />
      </main>
      <PublicFooter />
    </div>
  );
}
