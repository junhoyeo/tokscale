"use client";

/**
 * Benchmarks section of a model page: one row per tracked benchmark with a
 * result for the model (best score, rank among matched models, conditions,
 * date, source), the model's other results behind a disclosure, and the
 * dashboard benchmarks it was not evaluated in. A missing result is "Not
 * evaluated", never zero.
 */
import Link from "next/link";
import styled from "styled-components";
import type { ExplorePeriod } from "@/lib/explore";
import type { AgentUsage, ModelBenchmarksView, ModelBenchmarkView, ModelResultView } from "@/lib/benchmarks/modelPage";
import type { BenchmarkInfo } from "@/lib/benchmarks/attribution";
import { formatIsoDay } from "@/lib/benchmarks/format";
import { benchmarkPath, benchmarksPath } from "@/lib/benchmarks/urls";
import { formatCount, formatShare } from "./format";
import { AgentMark, InlineLink, Section, SectionNote, SectionTitle, VisuallyHidden } from "./ExploreUI";
import { ScoreValue } from "./ScoreValue";

/** Below this width the row stacks: name and score, then one wrapping line of facts. */
const WIDE = "@media (min-width: 960px)";
const NARROW = "@media (max-width: 959.98px)";
const COLUMNS = "minmax(0, 1.5fr) 128px 112px minmax(0, 1.1fr) 136px minmax(0, 1fr)";

const Head = styled.div`
  display: grid;
  align-items: start;
  justify-content: space-between;
  gap: 8px 24px;

  ${WIDE} {
    grid-template-columns: minmax(0, 1fr) auto;
  }
`;

const HeadLink = styled(InlineLink)`
  flex: none;
  font-size: 0.875rem;
`;

/* Header labels sit flush with their columns: score, rank and date on the right edge. */
const ColumnHead = styled.div`
  display: none;

  ${WIDE} {
    display: grid;
    grid-template-columns: ${COLUMNS};
    gap: 24px;
    padding: 8px 0;
    border-bottom: 1px solid var(--service-border);
    color: var(--service-text-muted);
    font-size: 0.8125rem;
    font-weight: 400;

    > span:nth-child(2),
    > span:nth-child(3),
    > span:nth-child(5) {
      text-align: right;
    }

    /* The score's source popover (22px plus its 4px gap) follows the number, so the
       header sits flush with the digits, not with the icon. */
    > span:nth-child(2) {
      padding-right: 26px;
    }
  }
`;

const Rows = styled.ul`
  margin: 0;
  padding: 0;
  border-top: 1px solid var(--service-border);
  list-style: none;

  ${WIDE} {
    border-top: 0;
  }
`;

const Row = styled.li`
  display: grid;
  grid-template-columns: minmax(0, 1fr) auto;
  gap: 6px 16px;
  align-items: start;
  padding: 12px 0;
  border-bottom: 1px solid var(--service-border);
  font-size: 0.875rem;
  font-variant-numeric: tabular-nums;

  ${WIDE} {
    grid-template-columns: ${COLUMNS};
    gap: 6px 24px;
  }
`;

const Name = styled.div`
  display: grid;
  min-width: 0;
  gap: 2px;
`;

const NameLink = styled(Link)`
  color: var(--service-text);
  font-weight: 500;
  text-decoration: none;

  &:hover {
    text-decoration: underline !important;
  }

  &:focus-visible {
    border-radius: 2px;
    outline: 2px solid var(--service-focus);
    outline-offset: 2px;
  }

  @media (max-width: 640px) {
    font-size: 1rem;
  }
`;

const Category = styled.span`
  color: var(--service-text-muted);
  font-size: 0.8125rem;

  @media (max-width: 640px) {
    font-size: 0.875rem;
  }
`;

/* The key number: right-aligned in its column, and at the row's right edge when the row stacks. */
const Score = styled.div`
  display: flex;
  min-width: 0;
  justify-content: flex-end;
  font-size: 0.875rem;
  font-weight: 600;

  ${NARROW} {
    justify-self: end;
  }
`;

