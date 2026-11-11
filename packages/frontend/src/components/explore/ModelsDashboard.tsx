"use client";

/**
 * /models dashboard: one row per model with usage, benchmark scores, price
 * estimates and specs. Column groups and sorting live in the URL
 * (?cols=, ?sort=) through history.replaceState; the period is a link.
 *
 * Wide screens get a real table that scrolls inside its own box when the
 * selected groups do not fit; narrow screens get one card per model.
 */
import { useEffect, useMemo, useRef, useState, type ReactNode } from "react";
import Link from "next/link";
import styled from "styled-components";
import type { BenchmarkInfo } from "@/lib/benchmarks/attribution";
import {
  COLUMN_GROUP_LABELS,
  COLUMN_GROUPS,
  GROUP_BENCHMARKS,
  dashboardQuery,
  effectiveSortKey,
  sortAscending,
  toggleColumnGroup,
  type ColumnGroup,
} from "@/lib/benchmarks/columns";
import { sortDashboardRows, type DashboardRow } from "@/lib/benchmarks/dashboard";
import { mixShares, type MixCounts } from "@/lib/benchmarks/price";
import type { Coverage, ExplorePeriod } from "@/lib/explore";
import { modelPath } from "@/lib/seo/explore";
import {
  CacheReadCell,
  ContextCell,
  DateCell,
  EstimateCell,
  ListPriceCell,
  OpenWeightsCell,
  TokensCell,
  TopAgentCell,
  UsersCell,
} from "./DashboardCells";
import { ControlBar, CoverageLine, InlineLink, LinkSegments, ModelMark, TouchSegment, VisuallyHidden } from "./ExploreUI";
import { DEFAULT_EXPLORE_PERIOD, EXPLORE_PERIODS, formatCount, formatShare, periodShortLabel } from "./format";
import { ScoreValue } from "./ScoreValue";
import { SegmentedGroup } from "@/components/leaderboard/RankingUI";
import { InstantTooltip } from "@/components/ui/InstantTooltip";
import { Select } from "@/components/ui/Select";

interface Column {
  id: string;
  group: ColumnGroup | "overview";
  label: string;
  /** Short explanation for the header tooltip and the card label. */
  title: string;
  sortable: boolean;
  width: number;
  render: (row: DashboardRow, align: "start" | "end") => ReactNode;
}

export interface ModelsDashboardProps {
  rows: DashboardRow[];
  coverage: Coverage;
  period: ExplorePeriod;
  initialGroups: ColumnGroup[];
  initialSort: string;
  benchmarks: Record<string, BenchmarkInfo>;
  /** Community token mix (counts) in the period, or null without usage data. */
  communityMix: MixCounts | null;
  /** Each listed model's own token mix (counts) in the period. */
  ownMixes: Record<string, MixCounts>;
  minUsers: number;
  topEci: number;
}

const Controls = styled(ControlBar)`
  align-items: flex-start;
  margin-bottom: 12px;
`;

const ChipRow = styled.div`
  display: flex;
  flex-wrap: wrap;
  align-items: center;
  gap: 8px 12px;
  margin-bottom: 12px;
`;

const ChipLabel = styled.span`
  color: var(--service-text-muted);
  font-size: 0.8125rem;
  font-weight: 500;
`;

const Chips = styled(SegmentedGroup)`
  flex-wrap: wrap;
  height: auto;
`;

const Meta = styled.div`
  display: flex;
  flex-wrap: wrap;
  align-items: baseline;
  justify-content: space-between;
  gap: 4px 16px;
  margin: 12px 0 8px;
  color: var(--service-text-muted);
  font-size: 0.8125rem;

  @media (max-width: 640px) {
    font-size: 0.875rem;
  }
`;

const ScrollBox = styled.div`
  position: relative;
  display: none;
  border-top: 1px solid var(--service-border);

  @media (min-width: 960px) {
    display: block;
  }
`;

