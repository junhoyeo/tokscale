"use client";

/**
 * Body of /benchmarks/[slug]: what the benchmark measures and how it is run,
 * every result with its conditions, date and source next to Tokscale usage,
 * the score by release date chart, method and matching notes, and sources.
 */
import Link from "next/link";
import { useEffect, useId, useRef, useState } from "react";
import styled from "styled-components";
import { PrimaryActionLink } from "@/components/leaderboard/RankingUI";
import { InstantTooltip } from "@/components/ui/InstantTooltip";
import type { SourceNote } from "@/lib/benchmarks/attribution";
import type { BenchmarkPageInfo, DateKind, MappingSummary, ResultColumns, ResultRow, TimelineData } from "@/lib/benchmarks/detail";
import { formatIsoDay } from "@/lib/benchmarks/format";
import { benchmarksPath } from "@/lib/benchmarks/urls";
import { formatCount, formatDateRange, formatTokens } from "./format";
import { ButtonSegments, ModelMark, PageLead, PageTitle, Section, SectionNote, SectionTitle, VisuallyHidden } from "./ExploreUI";
import { ScoreTimeline } from "./ScoreTimeline";
import { SourcesFooter } from "./SourcesFooter";

/** Rows rendered before "Show all": enough for the leaders, short enough to reach the chart on a phone. */
const PAGE_SIZE = 50;
/** Below this width the results table stacks each row. */
const STACK_BELOW = "959px";

const Breadcrumb = styled.nav`
  margin-bottom: 12px;

  ol {
    display: flex;
    flex-wrap: wrap;
    align-items: center;
    gap: 4px;
    margin: 0;
    padding: 0;
    list-style: none;
    color: var(--service-text-muted);
    font-size: 0.8125rem;
  }

  li:not(:last-child)::after {
    content: "/";
    margin-left: 6px;
    color: var(--service-border-strong);
  }

  @media (max-width: 640px) {
    ol {
      font-size: 0.875rem;
    }
  }
`;

const CrumbLink = styled(Link)`
  color: var(--service-text-muted);
  text-decoration: none;

  &:hover {
    color: var(--service-text);
  }

  &:focus-visible {
    border-radius: 4px;
    outline: 2px solid var(--service-focus);
    outline-offset: 2px;
  }

  @media (pointer: coarse), (max-width: 640px) {
    display: inline-flex;
    min-height: 44px;
    align-items: center;
  }
`;

const Identity = styled.header`
  display: grid;
  gap: 10px;
  margin-bottom: 24px;
`;

const Byline = styled.p`
  display: flex;
  flex-wrap: wrap;
  align-items: center;
  gap: 4px 0;
  margin: 0;
  color: var(--service-text-muted);
  font-size: 0.875rem;

  > span:not(:last-child)::after {
    content: "·";
    margin: 0 8px;
    color: var(--service-border-strong);
  }
`;

const Facts = styled.dl`
  display: grid;
  grid-template-columns: repeat(auto-fit, minmax(min(100%, 140px), 1fr));
  gap: 16px 24px;
  margin: 0 0 16px;
  padding: 14px 0;
  border-top: 1px solid var(--service-border);
  border-bottom: 1px solid var(--service-border);
`;

const Fact = styled.div`
  display: grid;
  align-content: start;
  gap: 2px;
  min-width: 0;

  dt {
    color: var(--service-text-muted);
    font-size: 0.75rem;
    font-weight: 500;
  }

  dd {
    margin: 0;
    color: var(--service-text);
    font-size: 0.9375rem;
    font-variant-numeric: tabular-nums;
    overflow-wrap: anywhere;
  }

  @media (max-width: 640px) {
    dt {
      font-size: 0.8125rem;
    }

    dd {
      font-size: 1rem;
    }
  }
`;

const FactSub = styled.span`
  display: block;
  color: var(--service-text-muted);
  font-size: 0.75rem;

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
    font-weight: 500;
  }

  @media (max-width: 640px) {
    font-size: 1rem;
  }
`;

const Note = styled.p`
  max-width: 80ch;
  margin: 0;
  padding-left: 10px;
  border-left: 2px solid var(--service-border-strong);
  color: var(--service-text-muted);
  font-size: 0.875rem;
  line-height: 1.5;

  @media (max-width: 640px) {
    font-size: 1rem;
  }
`;

