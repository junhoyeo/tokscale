"use client";

/**
 * /benchmarks: every tracked benchmark, grouped by category. Each row says
 * what the benchmark measures, who publishes it, its unit, how many models
 * it covers, when it was last updated and its license. Thin rules, no cards.
 */
import Link from "next/link";
import styled from "styled-components";
import type { SourceNote } from "@/lib/benchmarks/attribution";
import type { BenchmarkIndexGroup, BenchmarkIndexItem, BenchmarkIndexSummary, DateKind } from "@/lib/benchmarks/detail";
import { formatIsoDay } from "@/lib/benchmarks/format";
import { VisuallyHidden } from "./ExploreUI";
import { SourcesFooter } from "./SourcesFooter";

const SummaryLine = styled.p`
  display: flex;
  flex-wrap: wrap;
  gap: 2px 0;
  margin: 0 0 32px;
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
  gap: 40px;
`;

const GroupHead = styled.div`
  display: flex;
  flex-wrap: wrap;
  align-items: baseline;
  justify-content: space-between;
  gap: 4px 16px;
  padding-bottom: 10px;
  border-bottom: 1px solid var(--service-border-strong);
`;

const GroupTitle = styled.h2`
  margin: 0;
  color: var(--service-text);
  font-size: 1rem;
  font-weight: 600;
  letter-spacing: -0.01em;
`;

const GroupCount = styled.span`
  color: var(--service-text-muted);
  font-size: 0.8125rem;
  font-variant-numeric: tabular-nums;
`;

const List = styled.ul`
  margin: 0;
  padding: 0;
  list-style: none;
`;

const Item = styled.li`
  display: grid;
  grid-template-columns: minmax(0, 1fr);
  gap: 14px 40px;
  padding: 18px 0 20px;
  border-bottom: 1px solid var(--service-border);

  @media (min-width: 1100px) {
    grid-template-columns: minmax(0, 1fr) minmax(0, 620px);
    align-items: start;
  }
`;

const Main = styled.div`
  display: grid;
  align-content: start;
  gap: 6px;
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
  font-size: 1rem;
  font-weight: 600;
  letter-spacing: -0.01em;
  overflow-wrap: anywhere;

  @media (max-width: 640px) {
    font-size: 1.0625rem;
  }
`;

const NameLink = styled(Link)`
  ${nameStyles}
  text-decoration: none;

  &:hover {
    color: var(--service-accent-hover);
  }

  &:focus-visible {
    border-radius: 4px;
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
  align-items: center;
  padding: 0 8px;
  border: 1px solid var(--service-border-strong);
  border-radius: 999px;
  color: var(--service-text-muted);
  font-size: 0.6875rem;
  font-weight: 500;
  white-space: nowrap;
`;

const Measures = styled.p`
  max-width: 72ch;
  margin: 0;
  color: var(--service-text-muted);
  font-size: 0.9375rem;
  line-height: 1.55;
  text-wrap: pretty;

  @media (max-width: 640px) {
    font-size: 1rem;
  }
`;

const Byline = styled.p`
  margin: 0;
  color: var(--service-text-muted);
  font-size: 0.8125rem;

  @media (max-width: 640px) {
    font-size: 0.875rem;
  }
`;

const Note = styled.p`
  max-width: 72ch;
  margin: 4px 0 0;
  padding-left: 10px;
  border-left: 2px solid var(--service-border-strong);
  color: var(--service-text-muted);
  font-size: 0.8125rem;
  line-height: 1.5;

  @media (max-width: 640px) {
    font-size: 0.875rem;
  }
`;

const ExternalLink = styled.a`
  justify-self: start;
  color: var(--service-accent-hover);
  font-size: 0.875rem;
  text-decoration: none;

  &:hover {
    text-decoration: underline !important;
  }

  &:focus-visible {
    border-radius: 4px;
    outline: 2px solid var(--service-focus);
    outline-offset: 2px;
  }

  @media (max-width: 640px), (pointer: coarse) {
    display: inline-flex;
    min-height: 44px;
    align-items: center;
  }
`;

const Facts = styled.dl`
  display: grid;
  grid-template-columns: repeat(2, minmax(0, 1fr));
  gap: 14px 20px;
  margin: 0;

  @media (min-width: 720px) {
    grid-template-columns: minmax(0, 0.8fr) minmax(0, 0.9fr) minmax(0, 1.1fr) minmax(0, 1.2fr);
  }
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
    font-size: 0.875rem;
    font-variant-numeric: tabular-nums;
    overflow-wrap: anywhere;
  }

  @media (max-width: 640px) {
    dt {
      font-size: 0.8125rem;
    }

    dd {
      font-size: 0.9375rem;
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

const LicenseLink = styled.a`
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

const Footer = styled.div`
  margin-top: 48px;
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

function NewTab() {
  return <VisuallyHidden> (opens in a new tab)</VisuallyHidden>;
}

function BenchmarkRow({ item }: { item: BenchmarkIndexItem }) {
  const linkOnly = item.status === "link-only";
  return (
    <Item data-row-link={item.href ? "" : undefined}>
      <Main>
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
        <Byline>
          By {item.publisher}
          {item.collectedBy ? ` \u00b7 collected by ${item.collectedBy}` : null}
        </Byline>
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
      </Main>
      <Facts>
        <Fact>
          <dt>Unit</dt>
          <dd>{item.unitLabel}</dd>
        </Fact>
        <Fact>
          <dt>Models</dt>
          {linkOnly ? (
            <dd>
              Not republished
              <FactSub>Results stay on the source site</FactSub>
            </dd>
          ) : (
            <dd>
              {item.modelCount} matched
              <FactSub>{item.externalModelCount} in the source</FactSub>
            </dd>
          )}
        </Fact>
        <Fact>
          <dt>Last update</dt>
          {item.lastUpdate ? (
            <dd>
              {formatIsoDay(item.lastUpdate.date)}
              <FactSub>{DATE_KIND_TEXT[item.lastUpdate.kind]}</FactSub>
              {item.retrievedAt ? <FactSub>Retrieved {formatIsoDay(item.retrievedAt)}</FactSub> : null}
            </dd>
          ) : (
            <dd>
              {"\u2014"}
              <FactSub>{linkOnly ? "See the leaderboard" : "No dated results"}</FactSub>
            </dd>
          )}
        </Fact>
        <Fact>
          <dt>License</dt>
          <dd>
            {item.licenseUrl ? (
              <LicenseLink href={item.licenseUrl} target="_blank" rel="noopener noreferrer">
                {item.licenseName}
                <NewTab />
              </LicenseLink>
            ) : (
              item.licenseName
            )}
          </dd>
        </Fact>
      </Facts>
    </Item>
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
            <section key={group.category} aria-labelledby={titleId}>
              <GroupHead>
                <GroupTitle id={titleId}>{group.label}</GroupTitle>
                <GroupCount>
                  {group.items.length} {group.items.length === 1 ? "benchmark" : "benchmarks"}
                </GroupCount>
              </GroupHead>
              <List>
                {group.items.map((item) => (
                  <BenchmarkRow key={item.slug} item={item} />
                ))}
              </List>
            </section>
          );
        })}
      </Groups>
      <Footer>
        <SourcesFooter notes={sources} />
      </Footer>
    </>
  );
}
