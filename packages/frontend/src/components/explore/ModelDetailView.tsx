"use client";

/**
 * Body of /models/[key]: identity, period, summary facts, trend, token mix,
 * agents, related models, benchmarks, pricing, specs, top public profiles,
 * coverage and sources.
 *
 * Desktop grid (six columns): the trend beside the token mix, the agents beside
 * the lab's other models (two lists of the same shape), then the research and
 * the profiles at full width. Every panel keeps its own height.
 */
import { useMemo } from "react";
import Link from "next/link";
import styled from "styled-components";
import type { VariantInput } from "dither-kit/dither-paint";
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
import { DitherStack, DitherSwatch } from "./DitherViz";
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

/* Identity on the left, the period on the right; the period wraps under it on narrow screens. */
const IdentityBar = styled.div`
  display: flex;
  flex-wrap: wrap;
  align-items: flex-end;
  justify-content: space-between;
  gap: 16px 32px;
  margin-bottom: 20px;

  > :first-child {
    flex: 1 1 auto;
  }
`;

const PeriodBar = styled(ControlBar)`
  margin-bottom: 0;
`;

const Notice = styled.p`
  margin: 0 0 16px;
  padding: 10px 12px;
  border-radius: 2px;
  background: var(--service-surface-muted);
  color: var(--service-text);
  font-size: 0.875rem;
  line-height: 1.5;
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
  font-size: 0.8125rem;
  white-space: nowrap;
`;

const CoverageRow = styled(CoverageLine)`
  margin-bottom: 24px;
`;

const Grid = styled.div`
  display: grid;
  grid-template-columns: minmax(0, 1fr);
  align-items: start;
  gap: 32px;

  @media (min-width: 1024px) {
    grid-template-columns: repeat(6, minmax(0, 1fr));
    gap: 40px;
  }
`;

/* A section in the grid, spanning this many of the six desktop columns. */
const Cell = styled(Section)<{ $span: 2 | 3 | 4 | 6 }>`
  @media (min-width: 1024px) {
    grid-column: span ${({ $span }) => $span};
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

/* Title and note on the left, the section's one link on the right. */
const SectionHead = styled.div`
  display: flex;
  flex-wrap: wrap;
  align-items: flex-start;
  justify-content: space-between;
  gap: 8px 24px;
`;

/* Two lists share a row only when their heads are the same height: reserve room for two note lines in both. */
const ListHead = styled.div`
  min-height: 62px;
`;

const HeadLink = styled(InlineLink)`
  flex: none;
  font-size: 0.875rem;
`;

const MixRows = styled.ul`
  margin: 0;
  padding: 0;
  border-top: 1px solid var(--service-border);
  list-style: none;
`;

const MixRow = styled.li`
  display: flex;
  min-height: 46px;
  align-items: center;
  gap: 10px;
  border-bottom: 1px solid var(--service-border);
  color: var(--service-text);
  font-size: 0.875rem;
  font-variant-numeric: tabular-nums;
`;

/* Fixed slot, so the labels share one edge whether or not a part has a mark. */
const SwatchSlot = styled.span`
  display: inline-flex;
  width: 18px;
  flex: 0 0 auto;
`;

const MixShare = styled.span<{ $zero: boolean }>`
  margin-left: auto;
  color: ${({ $zero }) => ($zero ? "var(--service-text-muted)" : "var(--service-text)")};
  text-align: right;
`;

const Rows = styled.ol`
  margin: 0;
  padding: 0;
  border-top: 1px solid var(--service-border);