const ExternalLink = styled.a`
  color: var(--service-accent-hover);
  text-decoration: none;

  &:hover {
    text-decoration: underline !important;
  }

  &:focus-visible {
    border-radius: 4px;
    outline: 2px solid var(--service-focus);
    outline-offset: 2px;
  }
`;

const Sections = styled.div`
  display: grid;
  gap: 48px;
  margin-top: 32px;
`;

const ResultsHead = styled.div`
  display: flex;
  flex-wrap: wrap;
  align-items: flex-end;
  justify-content: space-between;
  gap: 12px 24px;
`;

const HeadText = styled.div`
  display: grid;
  gap: 4px;
  max-width: 80ch;
`;

const Table = styled.table`
  width: 100%;
  border-collapse: collapse;
  font-size: 0.875rem;
  font-variant-numeric: tabular-nums;

  th,
  td {
    padding: 12px 16px 12px 0;
    border-bottom: 1px solid var(--service-border);
    text-align: left;
    vertical-align: top;
  }

  th:last-child,
  td:last-child {
    padding-right: 0;
  }

  thead th {
    padding-top: 8px;
    padding-bottom: 8px;
    border-bottom-color: var(--service-border-strong);
    color: var(--service-text-muted);
    font-size: 0.75rem;
    font-weight: 500;
    vertical-align: bottom;
    white-space: nowrap;
  }

  td[data-col="rank"] {
    width: 2.5em;
    color: var(--service-text-muted);
  }

  td[data-col="score"],
  thead th[data-col="score"],
  td[data-col="usage"],
  thead th[data-col="usage"] {
    text-align: right;
  }

  td[data-col="score"] {
    white-space: nowrap;
  }

  @media (max-width: ${STACK_BELOW}) {
    display: block;

    thead {
      position: absolute;
      width: 1px;
      height: 1px;
      overflow: hidden;
      clip: rect(0 0 0 0);
      clip-path: inset(50%);
      white-space: nowrap;
    }

    tbody {
      display: block;
    }

    tbody tr {
      display: grid;
      grid-template-columns: 2em minmax(0, 1fr) auto;
      grid-template-areas:
        "rank model score"
        ". cond cond"
        ". cost cost"
        ". date date"
        ". usage source";
      gap: 4px 12px;
      padding: 14px 0;
      border-bottom: 1px solid var(--service-border);
    }

    th,
    td {
      display: block;
      min-width: 0;
      padding: 0;
      border: 0;
    }

    td[data-col="rank"] {
      grid-area: rank;
      width: auto;
      padding-top: 1px;
    }

    th[data-col="model"] {
      grid-area: model;
    }

    td[data-col="score"] {
      grid-area: score;
    }

    td[data-col="cond"] {
      grid-area: cond;
    }

    td[data-col="cost"] {
      grid-area: cost;
    }

    td[data-col="date"] {
      grid-area: date;
    }

    td[data-col="usage"] {
      grid-area: usage;
      text-align: left;
    }

    td[data-col="source"] {
      grid-area: source;
      justify-self: end;
    }

    td[data-empty="true"] {
      display: none;
    }
  }

  @media (max-width: 640px) {
    font-size: 0.9375rem;
  }
`;

const ModelCell = styled.div`
  display: flex;
  min-width: 0;
  align-items: flex-start;
  gap: 10px;
  font-weight: 400;

  > span:first-child {
    margin-top: 1px;
  }
`;

const ModelText = styled.span`
  display: grid;
  min-width: 0;
  gap: 2px;
  overflow-wrap: anywhere;
`;

const ModelLink = styled(Link)`
  color: var(--service-text);
  font-weight: 600;
  text-decoration: none;

  &:hover {
    color: var(--service-accent-hover);
  }

  &:focus-visible {
    border-radius: 4px;
    outline: 2px solid var(--service-focus);
    outline-offset: 2px;
  }
`;

const ModelName = styled.span`
  color: var(--service-text);
  font-weight: 600;
`;

const UnmatchedMark = styled.span`
  width: 20px;
  height: 20px;
  flex: 0 0 auto;
  border: 1px dashed var(--service-border-strong);
  border-radius: 4px;
`;

