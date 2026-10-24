"use client";

/**
 * Body of /models/[key] for a model that has benchmark results or a
 * models.dev entry but no public Tokscale usage yet: identity, a clear note,
 * then Benchmarks, Pricing (list prices, estimate at the community mix only),
 * Specs and Sources. The page is not indexed.
 */
import styled from "styled-components";
import type { ExplorePeriod, ModelRef } from "@/lib/explore";
import type { ModelResearchView } from "@/lib/benchmarks/modelPage";
import { modelsAllPath } from "@/lib/benchmarks/urls";
import { InlineLink } from "./ExploreUI";
import { ModelBenchmarks } from "./ModelBenchmarks";
import { ModelBreadcrumb, ModelIdentity, modelCrumbs } from "./ModelHeader";
import { ModelPricing } from "./ModelPricing";
import { ModelSpecs } from "./ModelSpecs";
import { SourcesFooter } from "./SourcesFooter";

const Notice = styled.div`
  display: grid;
  max-width: 72ch;
  gap: 4px;
  margin: 0 0 28px;
  padding: 12px 14px;
  border: 1px solid var(--service-border-strong);
  border-radius: 8px;
  color: var(--service-text-muted);
  font-size: 0.875rem;
  line-height: 1.55;

  > p {
    margin: 0;
  }

  > p:first-child {
    color: var(--service-text);
    font-weight: 600;
  }

  @media (max-width: 640px) {
    font-size: 1rem;
  }
`;

const Stack = styled.div`
  display: grid;
  gap: 32px;
  min-width: 0;
`;

export function ModelNoUsageView({
  model,
  research,
  period,
  shows,
}: {
  /** `labSlug` must already be a company page slug, or null. */
  model: ModelRef;
  research: ModelResearchView;
  /** Period of the community token mix used for the price estimate. */
  period: ExplorePeriod;
  /** What the page has, e.g. "benchmark results, list prices and specs" (researchLabel). */
  shows: string;
}) {
  return (
    <>
      <ModelBreadcrumb items={modelCrumbs(model)} />
      <ModelIdentity model={model} />

      <Notice role="note" aria-label="No public Tokscale usage yet">
        <p>No public Tokscale usage yet</p>
        <p>
          No public Tokscale submission includes {model.name} yet, so this page has no active users, tokens or agent
          shares. The {shows} below come from public sources.{" "}
          <InlineLink href={modelsAllPath()}>See models with usage</InlineLink>
        </p>
      </Notice>

      <Stack>
        <ModelBenchmarks benchmarks={research.benchmarks} modelName={model.name} period={null} />
        <ModelPricing
          pricing={research.pricing}
          modelName={model.name}
          period={period}
          retrievedAt={research.specs?.retrievedAt ?? null}
        />
        <ModelSpecs specs={research.specs} modelName={model.name} />
        <SourcesFooter notes={research.sources} />
      </Stack>
    </>
  );
}
