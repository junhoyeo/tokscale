import type { Metadata } from "next";
import { Navigation } from "@/components/layout/Navigation";
import { PublicFooter } from "@/components/layout/PublicFooter";
import { getModelLeaderboard } from "@/lib/explore";
import { modelsAllPath, modelsAllUrl } from "@/lib/benchmarks/urls";
import { TOKSCALE_X_HANDLE } from "@/lib/community";
import { homeUrl } from "@/lib/seo/urls";
import { breadcrumbJsonLd, modelsIndexUrl, serializeJsonLd } from "@/lib/seo/explore";
import { EXPLORE_PERIODS, EXPLORE_SORTS, periodShortLabel, resolveExplorePeriod, resolveExploreSort, sortLabel } from "@/components/explore/format";
import { toCatalogModel } from "@/components/explore/catalog";
import { ModelCatalog } from "@/components/explore/ModelCatalog";
import { ModelBreadcrumb } from "@/components/explore/ModelHeader";
import { companySlugForLab } from "@/lib/companyLinks";
import { PageHeader, PageLead, PageTitle } from "@/components/explore/ExploreUI";
import { buildExploreHref } from "../../leaderboard/views";

/** The catalog lists every model with data, so it defaults to all time. */
const CATALOG_DEFAULT_PERIOD = "all" as const;
const CATALOG_LIMIT = 1000;

const TITLE = "All AI Models Seen in Tokscale Submissions - Active Users, Tokens, Est. Cost | Tokscale";
const DESCRIPTION =
  "All AI models seen in public Tokscale submissions, grouped by lab, with active users, adoption, tokens and estimated cost per model.";

export function generateMetadata(): Metadata {
  const canonical = modelsAllUrl();
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

function text(value: string | string[] | undefined): string {
  return (Array.isArray(value) ? value[0] : value)?.slice(0, 80) ?? "";
}

export default async function AllModelsPage({ searchParams }: PageProps) {
  const params = await searchParams;
  const period = resolveExplorePeriod(params.period, CATALOG_DEFAULT_PERIOD);
  const sort = resolveExploreSort(params.sort);
  const ranked = await getModelLeaderboard({ period, sort, limit: CATALOG_LIMIT });
  const companySlugs: Record<string, string> = {};
  for (const row of ranked.rows) {
    const company = row.labSlug ? companySlugForLab(row.labSlug) : null;
    if (row.labSlug && company) companySlugs[row.labSlug] = company;
  }

  const path = modelsAllPath();
  const current = { period, sort };
  const defaults = { period: CATALOG_DEFAULT_PERIOD, sort: "users" };
  const periodOptions = EXPLORE_PERIODS.map((value) => ({
    href: buildExploreHref(path, current, { period: value }, defaults),
    label: periodShortLabel(value),
    active: value === period,
  }));
  const sortOptions = EXPLORE_SORTS.map((value) => ({
    href: buildExploreHref(path, current, { sort: value }, defaults),
    label: sortLabel(value),
    active: value === sort,
  }));

  const jsonLd = breadcrumbJsonLd([
    { name: "Tokscale", url: homeUrl() },
    { name: "Models", url: modelsIndexUrl() },
    { name: "All models", url: modelsAllUrl() },
  ]);

  return (
    <div className="service-page-shell">
      <Navigation />
      <main className="service-main" id="main-content">
        <script type="application/ld+json" dangerouslySetInnerHTML={{ __html: serializeJsonLd(jsonLd) }} />
        <ModelBreadcrumb items={[{ label: "Models", href: "/models" }, { label: "All models" }]} />
        <PageHeader>
          <PageTitle>All models</PageTitle>
          <PageLead>
            Every model seen in public Tokscale submissions, grouped by lab. Each lab lists its top models first; search
            or pick a lab to see the rest. Open a model for its trend, token mix, agents, benchmarks and prices.
          </PageLead>
        </PageHeader>
        <ModelCatalog
          rows={ranked.rows.map(toCatalogModel)}
          coverage={ranked.coverage}
          sort={sort}
          periodOptions={periodOptions}
          sortOptions={sortOptions}
          initialQuery={text(params.q)}
          initialLab={text(params.lab) || "all"}
          companySlugs={companySlugs}
        />
      </main>
      <PublicFooter />
    </div>
  );
}
