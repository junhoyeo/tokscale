import type { Metadata } from "next";
import { notFound } from "next/navigation";
import { Navigation } from "@/components/layout/Navigation";
import { PublicFooter } from "@/components/layout/PublicFooter";
import { getModelDetail, type ExplorePeriod, type ModelDetail, type ModelRef } from "@/lib/explore";
import { modelKey } from "@/lib/explore/modelKey";
import { modelRef } from "@/lib/explore/modelMeta";
import { benchmarkData } from "@/lib/benchmarks";
import { buildModelResearch, hasModelResearch, researchLabel, researchParts } from "@/lib/benchmarks/modelPage";
import { getModelMixes } from "@/lib/benchmarks/usage";
import { TOKSCALE_X_HANDLE } from "@/lib/community";
import { homeUrl } from "@/lib/seo/urls";
import { breadcrumbJsonLd, modelUrl, modelsIndexUrl, serializeJsonLd } from "@/lib/seo/explore";
import {
  DEFAULT_EXPLORE_PERIOD,
  EXPLORE_PERIODS,
  formatCount,
  formatTokens,
  periodShortLabel,
  resolveExplorePeriod,
} from "@/components/explore/format";
import { ModelDetailView } from "@/components/explore/ModelDetailView";
import { ModelNoUsageView } from "@/components/explore/ModelNoUsageView";
import { companySlugForLab } from "@/lib/companyLinks";
import { buildExploreHref } from "../../leaderboard/views";

interface PageProps {
  params: Promise<{ key: string }>;
  searchParams: Promise<{ [key: string]: string | string[] | undefined }>;
}

/** Canonical keys are lowercase and URL-safe; anything else cannot exist. */
const MODEL_KEY_PATTERN = /^[a-z0-9][a-z0-9._:-]{0,127}$/;

function decodeKey(raw: string): string | null {
  let key: string;
  try {
    key = decodeURIComponent(raw);
  } catch {
    return null;
  }
  return MODEL_KEY_PATTERN.test(key) ? key : null;
}

interface LoadedDetail {
  detail: ModelDetail;
  period: ExplorePeriod;
  fallbackFrom: ExplorePeriod | null;
}

/**
 * Load the requested period. If the model exists but had no usage in that
 * window, show all time with a notice instead of a 404.
 */
async function loadDetail(key: string, period: ExplorePeriod): Promise<LoadedDetail | null> {
  const detail = await getModelDetail(key, period);
  if (detail) return { detail, period, fallbackFrom: null };
  if (period === "all") return null;
  const allTime = await getModelDetail(key, "all");
  return allTime ? { detail: allTime, period: "all", fallbackFrom: period } : null;
}

/**
 * A model without Tokscale usage still gets a page when Tokscale has
 * benchmark results or a models.dev entry for it. Returns the canonical key.
 */
function researchOnlyKey(key: string): string | null {
  const canonical = modelKey(key);
  return hasModelResearch(benchmarkData, canonical) ? canonical : null;
}

/** Token mixes for the price estimate. A failure shows the estimate as unavailable instead of failing the page. */
async function loadMixes(period: ExplorePeriod) {
  try {
    return await getModelMixes(period);
  } catch (error) {
    console.error("[models] token mixes unavailable", error);
    return null;
  }
}

/** Model identity for pages without usage, with the lab linked only when a company page exists. */
function researchModelRef(key: string): ModelRef {
  const ref = modelRef(key);
  return { ...ref, labSlug: companySlugForLab(ref.labSlug) };
}

