"use client";

/**
 * /benchmarks: every tracked benchmark, grouped by category. Each group is a
 * real table: the benchmark on the left, then its source, matched models,
 * last update, unit and license on the right, numbers right-aligned in
 * tabular figures. Thin rules, no cards, 13px grey sentence-case headers.
 * Below 900px each row stacks: name and measures, then the same facts as
 * labelled pairs.
 */
import Link from "next/link";
import styled from "styled-components";
import type { SourceNote } from "@/lib/benchmarks/attribution";
import type { BenchmarkIndexGroup, BenchmarkIndexItem, BenchmarkIndexSummary, DateKind } from "@/lib/benchmarks/detail";
import { formatIsoDay } from "@/lib/benchmarks/format";
import { VisuallyHidden } from "./ExploreUI";
import { SourcesFooter } from "./SourcesFooter";

/** Below this width each row stacks; above it the group is a six-column table. */
const STACK_BELOW = "899px";

const SummaryLine = styled.p`
  display: flex;
  flex-wrap: wrap;
  gap: 2px 0;
  margin: 0 0 16px;
  color: var(--service-text-muted);
  font-size: 0.8125rem;
  line-height: 1.5;
  font-variant-numeric: tabular-nums;

  > span:not(:last-child)::after {
    content: "·";
    margin: 0 8px;
    color: var(--service-border-strong);
  }

  @media (max-width: 640px) {
    font-size: 0.875rem;
  }
`;

const Groups = styled.div`
  display: grid;
  gap: 28px;
`;

const Group = styled.section`
  display: grid;
  align-content: start;
  gap: 2px;
  min-width: 0;
`;

const GroupHead = styled.div`
  display: flex;
  flex-wrap: wrap;
  align-items: baseline;
  justify-content: space-between;
  gap: 2px 16px;
`;

const GroupTitle = styled.h2`
  margin: 0;
  color: var(--service-text);
  font-size: 1rem;
  font-weight: 600;
  letter-spacing: -0.01em;
`;

const GroupCount = styled.p`
  margin: 0;
  color: var(--service-text-muted);
  font-size: 0.8125rem;
  font-variant-numeric: tabular-nums;

  @media (max-width: 640px) {
    font-size: 0.875rem;
  }
`;

const Table = styled.table`
  width: 100%;
  margin-top: 2px;
  border-collapse: collapse;
  font-size: 0.875rem;
  font-variant-numeric: tabular-nums;

  thead tr {
    border-bottom: 1px solid var(--service-border-strong);
  }

  th,
  td {
    padding: 8px 20px 10px 0;
    border-bottom: 1px solid var(--service-border);
    text-align: left;
    vertical-align: top;
  }

  th:last-child,
  td:last-child {
    padding-right: 0;
  }

  thead th {
    padding-top: 4px;
    padding-bottom: 6px;
    border-bottom: 0;
    color: var(--service-text-muted);
    font-size: 0.8125rem;
    font-weight: 400;
    vertical-align: bottom;
    white-space: nowrap;
  }

  th[data-col="source"],
  th[data-col="models"],
  th[data-col="update"],
  td[data-col="source"],
  td[data-col="models"],
  td[data-col="update"] {
    text-align: right;
  }

  th[data-col="benchmark"] {
    width: 46%;
  }

  /* The fact columns hold short values; nowrap gives each its min-content
     width and the benchmark column absorbs the rest. */
  td[data-col="source"],
  td[data-col="models"],
  td[data-col="update"],
  td[data-col="unit"],
  td[data-col="license"] {
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
      grid-template-columns: repeat(2, minmax(0, 1fr));
      gap: 10px 20px;
      padding: 14px 0 16px;
      border-bottom: 1px solid var(--service-border);
    }

    th,
    td {
      display: block;
      min-width: 0;
      padding: 0;
      border: 0;
      text-align: left;
      white-space: normal;
    }

    th[data-col="benchmark"] {
      grid-column: 1 / -1;
      width: auto;
    }

    td[data-col="source"],
    td[data-col="models"],
    td[data-col="update"] {
      text-align: left;
    }
  }

  @media (max-width: 640px) {
    font-size: 0.9375rem;

    tbody tr {
      grid-template-columns: minmax(0, 1fr);
    }
  }
`;

/** The stacked layout's own label: the header row is hidden there. */
const CellLabel = styled.span`
  display: none;

  @media (max-width: ${STACK_BELOW}) {
    display: block;
    margin-bottom: 2px;
    color: var(--service-text-muted);
    font-size: 0.8125rem;
    font-weight: 400;
  }
`;