/* Wide: the facts are cells of the row grid. Narrow: one wrapping line. */
const Facts = styled.div`
  display: contents;

  ${NARROW} {
    display: flex;
    grid-column: 1 / -1;
    flex-wrap: wrap;
    align-items: baseline;
    gap: 2px 0;
    color: var(--service-text-muted);
    font-size: 0.8125rem;

    > *:not(:last-child)::after {
      content: "·";
      margin: 0 8px;
      color: var(--service-border-strong);
    }
  }

  @media (max-width: 640px) {
    font-size: 0.875rem;
  }
`;

const Fact = styled.span`
  min-width: 0;
  color: var(--service-text);
  overflow-wrap: anywhere;

  ${NARROW} {
    color: var(--service-text-muted);
  }
`;

const FactMuted = styled(Fact)`
  color: var(--service-text-muted);
`;

const RankFact = styled(Fact)`
  ${WIDE} {
    text-align: right;
  }
`;

/* Keeps the rank column on wide screens for rows without a rank; gone on narrow ones. */
const EmptyFact = styled.span`
  ${NARROW} {
    display: none;
  }
`;

/* "Rank", "Source:" labels: the column head carries them on wide screens. */
const NarrowLabel = styled.span`
  ${WIDE} {
    display: none;
  }
`;

const RankOf = styled.span`
  color: var(--service-text-muted);
`;

/* DOM order reads "Published Oct 6, 2026"; wide screens show the date first, on the column's right edge. */
const DateFact = styled(Fact)`
  ${WIDE} {
    display: flex;
    flex-direction: column-reverse;
    align-items: flex-end;
    gap: 2px;
    text-align: right;

    > span:first-child {
      color: var(--service-text-muted);
      font-size: 0.8125rem;
    }
  }
`;

const SourceFact = styled(Fact)`
  ${WIDE} {
    display: grid;
    gap: 2px;
  }
`;

const ExternalLink = styled.a`
  color: var(--service-text);
  text-decoration: none;

  &:hover {
    text-decoration: underline !important;
  }

  &:focus-visible {
    border-radius: 2px;
    outline: 2px solid var(--service-focus);
    outline-offset: 2px;
  }
`;

const Wide = styled.div`
  grid-column: 1 / -1;
  min-width: 0;
`;

const UsageLine = styled.p`
  display: flex;
  align-items: center;
  gap: 8px;
  margin: 0;
  color: var(--service-text-muted);
  font-size: 0.8125rem;
  line-height: 1.5;

  strong {
    color: var(--service-text);
    font-weight: 600;
  }

  @media (max-width: 640px) {
    align-items: flex-start;
    font-size: 0.875rem;
  }
`;

const RowNote = styled.p`
  margin: 0;
  color: var(--service-text-muted);
  font-size: 0.8125rem;
  line-height: 1.5;

  @media (max-width: 640px) {
    font-size: 0.875rem;
  }
`;

const More = styled.details`
  > summary {
    display: inline-flex;
    min-height: 28px;
    align-items: center;
    color: var(--service-text);
    font-size: 0.8125rem;
    cursor: pointer;

    @media (max-width: 640px), (pointer: coarse) {
      min-height: 44px;
      font-size: 0.875rem;
    }
  }

  > summary:hover {
    text-decoration: underline;
  }

  > summary:focus-visible {
    border-radius: 2px;
    outline: 2px solid var(--service-focus);
    outline-offset: 2px;
  }

  &[open] .when-closed,
  &:not([open]) .when-open {
    display: none;
  }
`;

const OtherRows = styled.ul`
  margin: 4px 0 0;
  padding: 0;
  border-top: 1px solid var(--service-border);
  list-style: none;
`;

const OtherRow = styled(Row)`
  padding: 8px 0;

  &:last-child {
    border-bottom: 0;
  }
`;