const RowHead = styled.th`
  &:focus {
    outline: none;
  }

  &:focus-visible {
    outline: 2px solid var(--service-focus);
    outline-offset: 2px;
  }
`;

const Sub = styled.span`
  display: block;
  color: var(--service-text-muted);
  font-size: 0.75rem;
  font-weight: 400;
  line-height: 1.45;

  @media (max-width: 640px) {
    font-size: 0.8125rem;
  }
`;

const ScoreText = styled.span`
  color: var(--service-text);
  font-size: 0.9375rem;
  font-weight: 600;
`;

const MutedText = styled.span`
  color: var(--service-text-muted);
`;

const DateSub = styled(Sub)`
  @media (max-width: ${STACK_BELOW}) {
    display: inline;

    &::before {
      content: " · ";
    }
  }
`;

const UsageSub = styled(Sub)`
  @media (max-width: ${STACK_BELOW}) {
    display: inline;

    &::before {
      content: " ";
    }
  }
`;

const NarrowOnly = styled.span`
  display: none;

  @media (max-width: ${STACK_BELOW}) {
    display: inline;
  }
`;

const WideOnly = styled(InstantTooltip)`
  @media (max-width: ${STACK_BELOW}) {
    display: none;
  }
`;

const SourceLink = styled.a`
  display: inline-flex;
  align-items: center;
  gap: 2px;
  color: var(--service-accent-hover);
  text-decoration: none;
  white-space: nowrap;

  &:hover {
    text-decoration: underline !important;
  }

  &:focus-visible {
    border-radius: 4px;
    outline: 2px solid var(--service-focus);
    outline-offset: 2px;
  }

  @media (max-width: 640px), (pointer: coarse) {
    min-height: 44px;
  }
`;

const TableFoot = styled.div`
  display: flex;
  flex-wrap: wrap;
  align-items: center;
  gap: 8px 16px;
  margin-top: 4px;
`;

const Status = styled.p`
  margin: 0;
  color: var(--service-text-muted);
  font-size: 0.8125rem;
  font-variant-numeric: tabular-nums;

  @media (max-width: 640px) {
    font-size: 0.875rem;
  }
`;

const ShowAll = styled.button`
  min-height: 32px;
  padding: 0 12px;
  border: 1px solid var(--service-border-strong);
  border-radius: 8px;
  background: transparent;
  color: var(--service-text);
  font-size: 0.8125rem;
  cursor: pointer;

  &:hover {
    border-color: var(--service-text-muted);
  }

  &:focus-visible {
    outline: 2px solid var(--service-focus);
    outline-offset: 2px;
  }

  @media (max-width: 640px), (pointer: coarse) {
    min-height: 44px;
    font-size: 0.9375rem;
  }
`;

const NotesGrid = styled.div`
  display: grid;
  grid-template-columns: minmax(0, 1fr);
  gap: 32px;

  @media (min-width: 1024px) {
    grid-template-columns: repeat(3, minmax(0, 1fr));
    gap: 32px 48px;
  }
`;

const NoteList = styled.ul`
  display: grid;
  gap: 8px;
  margin: 8px 0 0;
  padding-left: 18px;
  color: var(--service-text-muted);
  font-size: 0.875rem;
  line-height: 1.55;
  list-style: disc;

  li::marker {
    color: var(--service-border-strong);
  }

  @media (max-width: 640px) {
    font-size: 1rem;
  }
`;

const NotFoundWrap = styled.div`
  display: grid;
  max-width: 560px;
  gap: 12px;
  padding: 48px 0;

  > a {
    justify-self: start;
    margin-top: 8px;
  }
`;

const DATE_KIND_LABELS: Record<DateKind, string> = {
  run: "Run date",
  published: "Published",
  release: "Model released",
};

const LAST_UPDATE_TEXT: Record<DateKind, string> = {
  run: "Newest run date",
  published: "Newest publish date",
  release: "Newest model release",
};

function NewTab() {
  return <VisuallyHidden> (opens in a new tab)</VisuallyHidden>;
}

/** "tbench.ai/leaderboard" from "https://www.tbench.ai/leaderboard". */
function shortUrl(url: string): string {
  try {
    const parsed = new URL(url);
    const path = parsed.pathname.replace(/\/+$/, "");
    return `${parsed.hostname.replace(/^www\./, "")}${path}`;
  } catch {
    return url;
  }
}