const Identity = styled.div`
  display: grid;
  align-content: start;
  gap: 3px;
  min-width: 0;
`;

const NameRow = styled.div`
  display: flex;
  flex-wrap: wrap;
  align-items: center;
  gap: 4px 10px;
`;

const nameStyles = `
  color: var(--service-text);
  font-size: 0.9375rem;
  font-weight: 500;
  overflow-wrap: anywhere;
`;

const NameLink = styled(Link)`
  ${nameStyles}
  text-decoration: none;

  &:hover {
    text-decoration: underline;
    text-underline-offset: 3px;
  }

  &:focus-visible {
    border-radius: 2px;
    outline: 2px solid var(--service-focus);
    outline-offset: 2px;
  }

  @media (max-width: 640px), (pointer: coarse) {
    display: inline-flex;
    min-height: 44px;
    align-items: center;
  }
`;

const NameText = styled.span`
  ${nameStyles}
`;

const StatusTag = styled.span`
  display: inline-flex;
  min-height: 20px;
  flex: none;
  align-items: center;
  padding: 0 6px;
  border: 1px solid var(--service-border-strong);
  border-radius: 2px;
  color: var(--service-text-muted);
  font-size: 0.75rem;
  font-weight: 400;
  line-height: 1.2;
  white-space: nowrap;
`;

const Measures = styled.p`
  max-width: 64ch;
  margin: 0;
  color: var(--service-text-muted);
  font-size: 0.8125rem;
  line-height: 1.5;
  text-wrap: pretty;

  @media (max-width: 640px) {
    font-size: 0.875rem;
  }
`;

const Note = styled.p`
  max-width: 64ch;
  margin: 0;
  color: var(--service-text-muted);
  font-size: 0.8125rem;
  line-height: 1.5;

  @media (max-width: 640px) {
    font-size: 0.875rem;
  }
`;

/** The row's one action for link-only benchmarks: ink text, underline on hover. */
const ExternalLink = styled.a`
  justify-self: start;
  color: var(--service-text);
  font-size: 0.875rem;
  font-weight: 500;
  text-decoration: none;

  &:hover {
    text-decoration: underline;
    text-underline-offset: 3px;
  }

  &:focus-visible {
    border-radius: 2px;
    outline: 2px solid var(--service-focus);
    outline-offset: 2px;
  }

  @media (max-width: 640px), (pointer: coarse) {
    display: inline-flex;
    min-height: 44px;
    align-items: center;
  }
`;

const CellStack = styled.span`
  display: grid;
  gap: 2px;
  min-width: 0;
`;

const CellSub = styled.span`
  display: block;
  color: var(--service-text-muted);
  font-size: 0.8125rem;
  line-height: 1.45;
`;

const LicenseLink = styled.a`
  color: var(--service-text);
  text-decoration: none;

  &:hover {
    text-decoration: underline;
    text-underline-offset: 3px;
  }

  &:focus-visible {
    border-radius: 2px;
    outline: 2px solid var(--service-focus);
    outline-offset: 2px;
  }
`;

const Footer = styled.div`
  margin-top: 40px;
`;

const DATE_KIND_TEXT: Record<DateKind, string> = {
  run: "Newest run date",
  published: "Newest publish date",
  release: "Newest model release",
};

function retrievalText(summary: BenchmarkIndexSummary): string | null {
  const { retrievedFrom, retrievedTo } = summary;
  if (!retrievedFrom || !retrievedTo) return null;
  if (retrievedFrom === retrievedTo) return `Retrieved ${formatIsoDay(retrievedTo)}`;
  return `Retrieved ${formatIsoDay(retrievedFrom)} to ${formatIsoDay(retrievedTo)}`;
}

function countText(n: number, one: string, many: string): string {
  return `${n} ${n === 1 ? one : many}`;
}

function NewTab() {
  return <VisuallyHidden> (opens in a new tab)</VisuallyHidden>;
}

