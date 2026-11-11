"use client";

/**
 * Pricing section of a model page: list prices per provider (models.dev) and
 * the price at real agent usage, an estimate at the reference list price
 * weighted by Tokscale token mixes, next to the usual 3:1 blend. Every
 * effective price is labelled as an estimate with its assumptions.
 */
import type { ReactNode } from "react";
import styled from "styled-components";
import type { ExplorePeriod } from "@/lib/explore";
import type { MixEstimate, ModelPricingView, OfferingView } from "@/lib/benchmarks/modelPage";
import { formatContext, formatIsoDay, formatPricePerMillion } from "@/lib/benchmarks/format";
import { MetricItem, MetricLabel, MetricStrip, MetricValue } from "@/components/leaderboard/RankingUI";
import { formatShare } from "./format";
import { Section, SectionNote, SectionTitle, VisuallyHidden } from "./ExploreUI";
import { periodPhrase } from "./ModelBenchmarks";

const WIDE = "@media (min-width: 900px)";
const NARROW = "@media (max-width: 899.98px)";
const COLUMNS = "minmax(0, 1fr) minmax(0, 1.4fr) 84px 84px 96px 96px 80px";

const Part = styled.div`
  display: grid;
  gap: 10px;
  min-width: 0;
`;

const SubTitle = styled.h3`
  margin: 0;
  color: var(--service-text);
  font-size: 0.9375rem;
  font-weight: 600;
`;

const EstimateTag = styled.span`
  margin-left: 8px;
  color: var(--service-text-muted);
  font-size: 0.8125rem;
  font-weight: 400;
`;

const Strip = styled(MetricStrip)`
  max-width: 100%;
`;

const Item = styled(MetricItem)`
  @media (min-width: 720px) {
    min-width: 180px;
  }
`;

const Sub = styled.dd`
  margin: 2px 0 0;
  color: var(--service-text-muted);
  font-size: 0.75rem;
  line-height: 1.4;
  white-space: normal;

  @media (max-width: 640px) {
    font-size: 0.8125rem;
  }
`;

const Prose = styled.p`
  max-width: 80ch;
  margin: 0;
  color: var(--service-text-muted);
  font-size: 0.875rem;
  line-height: 1.55;

  strong {
    color: var(--service-text);
    font-weight: 600;
  }

  @media (max-width: 640px) {
    font-size: 1rem;
  }
`;

const AssumptionsTitle = styled.p`
  margin: 4px 0 0;
  color: var(--service-text);
  font-size: 0.8125rem;
  font-weight: 600;

  @media (max-width: 640px) {
    font-size: 0.875rem;
  }
`;

const Assumptions = styled.ul`
  display: grid;
  max-width: 80ch;
  gap: 2px;
  margin: 0;
  padding-left: 18px;
  color: var(--service-text-muted);
  font-size: 0.8125rem;
  line-height: 1.55;
  list-style: disc;

  li::marker {
    color: var(--service-border-strong);
  }

  @media (max-width: 640px) {
    font-size: 0.875rem;
  }
`;

const ColumnHead = styled.div`
  display: none;

  ${WIDE} {
    display: grid;
    grid-template-columns: ${COLUMNS};
    gap: 16px;
    padding: 8px 0;
    border-bottom: 1px solid var(--service-border);
    color: var(--service-text-muted);
    font-size: 0.75rem;
    font-weight: 500;

    > span:nth-child(n + 3) {
      text-align: right;
    }
  }
`;

const Offerings = styled.ul`
  margin: 0;
  padding: 0;
  border-top: 1px solid var(--service-border);
  list-style: none;

  ${WIDE} {
    border-top: 0;
  }
`;

const OfferingRow = styled.li`
  display: grid;
  grid-template-columns: minmax(0, 1fr);
  gap: 4px 16px;
  align-items: baseline;
  padding: 10px 0;
  border-bottom: 1px solid var(--service-border);
  font-size: 0.875rem;
  font-variant-numeric: tabular-nums;

  ${WIDE} {
    grid-template-columns: ${COLUMNS};
  }
`;

const Provider = styled.span`
  display: flex;
  min-width: 0;
  flex-wrap: wrap;
  align-items: center;
  gap: 4px 8px;
  color: var(--service-text);
  font-weight: 600;
`;

const ReferenceTag = styled.span`
  display: inline-flex;
  min-height: 18px;
  align-items: center;
  padding: 0 6px;
  border: 1px solid var(--service-border-strong);
  border-radius: 999px;
  color: var(--service-text-muted);
  font-size: 0.6875rem;
  font-weight: 500;
`;

const ModelId = styled.code`
  min-width: 0;
  color: var(--service-text-muted);
  font-size: 0.8125rem;
  overflow-wrap: anywhere;
`;

/* Wide: price cells of the row grid. Narrow: one wrapping line of labelled prices. */
const Prices = styled.div`
  display: contents;

  ${NARROW} {
    display: flex;
    flex-wrap: wrap;
    gap: 2px 0;
    color: var(--service-text-muted);

    > *:not(:last-child)::after {
      content: "·";
      margin: 0 8px;
      color: var(--service-border-strong);
    }
  }
`;

