"use client";

/**
 * Body of /models/[key]: identity, period, summary facts, trend, token mix,
 * agents, benchmarks, pricing, specs, top public profiles, related models,
 * coverage and sources.
 */
import Link from "next/link";
import styled from "styled-components";
import type { ExplorePeriod, ModelDetail, TokenMix } from "@/lib/explore";
import type { ModelResearchView } from "@/lib/benchmarks/modelPage";
import { modelsAllPath } from "@/lib/benchmarks/urls";
import { MetricItem, MetricLabel, MetricStrip, MetricValue } from "@/components/leaderboard/RankingUI";
import { modelPath, profilePath } from "@/lib/seo/explore";
import {
  describeUsersChange,
  formatCount,
  formatDay,
  estimatedCostPhrase,
  formatEstimatedCost,
  formatShare,
  formatTokens,
  periodLabel,
  previousWindowLabel,
  tokenMixParts,
  type TokenMixKey,
} from "./format";
import { UNKNOWN_LAB_NAME } from "./catalog";
import {
  AgentMark,
  ChangeText,
  ControlBar,
  CoverageLine,
  InlineLink,
  LinkSegments,
  ModelMark,
  Section,
  SectionNote,
  SectionTitle,
  ShareMeter,
  VisuallyHidden,
  type LinkOption,
} from "./ExploreUI";
import { ModelBenchmarks } from "./ModelBenchmarks";
import { ModelBreadcrumb, ModelIdentity, modelCrumbs } from "./ModelHeader";
import { ModelPricing } from "./ModelPricing";
import { ModelSpecs } from "./ModelSpecs";
import { SourcesFooter } from "./SourcesFooter";
import { TrendChart } from "./TrendChart";

const Notice = styled.p`
  margin: 0 0 16px;
  padding: 10px 12px;
  border: 1px solid var(--service-border-strong);
  border-radius: 8px;
  color: var(--service-text);
  font-size: 0.875rem;
`;

/*
 * Six facts need about 950px on one line. Between the phone grid (two per
 * line) and that width, show three per line instead of running past the page.
 */
const Facts = styled(MetricStrip)`
  margin-bottom: 12px;

  @media (min-width: 720px) and (max-width: 999px) {
    display: grid;
    grid-template-columns: repeat(3, minmax(0, 1fr));
    max-width: none;

    && > ${MetricItem} {
      min-width: 0;
      padding: 12px 20px;
      border-left: 1px solid var(--service-border);
    }

    && > ${MetricItem}:nth-child(3n + 1) {
      padding-left: 0;
      border-left: 0;
    }

    && > ${MetricItem}:nth-child(3n) {
      padding-right: 0;
    }

    && > ${MetricItem}:nth-child(n + 4) {
      border-top: 1px solid var(--service-border) !important;
    }
  }
`;

const FactSub = styled.dd`
  min-height: 1.2em;
  margin: 2px 0 0;
  color: var(--service-text-muted);
  font-size: 0.75rem;
  white-space: nowrap;
`;

const CoverageRow = styled(CoverageLine)`
  margin-bottom: 28px;
`;

const Grid = styled.div`
  display: grid;
  grid-template-columns: minmax(0, 1fr);
  gap: 32px;

  @media (min-width: 1024px) {
    grid-template-columns: repeat(2, minmax(0, 1fr));
    gap: 32px 48px;
  }
`;

const Wide = styled(Section)`
  @media (min-width: 1024px) {
    grid-column: 1 / -1;
  }
`;

/* Full-width grid cell for sections that render their own Section. */
const FullRow = styled.div`
  display: grid;
  min-width: 0;

  @media (min-width: 1024px) {
    grid-column: 1 / -1;
  }
`;

const MixBar = styled.div`
  display: flex;
  height: 14px;
  overflow: hidden;
  gap: 2px;
  border-radius: 4px;
`;

const MixSegment = styled.span`
  display: block;
  height: 100%;
  min-width: 0;
`;

const Legend = styled.ul`
  display: grid;
  grid-template-columns: repeat(auto-fill, minmax(150px, 1fr));
  gap: 8px 16px;
  margin: 0;
  padding: 0;
  list-style: none;
`;

const LegendItem = styled.li`
  display: flex;
  align-items: center;
  gap: 8px;
  color: var(--service-text);
  font-size: 0.875rem;
  font-variant-numeric: tabular-nums;

  > span:last-child {
    margin-left: auto;
    color: var(--service-text-muted);
  }
`;

