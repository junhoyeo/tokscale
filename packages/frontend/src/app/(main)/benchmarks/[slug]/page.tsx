import type { Metadata } from "next";
import { notFound } from "next/navigation";
import { Navigation } from "@/components/layout/Navigation";
import { PublicFooter } from "@/components/layout/PublicFooter";
import { BenchmarkDetailView } from "@/components/explore/BenchmarkDetailView";
import { benchmarkData, benchmarksUrl, benchmarkUrl, type ResolvedBenchmark } from "@/lib/benchmarks";
import { sourceNotes } from "@/lib/benchmarks/attribution";
import {
  benchmarkMetaDescription,
  benchmarkPageInfo,
  benchmarkPageSlugs,
  buildModelContext,
  buildResultRows,
  buildTimeline,
  hasBenchmarkPage,
  mappingSummary,
  resultColumns,
} from "@/lib/benchmarks/detail";
import { TOKSCALE_X_HANDLE } from "@/lib/community";
import { getBenchmarkModelUsage } from "@/lib/benchmarks/usage";
import { modelRef } from "@/lib/explore/modelMeta";
import { breadcrumbJsonLd, serializeJsonLd } from "@/lib/seo/explore";
import { homeUrl } from "@/lib/seo/urls";

interface PageProps {
  params: Promise<{ slug: string }>;
}

/** The usage column follows the explore rollups, which refresh every 10 minutes. */
export const revalidate = 600;

/** Benchmarks with republished results. Link-only benchmarks have no page. */
export function generateStaticParams() {
  return benchmarkPageSlugs(benchmarkData).map((slug) => ({ slug }));
}

function load(slug: string): ResolvedBenchmark | null {
  const resolved = benchmarkData.get(slug);
  return resolved && hasBenchmarkPage(resolved) ? resolved : null;
}

export async function generateMetadata({ params }: PageProps): Promise<Metadata> {
  const { slug } = await params;
  const resolved = load(slug);
  if (!resolved) return { title: "Benchmark not found | Tokscale", robots: { index: false } };
  const title = `${resolved.definition.name} results by model | Tokscale`;
  const description = benchmarkMetaDescription(resolved);
  const canonical = benchmarkUrl(resolved.definition.slug);
  return {
    title,
    description,
    alternates: { canonical },
    openGraph: { title, description, type: "website", url: canonical, siteName: "Tokscale" },
    twitter: { card: "summary_large_image", site: TOKSCALE_X_HANDLE, title, description },
  };
}

export default async function BenchmarkPage({ params }: PageProps) {
  const { slug } = await params;
  const resolved = load(slug);
  if (!resolved) notFound();
  const key = resolved.definition.slug;

  // Usage is optional on this page: without it the table and chart still render.
  const recent = await getBenchmarkModelUsage().catch(() => null);
  const ctx = buildModelContext(benchmarkData, recent
    ? Object.entries(recent.counts).map(([key, activeUsers]) => ({ ...modelRef(key), activeUsers }))
    : null);
  const rows = buildResultRows(resolved, ctx);
  const timeline = buildTimeline(resolved, benchmarkData.best(key), ctx);

  const crumbs = [
    { name: "Tokscale", url: homeUrl() },
    { name: "Benchmarks", url: benchmarksUrl() },
    { name: resolved.definition.name, url: benchmarkUrl(key) },
  ];

  return (
    <div className="service-page-shell">
      <Navigation />
      <main className="service-main" id="main-content">
        <script type="application/ld+json" dangerouslySetInnerHTML={{ __html: serializeJsonLd(breadcrumbJsonLd(crumbs)) }} />
        <BenchmarkDetailView
          info={benchmarkPageInfo(benchmarkData, resolved)}
          rows={rows}
          columns={resultColumns(rows)}
          timeline={timeline}
          mapping={mappingSummary(resolved.results)}
          usageWindow={recent ? { start: recent.start, end: recent.end } : null}
          sources={sourceNotes(benchmarkData, { slugs: [key], includeCatalog: timeline.catalogDates > 0 })}
        />
      </main>
      <PublicFooter />
    </div>
  );
}