`;

const Row = styled.li<{ $wide?: boolean }>`
  position: relative;
  display: grid;
  grid-template-columns: minmax(0, 1fr) minmax(120px, 180px) 88px;
  gap: 4px 16px;
  align-items: center;
  min-height: 46px;
  padding: 6px 0;
  list-style: none;
  border-bottom: 1px solid var(--service-border);
  font-size: 0.875rem;
  font-variant-numeric: tabular-nums;

  /* A list alone in its row: the name keeps a readable measure and the bar takes the rest,
     so the count still lands on the section's right edge. */
  @media (min-width: 1024px) {
    ${({ $wide }) => ($wide ? "grid-template-columns: minmax(0, 320px) minmax(0, 1fr) 88px;" : "")}
  }

  @media (max-width: 520px) {
    grid-template-columns: minmax(0, 1fr) auto;
    padding: 8px 0;
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
  white-space: nowrap;
`;

const RowLabel = styled.span`
  min-width: 0;
  overflow: hidden;
  text-overflow: ellipsis;
`;

/* A count over its unit, both on the column's right edge. */
const RowValue = styled.span`
  color: var(--service-text);
  line-height: 18px;
  text-align: right;
  white-space: nowrap;

  > span {
    display: block;
    color: var(--service-text-muted);
    font-size: 0.8125rem;
    line-height: 16px;
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
  min-width: 0;
  overflow: hidden;
  color: var(--service-text);
  text-decoration: none;
  text-overflow: ellipsis;
  white-space: nowrap;

  /* globals.css resets a:hover decoration with !important. */
  &:hover {
    text-decoration: underline !important;
  }
`;

/* Ten profiles read as two columns of five on wide screens, in rank order down each column. */
const PeopleRows = styled(Rows)<{ $rows: number }>`
  @media (min-width: 1200px) {
    display: grid;
    grid-auto-flow: column;
    grid-template-columns: repeat(2, minmax(0, 1fr));
    grid-template-rows: repeat(${({ $rows }) => $rows}, auto);
    column-gap: 40px;
    border-top: 0;

    > li:first-child,
    > li:nth-child(${({ $rows }) => $rows + 1}) {
      border-top: 1px solid var(--service-border);
    }
  }
`;

const PersonRow = styled(Row)`
  grid-template-columns: 24px 32px minmax(0, 1fr) 72px 72px 80px;
  gap: 4px 12px;
  min-height: 56px;
  padding: 8px 0;

  @media (max-width: 520px) {
    grid-template-columns: 24px 32px minmax(0, 1fr) auto;
  }
`;

const PersonRank = styled.span`
  color: var(--service-text-muted);
  text-align: right;
`;

const Avatar = styled.img`
  width: 32px;
  height: 32px;
  border-radius: 2px;
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
  font-size: 0.875rem;

  @media (max-width: 520px) {
    display: block;
  }
`;

const Empty = styled.p`
  margin: 0;
  color: var(--service-text-muted);
  font-size: 0.875rem;
`;

/* Rows a list shows before the rest fold away: the same 6 the related-model list is capped at. */
const LIST_ROWS = 6;

const MIX_COLORS: Record<TokenMixKey, string> = {
  input: "#2f8fff",
  output: "#9cc9ff",
  cacheRead: "#6b7686",
  cacheWrite: "#3a4352",
  reasoning: "#d7dee8",
};

/* Each kind also has its own dither texture; a legend swatch is a slice of its part. */
const MIX_TEXTURES: Record<TokenMixKey, VariantInput> = {
  input: "dotted",
  output: "hatched",
  cacheRead: "gradient",
  cacheWrite: "solid",
  reasoning: "dotted",
};

function TokenMixSection({ mix, tokens }: { mix: TokenMix; tokens: number }) {
  const parts = useMemo(() => tokenMixParts(mix), [mix]);
  const stack = useMemo(
    () =>
      parts
        .filter((part) => part.share > 0)
        .map((part) => ({ color: MIX_COLORS[part.key], variant: MIX_TEXTURES[part.key], share: part.share })),
    [parts],
  );
  return (
    <Cell $span={2} aria-labelledby="token-mix-title">
      <div>
        <SectionTitle id="token-mix-title">Token mix</SectionTitle>
        <SectionNote>
          How the {formatTokens(tokens)} tokens split. Cache reads are usually most of an agent&apos;s tokens and are priced
          far below input.
        </SectionNote>
      </div>
      {stack.length > 0 ? (
        <>
          <DitherStack parts={stack} height={16} />
          <MixRows>
            {parts.map((part) => (
              <MixRow key={part.key}>
                <SwatchSlot>
                  {part.share > 0 ? <DitherSwatch color={MIX_COLORS[part.key]} variant={MIX_TEXTURES[part.key]} /> : null}
                </SwatchSlot>
                <span>{part.label}</span>
                <MixShare $zero={part.share === 0}>{formatShare(part.share)}</MixShare>
              </MixRow>
            ))}
          </MixRows>
        </>
      ) : (
        <Empty>No token breakdown for this period.</Empty>
      )}
    </Cell>
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
      <Cell $span={6} aria-labelledby="research-title">
        <div>
          <SectionTitle id="research-title">Benchmarks, pricing and specs</SectionTitle>
          <SectionNote>
            No benchmark result and no models.dev entry match this model key yet, so there are no scores, list prices or
            specs to show.
          </SectionNote>
        </div>
      </Cell>
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
  /*
   * The agents and the lab's other models are the same kind of list, so they can
   * share a row only when they hold the same number of rows: two lists of
   * different lengths end at different heights and read as a mistake. Both show
   * the top N (six at most) and link out for the rest; when there is not enough
   * of either to fill the row, each takes the full width, one under the other.
   */
  const listRows = Math.min(LIST_ROWS, detail.agents.length, detail.related.length);
  const paired = listRows > 2;
  const agentsShown = detail.agents.slice(0, paired ? listRows : LIST_ROWS);
  const relatedShown = detail.related.slice(0, paired ? listRows : LIST_ROWS);
  const stackLists = !paired;

  return (
    <>
      <ModelBreadcrumb items={modelCrumbs(model)} />

      <IdentityBar>
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
        <PeriodBar>
          <LinkSegments label="Period" visibleLabel="Period" options={periodOptions} />
        </PeriodBar>
      </IdentityBar>

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
        <Cell $span={4} aria-label="Daily usage">
          <TrendChart trend={detail.trend} modelName={model.name} />
        </Cell>

        <TokenMixSection mix={summary.tokenMix} tokens={summary.tokens} />

        <Cell $span={stackLists ? 6 : 3} aria-labelledby="agents-title">
          <SectionHead>
            <ListHead>
              <SectionTitle id="agents-title">Agents that carry {model.name}</SectionTitle>
              <SectionNote>Each agent&apos;s share of this model&apos;s tokens, and how many accounts used the model through it.</SectionNote>
            </ListHead>
            <HeadLink href="/leaderboard?view=agents">All agents</HeadLink>
          </SectionHead>
          {agentsShown.length > 0 ? (
            <Rows>
              {agentsShown.map((agent) => (
                <Row key={agent.key} $wide={stackLists}>
                  <RowName>
                    <AgentMark agent={agent} size={20} />
                    <RowLabel>{agent.name}</RowLabel>
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
        </Cell>

        <Cell $span={stackLists ? 6 : 3} aria-labelledby="related-title">
          <SectionHead>
            <ListHead>
              <SectionTitle id="related-title">More from {labName}</SectionTitle>
              <SectionNote>Other {labName} models in the same period, by active users.</SectionNote>
            </ListHead>
            <HeadLink href={modelsAllPath()}>All models</HeadLink>
          </SectionHead>
          {relatedShown.length > 0 ? (
            <Rows>
              {relatedShown.map((related) => (
                <Row key={related.key} $wide={stackLists} data-row-link="">
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
        </Cell>

        {research ? <ResearchSections research={research} modelName={model.name} period={period} /> : null}

        <Cell $span={6} aria-labelledby="humans-title">
          <div>
            <SectionTitle id="humans-title">Top public profiles</SectionTitle>
            <SectionNote>
              Public profiles that used {model.name} most in this period, by tokens. Hidden accounts are never listed.
            </SectionNote>
          </div>
          {detail.humans.length > 0 ? (
            <PeopleRows $rows={Math.ceil(detail.humans.length / 2)}>
              {detail.humans.map((human) => (
                <PersonRow key={human.username} data-row-link="">
                  <PersonRank>{human.rank}</PersonRank>
                  <Avatar src={human.avatarUrl ?? `https://github.com/${human.username}.png?size=72`} alt="" width={32} height={32} loading="lazy" />
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
            </PeopleRows>
          ) : (
            <Empty>No public profiles used this model in this period.</Empty>
          )}
        </Cell>

        {research && hasResearch ? (
          <FullRow>
            <SourcesFooter notes={research.sources} />
          </FullRow>
        ) : null}
      </Grid>
    </>
  );
}