const Swatch = styled.span`
  width: 10px;
  height: 10px;
  flex: 0 0 auto;
  border-radius: 2px;
`;

const Rows = styled.ol`
  margin: 0;
  padding: 0;
  border-top: 1px solid var(--service-border);
`;

const Row = styled.li`
  position: relative;
  display: grid;
  grid-template-columns: minmax(0, 1fr) minmax(120px, 180px) 88px;
  gap: 4px 16px;
  align-items: center;
  min-height: 52px;
  padding: 8px 0;
  list-style: none;
  border-bottom: 1px solid var(--service-border);
  font-size: 0.875rem;
  font-variant-numeric: tabular-nums;

  @media (max-width: 520px) {
    grid-template-columns: minmax(0, 1fr) auto;
    min-height: 60px;
  }
`;

const RowName = styled.span`
  display: flex;
  min-width: 0;
  align-items: center;
  gap: 8px;
  overflow: hidden;
  color: var(--service-text);
  font-weight: 500;
  text-overflow: ellipsis;
  white-space: nowrap;
`;

const RowValue = styled.span`
  color: var(--service-text);
  text-align: right;
  white-space: nowrap;

  > span {
    display: block;
    color: var(--service-text-muted);
    font-size: 0.75rem;
  }
`;

const RowMeter = styled.span`
  min-width: 0;

  @media (max-width: 520px) {
    grid-column: 1 / -1;
    grid-row: 2;
  }
`;

/* The row's primary link: the whole row opens the page (row links, globals.css). */
const RowLink = styled(Link)`
  overflow: hidden;
  color: var(--service-text);
  text-decoration: none;
  text-overflow: ellipsis;

  &:hover {
    color: var(--service-accent-hover);
  }
`;

const PersonRow = styled(Row)`
  grid-template-columns: 28px 36px minmax(0, 1fr) auto auto auto;
  min-height: 60px;

  @media (max-width: 520px) {
    grid-template-columns: 24px 36px minmax(0, 1fr) auto;
  }
`;

const PersonRank = styled.span`
  color: var(--service-text-muted);
  font-size: 0.8125rem;
  font-weight: 600;
`;

const Avatar = styled.img`
  width: 36px;
  height: 36px;
  border-radius: 50%;
  object-fit: cover;
  outline: 1px solid var(--service-border);
  outline-offset: -1px;
`;

const PersonName = styled.span`
  display: grid;
  min-width: 0;

  > span {
    overflow: hidden;
    color: var(--service-text-muted);
    font-size: 0.8125rem;
    text-overflow: ellipsis;
    white-space: nowrap;
  }
`;

const HideOnNarrow = styled(RowValue)`
  @media (max-width: 520px) {
    display: none;
  }
`;

const ShowOnNarrow = styled.span`
  display: none;
  grid-column: 3 / -1;
  color: var(--service-text-muted);
  font-size: 0.8125rem;

  @media (max-width: 520px) {
    display: block;
  }
`;

const SmallLink = styled(InlineLink)`
  justify-self: start;
  font-size: 0.875rem;
`;

const Empty = styled.p`
  margin: 0;
  color: var(--service-text-muted);
  font-size: 0.875rem;
`;

const MIX_COLORS: Record<TokenMixKey, string> = {
  input: "#2f8fff",
  output: "#9cc9ff",
  cacheRead: "#6b7686",
  cacheWrite: "#3a4352",
  reasoning: "#d7dee8",
};

function TokenMixSection({ mix, tokens }: { mix: TokenMix; tokens: number }) {
  const parts = tokenMixParts(mix);
  const total = parts.reduce((sum, part) => sum + part.share, 0);
  return (
    <Section aria-labelledby="token-mix-title">
      <div>
        <SectionTitle id="token-mix-title">Token mix</SectionTitle>
        <SectionNote>
          How the {formatTokens(tokens)} tokens split. Cache reads are usually most of an agent&apos;s tokens and are priced
          far below input.
        </SectionNote>
      </div>
      {total > 0 ? (
        <>
          <MixBar aria-hidden="true">
            {parts
              .filter((part) => part.share > 0)
              .map((part) => (
                <MixSegment
                  key={part.key}
                  style={{ flexGrow: part.share, flexBasis: 0, minWidth: part.share > 0 ? 3 : 0, background: MIX_COLORS[part.key] }}
                />
              ))}
          </MixBar>
          <Legend>
            {parts.map((part) => (
              <LegendItem key={part.key}>
                <Swatch aria-hidden="true" style={{ background: MIX_COLORS[part.key] }} />
                <span>{part.label}</span>
                <span>{formatShare(part.share)}</span>
              </LegendItem>
            ))}
          </Legend>
        </>
      ) : (
        <Empty>No token breakdown for this period.</Empty>
      )}
    </Section>
  );
}