const ScrollArea = styled.div`
  position: relative;
`;

/* Positioned, so absolutely positioned cell content (screen-reader text) is clipped and scrolls with the table. */
const Scroller = styled.div`
  position: relative;
  overflow-x: auto;
  overscroll-behavior-x: contain;

  &:focus-visible {
    outline: 2px solid var(--service-focus);
    outline-offset: 2px;
  }
`;

/* Above the row covers and the controls raised over them, like the sticky model column. */
const Fade = styled.div<{ $visible: boolean }>`
  position: absolute;
  z-index: 3;
  top: 0;
  right: 0;
  bottom: 0;
  width: 48px;
  background: linear-gradient(to right, transparent, var(--service-canvas));
  opacity: ${({ $visible }) => ($visible ? 1 : 0)};
  pointer-events: none;
`;

const ScrollHint = styled.p`
  margin: 6px 0;
  color: var(--service-text-muted);
  font-size: 0.75rem;
  text-align: right;
`;

const Table = styled.table`
  width: 100%;
  min-width: max-content;
  border-collapse: separate;
  border-spacing: 0;
  font-size: 0.875rem;
  font-variant-numeric: tabular-nums;

  th,
  td {
    padding: 10px 12px;
    border-bottom: 1px solid var(--service-border);
    text-align: right;
    vertical-align: middle;
    white-space: nowrap;
  }

  thead th {
    color: var(--service-text-muted);
    font-size: 0.75rem;
    font-weight: 500;
  }

  thead tr.groups th {
    padding-top: 12px;
    padding-bottom: 4px;
    border-bottom: 0;
    color: var(--service-text);
    font-size: 0.75rem;
    font-weight: 600;
    text-align: left;
  }

  th[data-group-start="true"],
  td[data-group-start="true"] {
    border-left: 1px solid var(--service-border);
  }

  /*
   * Row links (globals.css). The model cell is sticky, and a sticky cell is the
   * containing block of its link's ::after, so that link covers only the cell;
   * a data-row-cover link in the first data cell covers the rest of the row.
   * The sticky cell stays above the cover (1) and the controls raised over it
   * (2), and keeps an opaque background so scrolled cells do not show through.
   */
  .model {
    position: sticky;
    left: 0;
    z-index: 3;
    min-width: 220px;
    max-width: 280px;
    background: var(--service-canvas);
    text-align: left;
  }

  tbody tr:hover .model,
  tbody tr:has([data-row-primary]:focus-visible) .model {
    background: linear-gradient(var(--v2-row-hover), var(--v2-row-hover)), var(--service-canvas);
  }

  /* The focus ring is drawn on the cells, so the sticky cell cannot hide part of it. */
  tbody tr [data-row-primary]:focus-visible::after {
    outline: none;
  }

  tbody tr:has([data-row-primary]:focus-visible) > * {
    box-shadow:
      inset 0 2px 0 var(--v2-focus),
      inset 0 -2px 0 var(--v2-focus);
  }

  tbody tr:has([data-row-primary]:focus-visible) > :first-child {
    box-shadow:
      inset 2px 0 0 var(--v2-focus),
      inset 0 2px 0 var(--v2-focus),
      inset 0 -2px 0 var(--v2-focus);
  }

  tbody tr:has([data-row-primary]:focus-visible) > :last-child {
    box-shadow:
      inset -2px 0 0 var(--v2-focus),
      inset 0 2px 0 var(--v2-focus),
      inset 0 -2px 0 var(--v2-focus);
  }
`;

const SortButton = styled.button<{ $active: boolean }>`
  display: inline-flex;
  align-items: center;
  gap: 4px;
  padding: 2px 0;
  border: 0;
  background: transparent;
  color: ${({ $active }) => ($active ? "var(--service-text)" : "var(--service-text-muted)")};
  font: inherit;
  font-weight: ${({ $active }) => ($active ? 600 : 500)};
  white-space: nowrap;

  &:hover {
    color: var(--service-text);
  }

  &:focus-visible {
    border-radius: 4px;
    outline: 2px solid var(--service-focus);
    outline-offset: 2px;
  }
`;