const PriceCell = styled.span`
  color: var(--service-text);
  white-space: nowrap;

  ${WIDE} {
    text-align: right;
  }
`;

const NarrowLabel = styled.span`
  color: var(--service-text-muted);

  ${WIDE} {
    display: none;
  }
`;

const Empty = styled.p`
  margin: 0;
  color: var(--service-text-muted);
  font-size: 0.875rem;
`;

const MoreProviders = styled.details`
  > summary {
    display: inline-flex;
    min-height: 36px;
    align-items: center;
    color: var(--service-accent-hover);
    font-size: 0.8125rem;
    cursor: pointer;

    @media (max-width: 640px), (pointer: coarse) {
      min-height: 44px;
      font-size: 0.875rem;
    }
  }

  > summary:focus-visible {
    border-radius: 4px;
    outline: 2px solid var(--service-focus);
    outline-offset: 2px;
  }

  &[open] .when-closed,
  &:not([open]) .when-open {
    display: none;
  }
`;

/** Providers listed before the rest fold into "Show N more providers". */
const PROVIDERS_SHOWN = 4;

interface OfferingCell {
  label: string;
  /** Visible value; null when models.dev does not list it. */
  text: string | null;
  /** What a screen reader hears after the value: "input per 1M tokens". */
  spoken: string;
}

function offeringCells(offering: OfferingView): OfferingCell[] {
  const price = (label: string, value: number | null): OfferingCell => ({
    label,
    text: value === null ? null : formatPricePerMillion(value),
    spoken: `${label.toLowerCase()} per 1M tokens`,
  });
  return [
    price("Input", offering.input),
    price("Output", offering.output),
    price("Cache read", offering.cacheRead),
    price("Cache write", offering.cacheWrite),
    { label: "Context", text: offering.context === null ? null : formatContext(offering.context), spoken: "tokens of context" },
  ];
}

function OfferingLine({ offering }: { offering: OfferingView }) {
  return (
    <OfferingRow>
      <Provider>
        {offering.providerName}
        {offering.reference ? <ReferenceTag>Reference</ReferenceTag> : null}
      </Provider>
      <ModelId>{offering.modelId}</ModelId>
      <Prices>
        {offeringCells(offering).map((cell) => (
          <PriceCell key={cell.label}>
            <NarrowLabel aria-hidden="true">{cell.label} </NarrowLabel>
            {cell.text === null ? (
              <>
                <span aria-hidden="true">—</span>
                <VisuallyHidden>{cell.label} not listed</VisuallyHidden>
              </>
            ) : (
              <>
                {cell.text}
                <VisuallyHidden> {cell.spoken}</VisuallyHidden>
              </>
            )}
          </PriceCell>
        ))}
      </Prices>
    </OfferingRow>
  );
}

function estimateValue(estimate: MixEstimate): string {
  return estimate.status === "ok" ? formatPricePerMillion(estimate.perMillion) : "—";
}

function estimateSub(estimate: MixEstimate): ReactNode {
  switch (estimate.status) {
    case "ok":
      return (
        <>
          Est. per 1M tokens
          <br />
          {formatShare(estimate.cacheReadShare)} cache reads
        </>
      );
    case "no-data":
      return "Unavailable: no Tokscale usage data";
    case "no-usage":
      return "Unavailable: no tokens in this period";
    case "no-price":
      return "Unavailable: no list price for this mix";
  }
}

/** Why neither estimate is shown. Without usage data or tokens the community estimate says it; otherwise a price is missing. */
function noEstimateText(community: MixEstimate): string {
  if (community.status === "no-data") return "The estimate needs Tokscale usage data, which is not available here.";
  if (community.status === "no-usage") return "There is no estimate: public Tokscale submissions have no tokens in this period.";
  return "There is no estimate: the reference provider does not list a price that the token mix needs.";
}

/** "input $4, output $20, cache read $0.20" (unlisted prices left out). */
function listPriceSentence(offering: OfferingView): string {
  const parts: string[] = [];
  if (offering.input !== null) parts.push(`input ${formatPricePerMillion(offering.input)}`);
  if (offering.output !== null) parts.push(`output ${formatPricePerMillion(offering.output)}`);
  if (offering.cacheRead !== null) parts.push(`cache read ${formatPricePerMillion(offering.cacheRead)}`);
  if (offering.cacheWrite !== null) parts.push(`cache write ${formatPricePerMillion(offering.cacheWrite)}`);
  return parts.join(", ");
}