interface ModelDetailViewProps {
  detail: ModelDetail;
  period: ExplorePeriod;
  periodOptions: LinkOption[];
  /** Set when the requested period had no usage and all time is shown instead. */
  fallbackFrom: ExplorePeriod | null;
  /** Benchmarks, pricing, specs and sources for the model. Omit to show usage only. */
  research?: ModelResearchView | null;
}

/** Benchmarks, Pricing and Specs (or one note when no source knows the model). */
function ResearchSections({
  research,
  modelName,
  period,
}: {
  research: ModelResearchView;
  modelName: string;
  period: ExplorePeriod;
}) {
  if (research.benchmarks.rows.length === 0 && !research.pricing && !research.specs) {
    return (
      <Wide aria-labelledby="research-title">
        <div>
          <SectionTitle id="research-title">Benchmarks, pricing and specs</SectionTitle>
          <SectionNote>
            No benchmark result and no models.dev entry match this model key yet, so there are no scores, list prices or
            specs to show.
          </SectionNote>
        </div>
      </Wide>
    );
  }
  return (
    <>
      <FullRow>
        <ModelBenchmarks benchmarks={research.benchmarks} modelName={modelName} period={period} />
      </FullRow>
      <FullRow>
        <ModelPricing
          pricing={research.pricing}
          modelName={modelName}
          period={period}
          retrievedAt={research.specs?.retrievedAt ?? null}
        />
      </FullRow>
      <FullRow>
        <ModelSpecs specs={research.specs} modelName={modelName} />
      </FullRow>
    </>
  );
}