/* A column header without sorting. It takes focus, so keyboard users can open its explanation. */
const HeaderTip = styled(InstantTooltip)`
  &:focus-visible {
    border-radius: 4px;
    outline: 2px solid var(--service-focus);
    outline-offset: 2px;
  }
`;

const ModelCell = styled.div`
  display: grid;
  grid-template-columns: auto minmax(0, 1fr);
  align-items: center;
  gap: 0 10px;

  > :first-child {
    grid-row: span 2;
  }
`;

/* Mouse-only cover over the rest of a table row; see the table styles. */
const RowCover = styled(Link)``;

const ModelLink = styled(Link)`
  overflow: hidden;
  color: var(--service-text);
  font-weight: 600;
  text-decoration: none;
  text-overflow: ellipsis;

  &:hover {
    color: var(--service-accent-hover);
  }

  &:focus-visible {
    border-radius: 4px;
    outline: 2px solid var(--service-focus);
    outline-offset: 2px;
  }
`;

const LabText = styled.span`
  overflow: hidden;
  color: var(--service-text-muted);
  font-size: 0.75rem;
  text-overflow: ellipsis;
`;

const Cards = styled.ol`
  margin: 0;
  padding: 0;
  border-top: 1px solid var(--service-border);
  list-style: none;

  @media (min-width: 960px) {
    display: none;
  }
`;

const Card = styled.li`
  display: grid;
  gap: 10px;
  padding: 14px 0;
  border-bottom: 1px solid var(--service-border);
`;

const CardHead = styled.div`
  display: grid;
  grid-template-columns: auto auto minmax(0, 1fr);
  align-items: center;
  gap: 0 10px;

  > span:first-child {
    color: var(--service-text-muted);
    font-size: 0.8125rem;
    font-variant-numeric: tabular-nums;
  }
`;

const CardName = styled.div`
  display: grid;
  min-width: 0;

  ${ModelLink} {
    font-size: 1rem;
  }

  @media (pointer: coarse) {
    ${ModelLink} {
      display: inline-flex;
      min-height: 44px;
      align-items: center;
    }
  }
`;

const CardFacts = styled.dl`
  display: grid;
  grid-template-columns: repeat(auto-fill, minmax(min(100%, 150px), 1fr));
  gap: 10px 16px;
  margin: 0;
  font-size: 0.875rem;
  font-variant-numeric: tabular-nums;

  > div {
    display: grid;
    align-content: start;
    gap: 2px;
    min-width: 0;
  }

  dt {
    overflow: hidden;
    color: var(--service-text-muted);
    font-size: 0.75rem;
    text-overflow: ellipsis;
    white-space: nowrap;
  }

  dd {
    min-width: 0;
    margin: 0;
  }
`;

/** Phones show this many cards, and wide screens this many rows, before "Show all". */
const CARD_LIMIT = 20;
const ROW_LIMIT = 60;

const TableMore = styled.button`
  display: none;
  min-height: 36px;
  margin-top: 12px;
  padding: 0 14px;
  border: 1px solid var(--service-border-strong);
  border-radius: 8px;
  background: transparent;
  color: var(--service-text);
  font-size: 0.875rem;

  &:focus-visible {
    outline: 2px solid var(--service-focus);
    outline-offset: 2px;
  }

  @media (min-width: 960px) {
    display: inline-flex;
    align-items: center;
  }
`;

const MoreButton = styled.button`
  width: 100%;
  min-height: 44px;
  margin-top: 12px;
  border: 1px solid var(--service-border-strong);
  border-radius: 8px;
  background: transparent;
  color: var(--service-text);
  font-size: 1rem;

  &:focus-visible {
    outline: 2px solid var(--service-focus);
    outline-offset: 2px;
  }

  @media (min-width: 960px) {
    display: none;
  }
`;