function BenchmarkRow({ item }: { item: BenchmarkIndexItem }) {
  const linkOnly = item.status === "link-only";
  return (
    <tr data-row-link={item.href ? "" : undefined}>
      <th scope="row" data-col="benchmark">
        <Identity>
          <NameRow>
            {item.href ? (
              <NameLink href={item.href} data-row-primary="">
                {item.name}
              </NameLink>
            ) : (
              <NameText>{item.name}</NameText>
            )}
            {item.status === "history" ? <StatusTag>History</StatusTag> : null}
            {linkOnly ? <StatusTag>Link only</StatusTag> : null}
          </NameRow>
          <Measures>{item.measures}</Measures>
          {item.notes.map((note) => (
            <Note key={note}>{note}</Note>
          ))}
          {linkOnly ? (
            <ExternalLink href={item.url} target="_blank" rel="noopener noreferrer">
              Open the {item.name} leaderboard
              <span aria-hidden="true"> {"\u2197"}</span>
              <NewTab />
            </ExternalLink>
          ) : null}
        </Identity>
      </th>
      <td data-col="source">
        <CellLabel aria-hidden="true">Source</CellLabel>
        <CellStack>
          By {item.publisher}
          {item.collectedBy ? <CellSub>Collected by {item.collectedBy}</CellSub> : null}
        </CellStack>
      </td>
      <td data-col="models">
        <CellLabel aria-hidden="true">Models</CellLabel>
        {linkOnly ? (
          <CellStack>
            Not republished
            <CellSub>Results stay on the source site</CellSub>
          </CellStack>
        ) : (
          <CellStack>
            {item.modelCount} matched
            <CellSub>{item.externalModelCount} in the source</CellSub>
          </CellStack>
        )}
      </td>
      <td data-col="update">
        <CellLabel aria-hidden="true">Last update</CellLabel>
        {item.lastUpdate ? (
          <CellStack>
            {formatIsoDay(item.lastUpdate.date)}
            <CellSub>{DATE_KIND_TEXT[item.lastUpdate.kind]}</CellSub>
            {item.retrievedAt ? <CellSub>Retrieved {formatIsoDay(item.retrievedAt)}</CellSub> : null}
          </CellStack>
        ) : (
          <CellStack>
            {"\u2014"}
            <CellSub>{linkOnly ? "See the leaderboard" : "No dated results"}</CellSub>
          </CellStack>
        )}
      </td>
      <td data-col="unit">
        <CellLabel aria-hidden="true">Unit</CellLabel>
        {item.unitLabel}
      </td>
      <td data-col="license">
        <CellLabel aria-hidden="true">License</CellLabel>
        {item.licenseUrl ? (
          <LicenseLink href={item.licenseUrl} target="_blank" rel="noopener noreferrer">
            {item.licenseName}
            <NewTab />
          </LicenseLink>
        ) : (
          item.licenseName
        )}
      </td>
    </tr>
  );
}

export function BenchmarkIndex({
  groups,
  summary,
  sources,
}: {
  groups: BenchmarkIndexGroup[];
  summary: BenchmarkIndexSummary;
  sources: SourceNote[];
}) {
  const retrieved = retrievalText(summary);
  return (
    <>
      <SummaryLine>
        <span>
          {summary.withResults} benchmarks with results
          {summary.linkOnly > 0 ? `, ${summary.linkOnly} linked only` : ""}
        </span>
        {summary.sources.length > 0 ? <span>From {summary.sources.join(", ")}</span> : null}
        {retrieved ? <span>{retrieved}</span> : null}
      </SummaryLine>
      <Groups>
        {groups.map((group) => {
          const titleId = `benchmarks-${group.category}`;
          return (
            <Group key={group.category} aria-labelledby={titleId}>
              <GroupHead>
                <GroupTitle id={titleId}>{group.label}</GroupTitle>
                <GroupCount>{countText(group.items.length, "benchmark", "benchmarks")}</GroupCount>
              </GroupHead>
              <Table>
                <caption>
                  <VisuallyHidden>
                    {group.label}: {countText(group.items.length, "benchmark", "benchmarks")}, with their source, matched
                    models, last update, unit and license
                  </VisuallyHidden>
                </caption>
                <thead>
                  <tr>
                    <th scope="col" data-col="benchmark">
                      Benchmark
                    </th>
                    <th scope="col" data-col="source">
                      Source
                    </th>
                    <th scope="col" data-col="models">
                      Models
                    </th>
                    <th scope="col" data-col="update">
                      Last update
                    </th>
                    <th scope="col" data-col="unit">
                      Unit
                    </th>
                    <th scope="col" data-col="license">
                      License
                    </th>
                  </tr>
                </thead>
                <tbody>
                  {group.items.map((item) => (
                    <BenchmarkRow key={item.slug} item={item} />
                  ))}
                </tbody>
              </Table>
            </Group>
          );
        })}
      </Groups>
      <Footer>
        <SourcesFooter notes={sources} />
      </Footer>
    </>
  );
}