export function ModelPricing({
  pricing,
  modelName,
  period,
  retrievedAt,
}: {
  pricing: ModelPricingView | null;
  modelName: string;
  /** Period of the token mixes. */
  period: ExplorePeriod;
  /** When the models.dev catalog was retrieved (ISO). */
  retrievedAt: string | null;
}) {
  const retrieved = retrievedAt ? `, retrieved ${formatIsoDay(retrievedAt.slice(0, 10))}` : "";
  if (!pricing) {
    return (
      <Section aria-labelledby="pricing-title">
        <SectionTitle id="pricing-title">Pricing</SectionTitle>
        <Empty>models.dev does not list {modelName}, so Tokscale has no list price or price estimate for it.</Empty>
      </Section>
    );
  }

  const { reference, community, own, blend } = pricing;
  const anyEstimate = community.status === "ok" || own?.status === "ok";
  const referenceOffering = pricing.offerings.find((offering) => offering.reference) ?? null;
  const when = periodPhrase(period);

  return (
    <Section aria-labelledby="pricing-title">
      <div>
        <SectionTitle id="pricing-title">Pricing</SectionTitle>
        <SectionNote>
          List prices from models.dev in US dollars per 1M tokens{retrieved}.{" "}
          {reference
            ? reference.isLab
              ? `${reference.providerName} is the lab's own API and the reference price for the estimates below.`
              : `${reference.providerName} is the reference price for the estimates below. It is the first provider listed; models.dev lists no price from the lab's own API.`
            : "No provider lists a price for this model."}
        </SectionNote>
      </div>

      <Part>
        <SubTitle>
          Price at real agent usage<EstimateTag>Estimate at list prices</EstimateTag>
        </SubTitle>
        <Prose>
          What 1M tokens cost at the reference list price when they split into input, output and cache the way Tokscale
          users&apos; agents used them. It is not what anyone paid.
        </Prose>
        <Strip>
          {own ? (
            <Item>
              <MetricLabel>At this model&apos;s mix</MetricLabel>
              <MetricValue $accent={own.status === "ok"}>{estimateValue(own)}</MetricValue>
              <Sub>{estimateSub(own)}</Sub>
            </Item>
          ) : null}
          <Item>
            <MetricLabel>At the community mix</MetricLabel>
            <MetricValue>{estimateValue(community)}</MetricValue>
            <Sub>{estimateSub(community)}</Sub>
          </Item>
          <Item>
            <MetricLabel>3:1 input/output blend</MetricLabel>
            <MetricValue>{blend === null ? "—" : formatPricePerMillion(blend)}</MetricValue>
            <Sub>
              {blend === null ? (
                "Unavailable: no input or output price"
              ) : (
                <>
                  Per 1M tokens at list prices
                  <br />
                  No cache
                </>
              )}
            </Sub>
          </Item>
        </Strip>
        {anyEstimate ? (
          <>
            <Prose>
              {own?.status === "ok" ? (
                <>
                  Cache reads were <strong>{formatShare(own.cacheReadShare)}</strong> of {modelName} tokens {when}
                  {community.status === "ok" ? ` and ${formatShare(community.cacheReadShare)} of all tokens` : ""}.
                </>
              ) : community.status === "ok" ? (
                <>
                  Cache reads were <strong>{formatShare(community.cacheReadShare)}</strong> of all tokens in public Tokscale
                  submissions {when}.
                </>
              ) : null}{" "}
              The 3:1 blend, (3 × input + output) ÷ 4, is the usual comparison and ignores cache.
            </Prose>
            <AssumptionsTitle id="price-assumptions">Assumptions</AssumptionsTitle>
            <Assumptions aria-labelledby="price-assumptions">
              {referenceOffering ? (
                <li>
                  {referenceOffering.providerName} list prices per 1M tokens: {listPriceSentence(referenceOffering)}.
                </li>
              ) : null}
              {own ? <li>This model&apos;s mix is its own input, output, cache and reasoning tokens {when}.</li> : null}
              <li>The community mix is the tokens of every identified model {when}.</li>
              {pricing.assumptions.map((line) => (
                <li key={line}>{line}</li>
              ))}
            </Assumptions>
          </>
        ) : (
          <Prose>{noEstimateText(community)}</Prose>
        )}
      </Part>

      <Part>
        <SubTitle>List prices by provider</SubTitle>
        {pricing.offerings.length > 0 ? (
          <div>
            <ColumnHead aria-hidden="true">
              <span>Provider</span>
              <span>Model id</span>
              <span>Input</span>
              <span>Output</span>
              <span>Cache read</span>
              <span>Cache write</span>
              <span>Context</span>
            </ColumnHead>
            <Offerings>
              {(pricing.offerings.length > PROVIDERS_SHOWN + 1 ? pricing.offerings.slice(0, PROVIDERS_SHOWN) : pricing.offerings).map(
                (offering) => (
                  <OfferingLine key={offering.provider} offering={offering} />
                ),
              )}
            </Offerings>
            {pricing.offerings.length > PROVIDERS_SHOWN + 1 ? (
              <MoreProviders>
                <summary>
                  <span className="when-closed">Show {pricing.offerings.length - PROVIDERS_SHOWN} more providers</span>
                  <span className="when-open">Show fewer providers</span>
                </summary>
                <Offerings>
                  {pricing.offerings.slice(PROVIDERS_SHOWN).map((offering) => (
                    <OfferingLine key={offering.provider} offering={offering} />
                  ))}
                </Offerings>
              </MoreProviders>
            ) : null}
          </div>
        ) : (
          <Empty>models.dev lists {modelName} without a provider price.</Empty>
        )}
      </Part>
    </Section>
  );
}