const SortSelect = styled.div`
  --select-height: 44px;
  --select-font-size: 1rem;
  --select-padding-x: 10px;
  --select-background: var(--service-surface);
  --select-border-color: var(--service-border-strong);

  display: inline-flex;
  min-width: 0;
  max-width: 100%;
  align-items: center;
  gap: 8px;
  color: var(--service-text-muted);
  font-size: 0.875rem;

  label {
    flex: 0 0 auto;
  }

  @media (min-width: 960px) {
    display: none;
  }
`;

const Footnote = styled.p`
  max-width: 90ch;
  margin: 12px 0 0;
  color: var(--service-text-muted);
  font-size: 0.8125rem;
  line-height: 1.55;

  @media (max-width: 640px) {
    font-size: 0.875rem;
  }
`;

function scoreColumn(info: BenchmarkInfo, group: ColumnGroup | "overview"): Column {
  return {
    id: info.slug,
    group,
    label: info.shortName,
    title: `${info.name} (${info.unitLabel}). ${info.measures}`,
    sortable: true,
    width: 104,
    render: (row, align) => <ScoreValue cell={row.scores[info.slug] ?? null} info={info} modelName={row.name} align={align} />,
  };
}

function buildColumns(
  groups: readonly ColumnGroup[],
  period: ExplorePeriod,
  benchmarks: Record<string, BenchmarkInfo>,
  communityMix: MixCounts | null,
  ownMixes: Record<string, MixCounts>,
): Column[] {
  const users: Column = {
    id: "users",
    group: "usage",
    label: "Active users",
    title: "Distinct visible accounts that used the model in the period, with the change against the previous period.",
    sortable: true,
    width: 104,
    render: (row, align) => <UsersCell row={row} period={period} align={align} />,
  };
  const agent: Column = {
    id: "agent",
    group: "usage",
    label: "Top agent",
    title: "The agent that carried most of the model's tokens in the period, and its share.",
    sortable: false,
    width: 150,
    render: (row) => <TopAgentCell row={row} />,
  };
  const community: Column = {
    id: "price-community",
    group: "price",
    label: "$/1M at agent mix",
    title: `Estimate at list prices: USD per 1M tokens at the community's token mix in the period${communityShareText(communityMix)}.`,
    sortable: true,
    width: 120,
    render: (row, align) => (
      <EstimateCell row={row} which="community" communityMix={communityMix} ownMix={ownMixes[row.key] ?? null} align={align} />
    ),
  };
  const score = (slug: string, group: ColumnGroup | "overview") => {
    const info = benchmarks[slug];
    return info ? [scoreColumn(info, group)] : [];
  };

  if (groups.length === 0) {
    return [
      { ...users, group: "overview" },
      { ...agent, group: "overview" },
      ...score("epoch-capabilities-index", "overview"),
      ...score("terminal-bench-4", "overview"),
      ...score("webdev-arena", "overview"),
      { ...community, group: "overview" },
    ];
  }

  const columns: Column[] = [];
  for (const group of groups) {
    if (group === "usage") {
      columns.push(users, {
        id: "tokens",
        group,
        label: "Tokens",
        title: "Tokens on the model in the period, cache included.",
        sortable: true,
        width: 88,
        render: (row) => <TokensCell row={row} />,
      }, agent);
    } else if (group === "capability" || group === "coding" || group === "webdev") {
      for (const slug of GROUP_BENCHMARKS[group]) columns.push(...score(slug, group));
    } else if (group === "price") {
      columns.push(
        {
          id: "price-input",
          group,
          label: "List in / out",
          title: "List price per 1M input and output tokens, from the lab's own API when models.dev lists it (models.dev).",
          sortable: true,
          width: 120,
          render: (row, align) => <ListPriceCell price={row.price} align={align} />,
        },
        {
          id: "cache-read",
          group,
          label: "Cache read",
          title: "List price per 1M cache-read tokens (models.dev).",
          sortable: false,
          width: 90,
          render: (row) => <CacheReadCell price={row.price} />,
        },
        community,
        {
          id: "price-own",
          group,
          label: "$/1M at own mix",
          title: "Estimate at list prices: USD per 1M tokens at this model's own token mix in the period.",
          sortable: true,
          width: 120,
          render: (row, align) => (
            <EstimateCell row={row} which="own" communityMix={communityMix} ownMix={ownMixes[row.key] ?? null} align={align} />
          ),
        },
      );
    } else if (group === "specs") {
      columns.push(
        { id: "context", group, label: "Context", title: "Context window in tokens (models.dev).", sortable: true, width: 80, render: (row) => <ContextCell value={row.spec?.context ?? null} /> },
        { id: "max-output", group, label: "Max output", title: "Output limit in tokens (models.dev).", sortable: false, width: 90, render: (row) => <ContextCell value={row.spec?.maxOutput ?? null} /> },
        { id: "released", group, label: "Released", title: "Release date (models.dev).", sortable: true, width: 110, render: (row) => <DateCell value={row.spec?.releaseDate ?? null} /> },
        { id: "knowledge", group, label: "Knowledge", title: "Knowledge cutoff (models.dev).", sortable: false, width: 100, render: (row) => <DateCell value={row.spec?.knowledgeCutoff ?? null} /> },
        { id: "open-weights", group, label: "Open weights", title: "Whether the weights are published (models.dev).", sortable: false, width: 96, render: (row) => <OpenWeightsCell value={row.spec?.openWeights ?? null} /> },
      );
    }
  }
  return columns;
}