export async function generateMetadata({ params }: PageProps): Promise<Metadata> {
  const { key: rawKey } = await params;
  const key = decodeKey(rawKey);
  const loaded = key ? await loadDetail(key, DEFAULT_EXPLORE_PERIOD) : null;
  if (key && !loaded) {
    const researchKey = researchOnlyKey(key);
    if (researchKey) {
      const model = modelRef(researchKey);
      const by = model.lab ? ` by ${model.lab}` : "";
      const parts = researchParts(benchmarkData, researchKey);
      const shows = [
        parts.benchmarks ? "benchmark results with run conditions and sources" : null,
        parts.catalog ? "list prices by provider and specs" : null,
      ].filter(Boolean);
      const title = `${model.name}${by} - ${researchLabel(parts).title} | Tokscale`;
      const description = `${model.name}${by}: ${shows.join(", ")}. No public Tokscale usage yet.`;
      return {
        title,
        description,
        alternates: { canonical: modelUrl(researchKey) },
        robots: { index: false, follow: true },
        openGraph: { title, description, type: "website", url: modelUrl(researchKey), siteName: "Tokscale" },
        twitter: { card: "summary_large_image", site: TOKSCALE_X_HANDLE, title, description },
      };
    }
  }
  if (!key || !loaded) {
    return { title: "Model not found | Tokscale", robots: { index: false } };
  }

  const { model, summary } = loaded.detail;
  const canonical = modelUrl(model.key);
  const by = model.lab ? ` by ${model.lab}` : "";
  const benchmarks = benchmarkData.forModel(model.key).length > 0 ? ", benchmark results" : "";
  const title = `${model.name}${by} - Usage, Active Users, Est. Cost | Tokscale`;
  const description = `${model.name}${by}: ${formatCount(summary.activeUsers)} active users and ${formatTokens(summary.tokens)} tokens in public Tokscale submissions, with daily trend, token mix, estimated cost${benchmarks}, and the coding agents that use it.`;

  return {
    title,
    description,
    alternates: { canonical },
    openGraph: { title, description, type: "website", url: canonical, siteName: "Tokscale" },
    twitter: { card: "summary_large_image", site: TOKSCALE_X_HANDLE, title, description },
  };
}

export default async function ModelPage({ params, searchParams }: PageProps) {
  const [{ key: rawKey }, query] = await Promise.all([params, searchParams]);
  const key = decodeKey(rawKey);
  if (!key) notFound();

  const requested = resolveExplorePeriod(query.period);
  // Mixes for the requested period load alongside the detail; most pages show that period.
  const requestedMixes = loadMixes(requested);
  const loaded = await loadDetail(key, requested);

  if (!loaded) {
    const researchKey = researchOnlyKey(key);
    if (!researchKey) notFound();
    const model = researchModelRef(researchKey);
    const research = buildModelResearch(benchmarkData, researchKey, { usage: null, mixes: await requestedMixes });
    const crumbs = [
      { name: "Tokscale", url: homeUrl() },
      { name: "Models", url: modelsIndexUrl() },
      { name: model.name, url: modelUrl(researchKey) },
    ];
    return (
      <div className="service-page-shell">
        <Navigation />
        <main className="service-main" id="main-content">
          <script type="application/ld+json" dangerouslySetInnerHTML={{ __html: serializeJsonLd(breadcrumbJsonLd(crumbs)) }} />
          <ModelNoUsageView
            model={model}
            research={research}
            period={requested}
            shows={researchLabel(researchParts(benchmarkData, researchKey)).phrase}
          />
        </main>
        <PublicFooter />
      </div>
    );
  }

  const { period, fallbackFrom } = loaded;
  // Link the lab only when a company page exists for it.
  const detail: ModelDetail = {
    ...loaded.detail,
    model: { ...loaded.detail.model, labSlug: companySlugForLab(loaded.detail.model.labSlug) },
  };
  const mixes = period === requested ? await requestedMixes : await loadMixes(period);
  const research = buildModelResearch(benchmarkData, detail.model.key, { usage: { agents: detail.agents }, mixes });
  const path = `/models/${encodeURIComponent(detail.model.key)}`;
  const periodOptions = EXPLORE_PERIODS.map((value) => ({
    href: buildExploreHref(path, { period: requested }, { period: value }, { period: DEFAULT_EXPLORE_PERIOD }),
    label: periodShortLabel(value),
    active: value === requested,
  }));

  const crumbs = [
    { name: "Tokscale", url: homeUrl() },
    { name: "Models", url: modelsIndexUrl() },
    { name: detail.model.name, url: modelUrl(detail.model.key) },
  ];

  return (
    <div className="service-page-shell">
      <Navigation />
      <main className="service-main" id="main-content">
        <script type="application/ld+json" dangerouslySetInnerHTML={{ __html: serializeJsonLd(breadcrumbJsonLd(crumbs)) }} />
        <ModelDetailView
          detail={detail}
          period={period}
          periodOptions={periodOptions}
          fallbackFrom={fallbackFrom}
          research={research}
        />
      </main>
      <PublicFooter />
    </div>
  );
}