/** A short URL that wraps after its slashes instead of in the middle of a word. */
function WrappingUrl({ url }: { url: string }) {
  const parts = shortUrl(url).split("/");
  return (
    <>
      {parts.map((part, index) => (
        <span key={`${index}-${part}`}>
          {index > 0 ? (
            <>
              /<wbr />
            </>
          ) : null}
          {part}
        </span>
      ))}
    </>
  );
}

function plural(n: number, one: string, many: string): string {
  return `${formatCount(n)} ${n === 1 ? one : many}`;
}

function Header({ info }: { info: BenchmarkPageInfo }) {
  return (
    <>
      <Breadcrumb aria-label="Breadcrumb">
        <ol>
          <li>
            <CrumbLink href={benchmarksPath()}>Benchmarks</CrumbLink>
          </li>
          <li aria-current="page">{info.name}</li>
        </ol>
      </Breadcrumb>
      <Identity>
        <PageTitle>{info.name}</PageTitle>
        <PageLead>{info.measures}</PageLead>
        <Byline>
          <span>By {info.publisher}</span>
          {info.collectedBy ? <span>Collected by {info.collectedBy}</span> : null}
          <span>{info.categoryLabel}</span>
          {info.status === "history" ? <span>Kept as history</span> : null}
        </Byline>
      </Identity>

      <Facts>
        <Fact>
          <dt>Unit</dt>
          <dd>{info.unitLabel}</dd>
        </Fact>
        <Fact>
          <dt>Results</dt>
          <dd>
            {plural(info.resultCount, "result", "results")}
            <FactSub>
              {plural(info.modelCount, "Tokscale model", "Tokscale models")} matched, {formatCount(info.externalModelCount)} in the source
            </FactSub>
          </dd>
        </Fact>
        <Fact>
          <dt>Last update</dt>
          <dd>
            {info.lastUpdate ? formatIsoDay(info.lastUpdate.date) : "\u2014"}
            {info.lastUpdate ? <FactSub>{LAST_UPDATE_TEXT[info.lastUpdate.kind]}</FactSub> : null}
          </dd>
        </Fact>
        <Fact>
          <dt>Retrieved</dt>
          <dd>
            {info.retrievedAt ? formatIsoDay(info.retrievedAt) : "\u2014"}
            {info.dataDate ? <FactSub>Source data through {formatIsoDay(info.dataDate)}</FactSub> : null}
          </dd>
        </Fact>
        <Fact>
          <dt>License</dt>
          <dd>
            {info.licenseUrl ? (
              <ExternalLink href={info.licenseUrl} target="_blank" rel="noopener noreferrer">
                {info.licenseName}
                <NewTab />
              </ExternalLink>
            ) : (
              info.licenseName
            )}
          </dd>
        </Fact>
        <Fact>
          <dt>Source</dt>
          <dd>
            <ExternalLink href={info.url} target="_blank" rel="noopener noreferrer">
              <WrappingUrl url={info.url} />
              <span aria-hidden="true"> {"\u2197"}</span>
              <NewTab />
            </ExternalLink>
          </dd>
        </Fact>
      </Facts>

      <Identity as="div">
        <Prose>
          <strong>Conditions.</strong> {info.conditions}
        </Prose>
        <Prose>
          <strong>Attribution.</strong> {info.attribution}
        </Prose>
        {info.notes.map((note) => (
          <Note key={note}>{note}</Note>
        ))}
      </Identity>
    </>
  );
}

function UsageCell({ row, usageAvailable }: { row: ResultRow; usageAvailable: boolean }) {
  if (!usageAvailable) return null;
  if (row.activeUsers !== null) {
    return (
      <td data-col="usage" role="cell">
        {formatCount(row.activeUsers)}
        <UsageSub>
          active users<NarrowOnly aria-hidden="true"> on Tokscale, last 30 days</NarrowOnly>
        </UsageSub>
      </td>
    );
  }
  if (!row.modelKey) {
    return (
      <td data-col="usage" role="cell" data-empty="true">
        <span aria-hidden="true">{"\u2014"}</span>
        <VisuallyHidden>Not matched to a Tokscale model</VisuallyHidden>
      </td>
    );
  }
  return (
    <td data-col="usage" role="cell">
      <WideOnly aria-hidden="true" content="No Tokscale usage in the last 30 days" describe={false}>
        {"\u2014"}
      </WideOnly>
      <NarrowOnly aria-hidden="true">
        <MutedText>No Tokscale usage in the last 30 days</MutedText>
      </NarrowOnly>
      <VisuallyHidden>No Tokscale usage in the last 30 days</VisuallyHidden>
    </td>
  );
}