export function ModelDetailView({ detail, period, periodOptions, fallbackFrom, research = null }: ModelDetailViewProps) {
  const { model, summary, coverage } = detail;
  const change = describeUsersChange(summary.activeUsers, summary.previousActiveUsers, summary.previousActiveUsers === 0, period);
  const previousLabel = previousWindowLabel(period);
  const labName = model.lab ?? UNKNOWN_LAB_NAME;
  const hasResearch = research !== null && (research.benchmarks.rows.length > 0 || research.pricing !== null || research.specs !== null);

  return (
    <>
      <ModelBreadcrumb items={modelCrumbs(model)} />

      <ModelIdentity model={model}>
        {model.aliases.length > 0 ? (
          <span>
            Also reported as{" "}
            {model.aliases.map((alias, index) => (
              <span key={alias}>
                {index > 0 ? ", " : null}
                <code>{alias}</code>
              </span>
            ))}
          </span>
        ) : null}
      </ModelIdentity>

      <ControlBar>
        <LinkSegments label="Period" visibleLabel="Period" options={periodOptions} />
      </ControlBar>

      {fallbackFrom ? (
        <Notice role="status">
          No usage of {model.name} in the {periodLabel(fallbackFrom).toLowerCase()}. Showing all time instead.
        </Notice>
      ) : null}

      <Facts>
        <MetricItem>
          <MetricLabel>Rank</MetricLabel>
          <MetricValue>{summary.rank ? `#${summary.rank}` : "—"}</MetricValue>
          <FactSub>{summary.rank ? "by active users" : "not ranked"}</FactSub>
        </MetricItem>
        <MetricItem>
          <MetricLabel>Active users</MetricLabel>
          <MetricValue $accent>{formatCount(summary.activeUsers)}</MetricValue>
          <FactSub>
            {change.kind === "none" ? periodLabel(period) : <ChangeText change={change} />}
            {previousLabel && change.kind !== "none" ? <VisuallyHidden> vs {previousLabel}</VisuallyHidden> : null}
          </FactSub>
        </MetricItem>
        <MetricItem>
          <MetricLabel>Adoption</MetricLabel>
          <MetricValue>{formatShare(summary.adoption)}</MetricValue>
          <FactSub>of {formatCount(coverage.accounts)} accounts</FactSub>
        </MetricItem>
        <MetricItem>
          <MetricLabel>Tokens</MetricLabel>
          <MetricValue>{formatTokens(summary.tokens)}</MetricValue>
          <FactSub>cache included</FactSub>
        </MetricItem>
        <MetricItem>
          <MetricLabel>Est. cost</MetricLabel>
          <MetricValue>{formatEstimatedCost(summary.estimatedCost, summary.tokens)}</MetricValue>
          <FactSub>at public list prices</FactSub>
        </MetricItem>
        <MetricItem>
          <MetricLabel>First seen</MetricLabel>
          <MetricValue>{summary.firstSeen ? formatDay(summary.firstSeen) : "—"}</MetricValue>
          <FactSub>in any submission</FactSub>
        </MetricItem>
      </Facts>
      <CoverageRow coverage={coverage} />

      <Grid>
        <Wide aria-label="Daily usage">
          <TrendChart trend={detail.trend} modelName={model.name} />
        </Wide>

        <TokenMixSection mix={summary.tokenMix} tokens={summary.tokens} />

        <Section aria-labelledby="agents-title">
          <div>
            <SectionTitle id="agents-title">Agents that carry {model.name}</SectionTitle>
            <SectionNote>Each agent&apos;s share of this model&apos;s tokens, and how many accounts used the model through it.</SectionNote>
          </div>
          {detail.agents.length > 0 ? (
            <Rows>
              {detail.agents.map((agent) => (
                <Row key={agent.key}>
                  <RowName>
                    <AgentMark agent={agent} size={20} />
                    {agent.name}
                  </RowName>
                  <RowMeter>
                    <ShareMeter share={agent.share} label="of this model's tokens" />
                  </RowMeter>
                  <RowValue>
                    {formatCount(agent.activeUsers)}
                    <span>active users</span>
                  </RowValue>
                </Row>
              ))}
            </Rows>
          ) : (
            <Empty>No agent data for this period.</Empty>
          )}
        </Section>

        {research ? <ResearchSections research={research} modelName={model.name} period={period} /> : null}

        <Section aria-labelledby="humans-title">
          <div>
            <SectionTitle id="humans-title">Top public profiles</SectionTitle>
            <SectionNote>
              Public profiles that used {model.name} most in this period, by tokens. Hidden accounts are never listed.
            </SectionNote>
          </div>
          {detail.humans.length > 0 ? (
            <Rows>
              {detail.humans.map((human) => (
                <PersonRow key={human.username} data-row-link="">
                  <PersonRank>{human.rank}</PersonRank>
                  <Avatar src={human.avatarUrl ?? `https://github.com/${human.username}.png?size=72`} alt="" width={36} height={36} loading="lazy" />
                  <PersonName>
                    <RowLink href={profilePath(human.username)} data-row-primary="">
                      {human.displayName ?? human.username}
                    </RowLink>
                    <span>@{human.username}</span>
                  </PersonName>
                  <RowValue>
                    {formatTokens(human.tokens)}
                    <span>tokens</span>
                  </RowValue>
                  <HideOnNarrow>
                    {formatEstimatedCost(human.estimatedCost, human.tokens)}
                    <span>est. cost</span>
                  </HideOnNarrow>
                  <HideOnNarrow>
                    {formatCount(human.activeDays)}
                    <span>active days</span>
                  </HideOnNarrow>
                  <ShowOnNarrow>
                    {estimatedCostPhrase(human.estimatedCost, human.tokens)} · {formatCount(human.activeDays)} active days
                  </ShowOnNarrow>
                </PersonRow>
              ))}
            </Rows>
          ) : (
            <Empty>No public profiles used this model in this period.</Empty>
          )}
        </Section>

        <Section aria-labelledby="related-title">
          <div>
            <SectionTitle id="related-title">More from {labName}</SectionTitle>
            <SectionNote>Other {labName} models in the same period, by active users.</SectionNote>
          </div>
          {detail.related.length > 0 ? (
            <Rows>
              {detail.related.map((related) => (
                <Row key={related.key} data-row-link="">
                  <RowName>
                    <ModelMark model={related} size={20} />
                    <RowLink href={modelPath(related.key)} data-row-primary="">
                      {related.name}
                    </RowLink>
                  </RowName>
                  <RowMeter>
                    <ShareMeter share={related.adoption} label="adoption" />
                  </RowMeter>
                  <RowValue>
                    {formatCount(related.activeUsers)}
                    <span>active users</span>
                  </RowValue>
                </Row>
              ))}
            </Rows>
          ) : (
            <Empty>No other {labName} models with data in this period.</Empty>
          )}
          <SmallLink href={modelsAllPath()}>All models</SmallLink>
        </Section>

        {research && hasResearch ? (
          <FullRow>
            <SourcesFooter notes={research.sources} />
          </FullRow>
        ) : null}
      </Grid>
    </>
  );
}