const ListedAs = styled.span`
  min-width: 0;
  color: var(--service-text-muted);
  overflow-wrap: anywhere;
`;

const Footer = styled.p`
  margin: 0;
  color: var(--service-text-muted);
  font-size: 0.8125rem;
  line-height: 1.6;

  @media (max-width: 640px) {
    font-size: 0.875rem;
  }
`;

const Empty = styled.p`
  margin: 0;
  color: var(--service-text-muted);
  font-size: 0.875rem;
`;

const DATE_KIND_LABELS = { run: "Run date", published: "Published", release: "Model released" } as const;

/** "in the last 30 days", "across all time". */
export function periodPhrase(period: ExplorePeriod): string {
  if (period === "7d") return "in the last 7 days";
  if (period === "30d") return "in the last 30 days";
  return "across all time";
}

function ResultFacts({ result, rank }: { result: ModelResultView; rank: { rank: number; of: number } | null }) {
  const { cell } = result;
  return (
    <Facts>
      {rank ? (
        <RankFact>
          <NarrowLabel>Rank </NarrowLabel>#{formatCount(rank.rank)} <RankOf>of {formatCount(rank.of)}</RankOf>
          <VisuallyHidden> matched models</VisuallyHidden>
        </RankFact>
      ) : (
        <EmptyFact aria-hidden="true" />
      )}
      {result.conditions ? <Fact>{result.conditions}</Fact> : <FactMuted>No conditions reported</FactMuted>}
      {cell.date && cell.dateKind ? (
        <DateFact>
          <span>{DATE_KIND_LABELS[cell.dateKind]} </span>
          <span>{formatIsoDay(cell.date)}</span>
        </DateFact>
      ) : (
        <FactMuted>No date reported</FactMuted>
      )}
      <SourceFact>
        <span>
          <NarrowLabel>Source: </NarrowLabel>
          <ExternalLink href={cell.sourceUrl} target="_blank" rel="noopener noreferrer">
            {sourceHost(cell.sourceUrl)}
            <VisuallyHidden> (opens in a new tab)</VisuallyHidden>
          </ExternalLink>
        </span>
      </SourceFact>
    </Facts>
  );
}

/** "tbench.ai", "epoch.ai": the site the source link opens. */
export function sourceHost(url: string): string {
  try {
    return new URL(url).hostname.replace(/^www\./, "");
  } catch {
    return "Source";
  }
}

function UsageNote({ usage, modelName, period }: { usage: AgentUsage; modelName: string; period: ExplorePeriod }) {
  return (
    <UsageLine>
      <AgentMark agent={usage} size={16} />
      <span>
        <strong>Tokscale usage:</strong>{" "}
        {usage.share > 0 ? (
          <>
            {usage.name} carried {formatShare(usage.share)} of {modelName} tokens {periodPhrase(period)}
            {usage.activeUsers > 0
              ? ` (${formatCount(usage.activeUsers)} active ${usage.activeUsers === 1 ? "user" : "users"})`
              : ""}
            .
          </>
        ) : (
          <>
            no {modelName} tokens went through {usage.name} {periodPhrase(period)}.
          </>
        )}
      </span>
    </UsageLine>
  );
}