function ResultRowView({
  row,
  rank,
  columns,
  usageAvailable,
  rowHeadId,
}: {
  row: ResultRow;
  rank: number;
  columns: ResultColumns;
  usageAvailable: boolean;
  rowHeadId: string;
}) {
  const matched = row.modelKey !== null && row.modelName !== null;
  const name = matched ? (row.modelName as string) : row.sourceName;
  return (
    <tr role="row" data-row-link={row.href ? "" : undefined}>
      <td data-col="rank" role="cell">
        {rank}
      </td>
      <RowHead scope="row" role="rowheader" data-col="model" id={rowHeadId} tabIndex={-1}>
        <ModelCell>
          {matched ? <ModelMark model={{ name, logoUrl: row.logoUrl }} size={20} /> : <UnmatchedMark aria-hidden="true" />}
          <ModelText>
            {row.href ? (
              <ModelLink href={row.href} data-row-primary="">
                {name}
              </ModelLink>
            ) : (
              <ModelName>{name}</ModelName>
            )}
            {matched && row.sourceName !== name ? <Sub>Listed as {row.sourceName}</Sub> : null}
            {!matched ? <Sub>Not matched to a Tokscale model</Sub> : null}
          </ModelText>
        </ModelCell>
      </RowHead>
      <td data-col="score" role="cell">
        <ScoreText>{row.scoreText}</ScoreText>
        {row.interval ? (
          <Sub>
            {row.interval.kind === "ci" ? (
              <>
                <span aria-hidden="true">CI </span>
                <VisuallyHidden>Confidence interval </VisuallyHidden>
              </>
            ) : (
              <>
                <span aria-hidden="true">s.e. </span>
                <VisuallyHidden>Standard error </VisuallyHidden>
              </>
            )}
            {row.interval.text}
          </Sub>
        ) : null}
        {row.sample ? <Sub>{row.sample}</Sub> : null}
      </td>
      {columns.conditions ? (
        <td data-col="cond" role="cell" data-empty={row.conditions === null && row.note === null ? "true" : undefined}>
          {row.conditions ?? <MutedText>Not reported</MutedText>}
          {row.note ? <Sub>{row.note}</Sub> : null}
        </td>
      ) : null}
      {columns.cost ? (
        <td data-col="cost" role="cell" data-empty={row.cost === null && row.tokens === null ? "true" : undefined}>
          {row.cost ?? (
            <>
              <span aria-hidden="true">{"\u2014"}</span>
              <VisuallyHidden>Cost not reported</VisuallyHidden>
            </>
          )}
          {row.tokens !== null ? (
            <Sub>
              {formatTokens(row.tokens)} tokens{row.basis ? ` ${row.basis}` : ""}
            </Sub>
          ) : null}
        </td>
      ) : null}
      <td data-col="date" role="cell" data-empty={row.date ? undefined : "true"}>
        {row.date ? (
          <>
            {formatIsoDay(row.date.date)}
            <DateSub>{DATE_KIND_LABELS[row.date.kind]}</DateSub>
          </>
        ) : (
          <>
            <span aria-hidden="true">{"\u2014"}</span>
            <VisuallyHidden>No date reported</VisuallyHidden>
          </>
        )}
      </td>
      <UsageCell row={row} usageAvailable={usageAvailable} />
      <td data-col="source" role="cell">
        <SourceLink href={row.sourceUrl} target="_blank" rel="noopener noreferrer">
          Source
          <span aria-hidden="true">{"\u2197"}</span>
          <VisuallyHidden> for {name} (opens in a new tab)</VisuallyHidden>
        </SourceLink>
      </td>
    </tr>
  );
}

type View = "best" | "all";