function communityShareText(mix: MixCounts | null): string {
  if (!mix) return "";
  return `, where cache reads are ${formatShare(mixShares(mix).cacheRead)} of tokens`;
}

function useOverflow(ref: React.RefObject<HTMLDivElement | null>, deps: unknown[]) {
  const [state, setState] = useState({ overflow: false, atEnd: true });
  useEffect(() => {
    const el = ref.current;
    if (!el) return;
    const update = () => {
      const overflow = el.scrollWidth > el.clientWidth + 1;
      setState({ overflow, atEnd: !overflow || el.scrollLeft + el.clientWidth >= el.scrollWidth - 2 });
    };
    update();
    el.addEventListener("scroll", update, { passive: true });
    const observer = new ResizeObserver(update);
    observer.observe(el);
    return () => {
      el.removeEventListener("scroll", update);
      observer.disconnect();
    };
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, deps);
  return state;
}

export function ModelsDashboard({
  rows,
  coverage,
  period,
  initialGroups,
  initialSort,
  benchmarks,
  communityMix,
  ownMixes,
  minUsers,
  topEci,
}: ModelsDashboardProps) {
  const [groups, setGroups] = useState<ColumnGroup[]>(initialGroups);
  const [sortChoice, setSortChoice] = useState(initialSort);
  const [showAll, setShowAll] = useState(false);
  const sort = effectiveSortKey(sortChoice, groups);
  const columns = useMemo(() => buildColumns(groups, period, benchmarks, communityMix, ownMixes), [groups, period, benchmarks, communityMix, ownMixes]);
  const sorted = useMemo(() => sortDashboardRows(rows, sort, sortAscending(sort)), [rows, sort]);
  const scrollRef = useRef<HTMLDivElement>(null);
  const { overflow, atEnd } = useOverflow(scrollRef, [columns.length]);

  useEffect(() => {
    const query = dashboardQuery({ groups, sort, period, defaultPeriod: DEFAULT_EXPLORE_PERIOD });
    const next = `${window.location.pathname}${query}`;
    if (next !== `${window.location.pathname}${window.location.search}`) {
      window.history.replaceState(window.history.state, "", next);
    }
  }, [groups, sort, period]);

  const periodOptions = EXPLORE_PERIODS.map((value) => ({
    href: `/models${dashboardQuery({ groups, sort, period: value, defaultPeriod: DEFAULT_EXPLORE_PERIOD })}`,
    label: periodShortLabel(value),
    active: value === period,
  }));

  const sortable = columns.filter((c) => c.sortable);
  const headerGroups: Array<{ group: Column["group"]; span: number }> = [];
  for (const column of columns) {
    const last = headerGroups[headerGroups.length - 1];
    if (last && last.group === column.group) last.span += 1;
    else headerGroups.push({ group: column.group, span: 1 });
  }
  const groupStart = new Set<string>();
  columns.forEach((c, i) => {
    if (i === 0 || columns[i - 1].group !== c.group) groupStart.add(c.id);
  });
  const withUsers = rows.filter((r) => (r.usage?.activeUsers ?? 0) >= minUsers).length;
  const fromEci = rows.length - withUsers;
  const usersInfo = `the ${formatCount(withUsers)} with ${minUsers}+ active users in this period${fromEci > 0 ? `, and ${formatCount(fromEci)} more from the top ${topEci} by Epoch Capabilities Index` : ""}`;

  return (
    <>
      <Controls>
        <LinkSegments label="Period" visibleLabel="Period" options={periodOptions} />
      </Controls>
      <ChipRow>
        <ChipLabel id="column-groups-label">Columns</ChipLabel>
        <Chips role="group" aria-labelledby="column-groups-label">
          <TouchSegment type="button" $active={groups.length === 0} aria-pressed={groups.length === 0} onClick={() => setGroups([])}>
            Overview
          </TouchSegment>
          {COLUMN_GROUPS.map((group) => (
            <TouchSegment
              key={group}
              type="button"
              $active={groups.includes(group)}
              aria-pressed={groups.includes(group)}
              onClick={() => setGroups((current) => toggleColumnGroup(current, group))}
            >
              {COLUMN_GROUP_LABELS[group]}
            </TouchSegment>
          ))}
        </Chips>
        <SortSelect>
          <label htmlFor="models-sort" id="models-sort-label">
            Sort by
          </label>
          <Select
            id="models-sort"
            aria-labelledby="models-sort-label"
            value={sort}
            options={sortable.map((c) => ({ value: c.id, label: c.label }))}
            onChange={setSortChoice}
          />
        </SortSelect>
      </ChipRow>

      <CoverageLine coverage={coverage} />
      <Meta>
        <span aria-live="polite">
          {formatCount(rows.length)} models: {usersInfo}.
        </span>
        <span>
          <InlineLink href="/models/all">All models</InlineLink>
          {" · "}
          <InlineLink href="/benchmarks">About the benchmarks</InlineLink>
        </span>
      </Meta>

      <ScrollBox>
        {overflow ? <ScrollHint aria-hidden="true">Scroll sideways for more columns</ScrollHint> : null}
        <ScrollArea>
        <Scroller ref={scrollRef} tabIndex={overflow ? 0 : -1} role="region" aria-label="Models table">
          <Table>
            <caption>
              <VisuallyHidden>
                Models by {columns.find((c) => c.id === sort)?.label ?? "active users"}, {periodShortLabel(period).toLowerCase()}
              </VisuallyHidden>
            </caption>
            <thead>
              {groups.length > 0 ? (
                <tr className="groups">
                  <th className="model" scope="col" rowSpan={2}>
                    <VisuallyHidden>Model</VisuallyHidden>
                  </th>
                  {headerGroups.map(({ group, span }, index) => (
                    <th key={`${group}-${index}`} colSpan={span} scope="colgroup" data-group-start="true">
                      {group === "overview" ? "Overview" : COLUMN_GROUP_LABELS[group]}
                    </th>
                  ))}
                </tr>
              ) : null}
              <tr>
                {groups.length === 0 ? (
                  <th className="model" scope="col">
                    Model
                  </th>
                ) : null}
                {columns.map((column) => (
                  <th
                    key={column.id}
                    scope="col"
                    data-group-start={groupStart.has(column.id) ? "true" : undefined}
                    aria-sort={column.id === sort ? (sortAscending(sort) ? "ascending" : "descending") : undefined}
                    style={{ minWidth: column.width }}
                  >
                    {column.sortable ? (
                      <InstantTooltip
                        as={SortButton}
                        type="button"
                        $active={column.id === sort}
                        aria-label={column.id === sort ? `${column.label}, sorted` : `Sort by ${column.label}`}
                        onClick={() => setSortChoice(column.id)}
                        content={column.title}
                      >
                        {column.label}
                        <span aria-hidden="true">{column.id === sort ? (sortAscending(sort) ? "\u2191" : "\u2193") : ""}</span>
                      </InstantTooltip>
                    ) : (
                      <HeaderTip content={column.title} focusable>
                        {column.label}
                      </HeaderTip>
                    )}
                  </th>
                ))}
              </tr>
            </thead>
            <tbody>
              {(showAll ? sorted : sorted.slice(0, ROW_LIMIT)).map((row, index) => (
                <tr key={row.key} data-row-link="">
                  <th className="model" scope="row">
                    <ModelCell>
                      <ModelMark model={row} size={24} />
                      <ModelLink href={modelPath(row.key)} data-row-primary="">
                        {row.name}
                      </ModelLink>
                      <LabText>
                        {index + 1}. {row.lab ?? "Lab unknown"}
                      </LabText>
                    </ModelCell>
                  </th>
                  {columns.map((column, columnIndex) => (
                    <td key={column.id} data-group-start={groupStart.has(column.id) ? "true" : undefined}>
                      {columnIndex === 0 ? (
                        <RowCover href={modelPath(row.key)} prefetch={false} tabIndex={-1} aria-hidden="true" data-row-cover="" />
                      ) : null}
                      {column.render(row, "end")}
                    </td>
                  ))}
                </tr>
              ))}
            </tbody>
          </Table>
        </Scroller>
        <Fade $visible={overflow && !atEnd} aria-hidden="true" />
        </ScrollArea>
        {!showAll && sorted.length > ROW_LIMIT ? (
          <TableMore type="button" onClick={() => setShowAll(true)}>
            Show all {formatCount(sorted.length)} models
          </TableMore>
        ) : null}
      </ScrollBox>

      <Cards aria-label="Models">
        {(showAll ? sorted : sorted.slice(0, CARD_LIMIT)).map((row, index) => (
          <Card key={row.key} data-row-link="">
            <CardHead>
              <span>{index + 1}</span>
              <ModelMark model={row} size={28} />
              <CardName>
                <ModelLink href={modelPath(row.key)} data-row-primary="">
                  {row.name}
                </ModelLink>
                <LabText>{row.lab ?? "Lab unknown"}</LabText>
              </CardName>
            </CardHead>
            <CardFacts>
              {columns.map((column) => (
                <div key={column.id}>
                  <InstantTooltip as="dt" content={column.title} describe={false}>
                    {column.label}
                  </InstantTooltip>
                  <dd>{column.render(row, "start")}</dd>
                </div>
              ))}
            </CardFacts>
          </Card>
        ))}
      </Cards>
      {!showAll && sorted.length > CARD_LIMIT ? (
        <MoreButton type="button" onClick={() => setShowAll(true)}>
          Show all {formatCount(sorted.length)} models
        </MoreButton>
      ) : null}

      <Footnote>
        Usage counts public Tokscale submissions only; hidden accounts are excluded. Scores are the best result each source
        reports for the model, with its effort and agent or harness; open a score for its source and date. Prices are list
        prices from models.dev, and every $/1M figure is an estimate at list prices, not what anyone paid. Usage and scores are
        shown side by side; one does not explain the other. Sorting by a score puts models that were not evaluated last.
      </Footnote>
    </>
  );
}