function BenchmarkRow({
  row,
  modelName,
  period,
}: {
  row: ModelBenchmarkView;
  modelName: string;
  period: ExplorePeriod | null;
}) {
  const info: BenchmarkInfo = row.info;
  const notes = [...(row.history && row.statusNote ? [row.statusNote] : []), ...row.notes];
  const usages = [row.best, ...row.others].map((r) => r.agentUsage);
  // One usage line per agent: several efforts of the same agent share it.
  const agentLines = usages.filter(
    (usage, index): usage is AgentUsage => usage !== null && usages.findIndex((u) => u?.key === usage.key) === index,
  );
  return (
    <Row data-row-link="">
      <Name>
        <NameLink href={benchmarkPath(row.slug)} data-row-primary="">
          {info.name}
        </NameLink>
        <Category>
          {row.categoryLabel}
          {info.collectedBy ? ` · ${info.publisher}, collected by ${info.collectedBy}` : ` · ${info.publisher}`}
        </Category>
      </Name>
      <Score>
        <ScoreValue cell={row.best.cell} info={info} modelName={modelName} align="end" showTag={false} />
      </Score>
      <ResultFacts result={row.best} rank={{ rank: row.rank, of: row.rankedModels }} />
      {period && agentLines.length > 0 ? (
        <Wide>
          {agentLines.map((usage) => (
            <UsageNote key={usage.key} usage={usage} modelName={modelName} period={period} />
          ))}
        </Wide>
      ) : null}
      {notes.length > 0 ? (
        <Wide>
          {notes.map((note) => (
            <RowNote key={note}>{note}</RowNote>
          ))}
        </Wide>
      ) : null}
      {row.others.length > 0 ? (
        <Wide>
          <More>
            <summary>
              <span className="when-closed">
                Show {row.others.length === 1 ? "1 more result" : `${row.others.length} more results`} with other run
                conditions
              </span>
              <span className="when-open">Hide the other results</span>
            </summary>
            <OtherRows>
              {row.others.map((other) => (
                <OtherRow key={other.id}>
                  <ListedAs>Listed as {other.cell.sourceName}</ListedAs>
                  <Score>
                    <ScoreValue cell={other.cell} info={info} modelName={modelName} align="end" showTag={false} />
                  </Score>
                  <ResultFacts result={other} rank={null} />
                </OtherRow>
              ))}
            </OtherRows>
          </More>
        </Wide>
      ) : null}
    </Row>
  );
}

export function ModelBenchmarks({
  benchmarks,
  modelName,
  period,
}: {
  benchmarks: ModelBenchmarksView;
  modelName: string;
  /** The usage period of the page; null on pages for models without Tokscale usage. */
  period: ExplorePeriod | null;
}) {
  const hasAgentUsage = period !== null && benchmarks.rows.some((row) => [row.best, ...row.others].some((r) => r.agentUsage));
  return (
    <Section aria-labelledby="benchmarks-title">
      <Head>
        <div>
          <SectionTitle id="benchmarks-title">Benchmarks</SectionTitle>
          <SectionNote>
            The best result for {modelName} in each benchmark Tokscale tracks. Each benchmark has its own unit, run
            conditions and source, so compare a score only with other models on the same benchmark. Rank is among the
            models Tokscale matched in that benchmark.
            {hasAgentUsage
              ? " Agent rows also show Tokscale usage through that agent, side by side with the score. Usage does not explain the score."
              : ""}
          </SectionNote>
        </div>
        <HeadLink href={benchmarksPath()}>All benchmarks</HeadLink>
      </Head>

      {benchmarks.rows.length > 0 ? (
        <div>
          <ColumnHead aria-hidden="true">
            <span>Benchmark</span>
            <span>Score</span>
            <span>Rank</span>
            <span>Conditions</span>
            <span>Date</span>
            <span>Source</span>
          </ColumnHead>
          <Rows>
            {benchmarks.rows.map((row) => (
              <BenchmarkRow key={row.slug} row={row} modelName={modelName} period={period} />
            ))}
          </Rows>
        </div>
      ) : (
        <Empty>No benchmark that Tokscale tracks has a result for {modelName} yet.</Empty>
      )}

      {benchmarks.rows.length > 0 && benchmarks.notEvaluated.length > 0 ? (
        <Footer>
          Not evaluated:{" "}
          {benchmarks.notEvaluated.map((item, index) => (
            <span key={item.slug}>
              {index > 0 ? ", " : null}
              <InlineLink href={benchmarkPath(item.slug)}>{item.name}</InlineLink>
            </span>
          ))}
          . A missing result is not a score of zero.
        </Footer>
      ) : null}
    </Section>
  );
}