function ResultsSection({
  info,
  rows,
  columns,
  usageWindow,
}: {
  info: BenchmarkPageInfo;
  rows: ResultRow[];
  columns: ResultColumns;
  usageWindow: { start: string; end: string } | null;
}) {
  const tableId = useId().replace(/:/g, "");
  const best = rows.filter((row) => row.bestRank !== null);
  const canSwitch = best.length < rows.length;
  const [view, setView] = useState<View>("best");
  const [expanded, setExpanded] = useState(false);
  const focusTarget = useRef<string | null>(null);

  const listed = view === "best" && canSwitch ? best : rows;
  const shown = expanded ? listed : listed.slice(0, PAGE_SIZE);
  const usageAvailable = usageWindow !== null;

  useEffect(() => {
    const target = focusTarget.current;
    if (!target) return;
    focusTarget.current = null;
    document.getElementById(target)?.focus();
  }, [expanded]);

  const onView = (next: View) => {
    setView(next);
    setExpanded(false);
  };

  const showAll = () => {
    const next = listed[PAGE_SIZE];
    if (next) focusTarget.current = `${tableId}-r${next.id}`;
    setExpanded(true);
  };

  const noun = view === "best" && canSwitch ? ["model", "models"] : ["result", "results"];

  return (
    <Section aria-labelledby={`${tableId}-title`}>
      <ResultsHead>
        <HeadText>
          <SectionTitle id={`${tableId}-title`}>Results</SectionTitle>
          <SectionNote>
            {view === "best" && canSwitch
              ? "One row per model: its best reported result. "
              : canSwitch
                ? "Every result in the source. A model can appear more than once, with a different reasoning effort or agent. "
                : "Every result in the source. "}
            The rank is the position in this list, by score; the source&apos;s own ranking can differ.
          </SectionNote>
          {usageWindow ? (
            <SectionNote>
              Tokscale usage counts accounts that used the model in public Tokscale submissions, {formatDateRange(usageWindow.start, usageWindow.end)}.
              It is shown next to the score and does not explain it.
            </SectionNote>
          ) : (
            <SectionNote>Tokscale usage could not be loaded, so the usage column is hidden.</SectionNote>
          )}
        </HeadText>
        {canSwitch ? (
          <ButtonSegments
            label="Rows to show"
            value={view}
            onChange={onView}
            options={[
              { value: "best", label: "Best per model", count: best.length },
              { value: "all", label: "All results", count: rows.length },
            ]}
          />
        ) : null}
      </ResultsHead>

      <Table role="table" aria-labelledby={`${tableId}-title`} id={tableId}>
        <thead role="rowgroup">
          <tr role="row">
            <th scope="col" role="columnheader" data-col="rank">
              <span aria-hidden="true">#</span>
              <VisuallyHidden>Rank</VisuallyHidden>
            </th>
            <th scope="col" role="columnheader" data-col="model">
              Model
            </th>
            <th scope="col" role="columnheader" data-col="score">
              Score <MutedText>({info.unitLabel})</MutedText>
            </th>
            {columns.conditions ? (
              <th scope="col" role="columnheader" data-col="cond">
                Conditions
              </th>
            ) : null}
            {columns.cost ? (
              <th scope="col" role="columnheader" data-col="cost">
                Cost and tokens
              </th>
            ) : null}
            <th scope="col" role="columnheader" data-col="date">
              Date
            </th>
            {usageAvailable ? (
              <th scope="col" role="columnheader" data-col="usage">
                Tokscale usage <MutedText>(30 days)</MutedText>
              </th>
            ) : null}
            <th scope="col" role="columnheader" data-col="source">
              <VisuallyHidden>Source link</VisuallyHidden>
              <span aria-hidden="true">Source</span>
            </th>
          </tr>
        </thead>
        <tbody role="rowgroup">
          {shown.map((row) => (
            <ResultRowView
              key={row.id}
              row={row}
              rank={view === "best" && canSwitch ? (row.bestRank ?? row.rank) : row.rank}
              columns={columns}
              usageAvailable={usageAvailable}
              rowHeadId={`${tableId}-r${row.id}`}
            />
          ))}
        </tbody>
      </Table>

      <TableFoot>
        <Status aria-live="polite">
          Showing {formatCount(shown.length)} of {plural(listed.length, noun[0], noun[1])}
        </Status>
        {shown.length < listed.length ? (
          <ShowAll type="button" onClick={showAll} aria-controls={tableId}>
            Show all {plural(listed.length, noun[0], noun[1])}
          </ShowAll>
        ) : null}
      </TableFoot>
    </Section>
  );
}

function MethodSection({ info, mapping }: { info: BenchmarkPageInfo; mapping: MappingSummary }) {
  const methods = [
    mapping.exact > 0 ? `${formatCount(mapping.exact)} by exact id` : null,
    mapping.normalized > 0 ? `${formatCount(mapping.normalized)} by normalized name` : null,
    mapping.manual > 0 ? `${formatCount(mapping.manual)} by a manual rule` : null,
  ].filter(Boolean);
  return (
    <Section aria-labelledby="method-title">
      <SectionTitle id="method-title">Method and matching</SectionTitle>
      <NotesGrid>
        <div>
          <SectionTitle as="h3">How results are run</SectionTitle>
          <NoteList>
            <li>{info.conditions}</li>
            {info.methodNotes.map((note) => (
              <li key={note}>{note}</li>
            ))}
          </NoteList>
        </div>
        <div>
          <SectionTitle as="h3">Matching to Tokscale models</SectionTitle>
          <NoteList>
            <li>
              {formatCount(mapping.matched)} of {plural(mapping.results, "result", "results")} are matched to{" "}
              {plural(mapping.models, "Tokscale model", "Tokscale models")}
              {methods.length > 0 ? `: ${methods.join(", ")}` : ""}.
            </li>
            <li>
              {mapping.reviewed === 0
                ? "No match is marked as reviewed by hand yet."
                : `${formatCount(mapping.reviewed)} of the matches are reviewed by hand.`}
            </li>
            {mapping.unmatched > 0 ? (
              <li>
                {plural(mapping.unmatched, "result", "results")} from {plural(mapping.unmatchedModels, "model", "models")}{" "}
                {mapping.unmatched === 1 ? "is" : "are"} not matched. They stay in the table without a Tokscale link.
              </li>
            ) : null}
            <li>Reasoning effort, thinking budget and agent are conditions of a result. They never change which model a result belongs to.</li>
          </NoteList>
        </div>
        <div>
          <SectionTitle as="h3">License and attribution</SectionTitle>
          <NoteList>
            <li>
              {info.licenseUrl ? (
                <ExternalLink href={info.licenseUrl} target="_blank" rel="noopener noreferrer">
                  {info.licenseName}
                  <NewTab />
                </ExternalLink>
              ) : (
                info.licenseName
              )}
              {info.retrievedAt ? `, retrieved ${formatIsoDay(info.retrievedAt)}.` : "."}
            </li>
            <li>{info.attribution}</li>
          </NoteList>
        </div>
      </NotesGrid>
    </Section>
  );
}

interface BenchmarkDetailViewProps {
  info: BenchmarkPageInfo;
  rows: ResultRow[];
  columns: ResultColumns;
  timeline: TimelineData;
  mapping: MappingSummary;
  /** The 30-day usage window, or null when Tokscale usage could not be loaded. */
  usageWindow: { start: string; end: string } | null;
  sources: SourceNote[];
}

export function BenchmarkDetailView({ info, rows, columns, timeline, mapping, usageWindow, sources }: BenchmarkDetailViewProps) {
  return (
    <>
      <Header info={info} />
      <Sections>
        <ResultsSection info={info} rows={rows} columns={columns} usageWindow={usageWindow} />
        <Section aria-label="Score by release date">
          <ScoreTimeline
            timeline={timeline}
            unit={info.unit}
            unitLabel={info.unitLabel}
            benchmarkName={info.name}
            sourceName={info.sourceName}
            usageAvailable={usageWindow !== null}
          />
        </Section>
        <MethodSection info={info} mapping={mapping} />
        <SourcesFooter notes={sources} />
      </Sections>
    </>
  );
}

export function BenchmarkNotFound() {
  return (
    <NotFoundWrap>
      <PageTitle>Benchmark not found</PageTitle>
      <PageLead>
        Tokscale has no results for this benchmark. Some benchmarks are linked to their own site instead of being copied here.
      </PageLead>
      <PrimaryActionLink href={benchmarksPath()}>See all benchmarks</PrimaryActionLink>
    </NotFoundWrap>
  );
}
