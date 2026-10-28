"use client";

/**
 * Best score per model, as dithered bars in model-release order.
 *
 * The bars are dither-kit's `<BarChart>` inside the shared `<ChartBox>` frame;
 * the plot is a focusable group whose ArrowLeft / ArrowRight / Home / End
 * move a readout through the models, announced politely. Pointer hover and
 * the keyboard share one marker: a 1px line on the active bar, drawn as a
 * small chart part on the kit's chart context (the kit's bar painter has no
 * crosshair of its own). Every bar is also in the "Show data" table.
 */
import Link from "next/link";
import { useCallback, useId, useMemo, useState, type KeyboardEvent } from "react";
import styled from "styled-components";
import { BarChart } from "dither-kit/bar-chart";
import { buildYScale } from "dither-kit/scales";
import { Bar } from "dither-kit/Bar";
import { Grid } from "dither-kit/Grid";
import { XAxis } from "dither-kit/XAxis";
import { YAxis } from "dither-kit/YAxis";
import { Tooltip } from "dither-kit/Tooltip";
import { useChart } from "dither-kit/chart-context";
import type { VariantInput } from "dither-kit/dither-paint";
import type { BenchmarkUnit } from "@/lib/benchmarks/definitions";
import { TIMELINE_MIN_POINTS, axisLabel, type TimelineData, type TimelinePoint } from "@/lib/benchmarks/detail";
import { formatIsoDay } from "@/lib/benchmarks/format";
import { CHART_PROPS, ChartBox, chartFrame } from "./ExploreCharts";
import { DitherSwatch } from "./DitherViz";
import { SectionNote, VisuallyHidden } from "./ExploreUI";
import { formatCount } from "./format";

/** Chart height in css px; the phone gets a shorter plot, not a squeezed one. */
const CHART_HEIGHT = 260;
const CHART_HEIGHT_PHONE = 200;
const CHART_MARGINS = { top: 12, right: 8, bottom: 26, left: 48 };
/** Y ticks: 3-5 gridlines, the count the kit's axis and grid share. */
const Y_TICKS = 4;
/** Plot height only scales pixels; the kit's tick maths reads the domain. */
const PLOT_PROBE = 160;

/** The kit's config key doubles as the tooltip row's label. */
const KEY_USED = "used:Used on Tokscale";
const KEY_UNUSED = "unused:No Tokscale usage";

/**
 * Both series drop the kit's top-edge line: a zero-height band would
 * otherwise paint one edge row at the baseline for every model in the other
 * series. The dimming is then purely the unused series' opacity.
 */
const FLAT_VARIANT: VariantInput = { edge: 0 };

const Head = styled.div`
  display: flex;
  flex-wrap: wrap;
  align-items: flex-start;
  justify-content: space-between;
  gap: 8px 24px;
`;

const HeadText = styled.div`
  display: grid;
  max-width: 78ch;
  gap: 4px;
  min-width: 0;
`;

const Legend = styled.ul`
  display: flex;
  flex-wrap: wrap;
  gap: 4px 16px;
  margin: 0;
  padding: 0;
  color: var(--service-text-muted);
  font-size: 0.8125rem;
  list-style: none;

  li {
    display: inline-flex;
    align-items: center;
    gap: 8px;
  }

  @media (max-width: 640px) {
    font-size: 0.875rem;
  }
`;

const DimmedSwatch = styled.span`
  display: inline-flex;
  flex: 0 0 auto;
  opacity: 0.45;
`;

const Readout = styled.p`
  min-height: 1.5em;
  margin: 12px 0 4px;
  color: var(--service-text);
  font-size: 0.875rem;
  font-variant-numeric: tabular-nums;
  line-height: 1.5;

  > span {
    color: var(--service-text-muted);
  }

  @media (max-width: 640px) {
    font-size: 0.9375rem;
  }
`;

const Frame = styled.div`
  position: relative;
  touch-action: pan-y;
  cursor: crosshair;

  ${chartFrame}

  /* chartFrame is written for a fixed-height box; here the plot is the box. */
  height: auto;

  svg .stroke-border {
    stroke: var(--service-border-strong);
  }

  &:focus-visible {
    border-radius: 2px;
    outline: 2px solid var(--service-focus);
    outline-offset: 4px;
  }
`;

const Plot = styled(ChartBox)`
  height: ${CHART_HEIGHT}px;

  @media (max-width: 640px) {
    height: ${CHART_HEIGHT_PHONE}px;
  }
`;

const Caption = styled.p`
  max-width: 80ch;
  margin: 12px 0 0;
  color: var(--service-text-muted);
  font-size: 0.8125rem;
  line-height: 1.5;

  @media (max-width: 640px) {
    font-size: 0.875rem;
  }
`;

const DataDetails = styled.details`
  margin-top: 8px;

  > summary {
    display: inline-flex;
    min-height: 32px;
    align-items: center;
    color: var(--service-text);
    font-size: 0.8125rem;
    font-weight: 500;
    cursor: pointer;
    text-underline-offset: 4px;
  }

  > summary:hover {
    text-decoration: underline;
  }

  > summary:focus-visible {
    border-radius: 2px;
    outline: 2px solid var(--service-focus);
    outline-offset: 2px;
  }

  @media (max-width: 640px), (pointer: coarse) {
    > summary {
      min-height: 44px;
      font-size: 0.875rem;
    }
  }
`;

/* Row focus rings sit inside, so the scroller does not clip them. */
const DataScroll = styled.div`
  --row-focus-offset: -2px;

  max-height: 360px;
  overflow: auto;
  border-top: 1px solid var(--service-border);

  &:focus-visible {
    outline: 2px solid var(--service-focus);
    outline-offset: 2px;
  }
`;

const DataTable = styled.table`
  width: 100%;
  border-collapse: collapse;
  font-size: 0.875rem;
  font-variant-numeric: tabular-nums;

  th,
  td {
    padding: 10px 12px 10px 0;
    border-bottom: 1px solid var(--service-border);
    text-align: left;
    vertical-align: top;
  }

  th:last-child,
  td:last-child {
    padding-right: 0;
  }

  th[scope="col"] {
    position: sticky;
    /* Above the row links and the links raised over them (globals.css). */
    z-index: 3;
    top: 0;
    background: var(--service-canvas);
    color: var(--service-text-muted);
    font-size: 0.8125rem;
    font-weight: 400;
    white-space: nowrap;
  }

  th[scope="row"] {
    font-weight: 500;
    overflow-wrap: anywhere;
  }

  .num {
    text-align: right;
  }

  @media (max-width: 640px) {
    font-size: 0.9375rem;
  }
`;

const DataSub = styled.span`
  display: block;
  color: var(--service-text-muted);
  font-size: 0.8125rem;
`;

const DataLink = styled(Link)`
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

interface Tick {
  value: number;
  label: string;
}

/** The smallest positive tick in a set, i.e. the axis step. */
function tickStep(values: readonly number[], fallback: number): number {
  let step = Number.POSITIVE_INFINITY;
  for (const value of values) {
    if (value > 0 && value < step) step = value;
  }
  return Number.isFinite(step) ? step : fallback;
}

/**
 * Where the bars' floor sits on the axis, in score units.
 *
 * Percentages, the ECI and arena ratings are read from zero, and the kit's
 * domain is always `[0, max]` niced, so those pass straight through. Agent
 * Arena's effect can be negative: the floor then moves to the first whole
 * tick below the lowest score, so zero stays on a tick and bars stay true.
 */
function barBase(unit: BenchmarkUnit, scores: readonly number[]): number {
  if (unit !== "effect") return 0;
  const finite = scores.filter((value) => Number.isFinite(value));
  if (finite.length === 0) return 0;
  const low = Math.min(...finite);
  if (low >= 0) return 0;
  const high = Math.max(...finite);
  const spread = Math.max(high - low, 1e-6);
  const step = tickStep(buildYScale(spread, PLOT_PROBE).ticks(Y_TICKS), spread);
  let base = -step;
  while (base > low - 1e-12) base -= step;
  return Number(base.toPrecision(12));
}

/**
 * Y ticks as the kit draws them (`domain [0, max].nice()`, 4 ticks),
 * relabelled in the benchmark's unit. Plot values are shifted by the bar
 * floor, so every label adds it back before formatting.
 */
function plotTicks(max: number, base: number, unit: BenchmarkUnit): Tick[] {
  const safe = Number.isFinite(max) && max > 0 ? max : 1;
  const domain = buildYScale(safe, PLOT_PROBE);
  const values = domain.ticks(Y_TICKS);
  const step = tickStep(values, domain.domain()[1]);
  const ticks = values.map((value) => ({ value, label: axisLabel(unit, value + base, step) }));
  // A shifted floor always shows zero, or the sign of a bar has no scale.
  if (base !== 0 && !ticks.some((tick) => Math.abs(tick.value) <= 1e-12)) {
    ticks.push({ value: -base, label: axisLabel(unit, 0, step) });
    ticks.sort((a, b) => a.value - b.value);
  }
  return ticks;
}

/**
 * X tick: the release day and year, "Sep 22 ’26". The axis is categorical
 * (models in release order), so a bare month repeats and a bare day misreads
 * across years; five of these fit a phone-width plot without turning.
 */
function releaseTick(iso: string): string {
  const [year, month, day] = iso.split("-");
  const date = new Date(`${year}-${month}-${day}T00:00:00Z`);
  const short = date.toLocaleDateString("en-US", { month: "short", day: "numeric", timeZone: "UTC" });
  return `${short} \u2019${year.slice(2)}`;
}

/**
 * 1px marker on the active bar, in the one data hue. It reads
 * `hoverIndex ?? markerIndex`, so pointer hover and the arrow keys light the
 * same line. App code on the kit's chart context: the bar painter draws no
 * crosshair of its own.
 */
function ActiveMarker() {
  const chart = useChart();
  if (!chart.ready) return null;
  const index = chart.hoverIndex ?? chart.markerIndex;
  if (index === null || index < 0 || index >= chart.dataLength) return null;
  const x = Math.round(chart.xCenter(index));
  return (
    <line
      x1={x + 0.5}
      x2={x + 0.5}
      y1={0}
      y2={chart.plot.height}
      stroke="var(--service-accent)"
      strokeWidth={1}
      vectorEffect="non-scaling-stroke"
      aria-hidden="true"
    />
  );
}

interface ScoreTimelineProps {
  timeline: TimelineData;
  unit: BenchmarkUnit;
  unitLabel: string;
  benchmarkName: string;
  /** Who lists release dates in the benchmark's snapshot, e.g. "Epoch AI". */
  sourceName: string | null;
  /** False when Tokscale usage could not be loaded: bars are then not split by usage. */
  usageAvailable: boolean;
}

function usageText(point: TimelinePoint, usageAvailable: boolean): string {
  if (!usageAvailable) return "";
  return point.activeUsers ? `${formatCount(point.activeUsers)} active users on Tokscale in the last 30 days` : "No Tokscale usage in the last 30 days";
}

function releaseText(point: TimelinePoint): string {
  return `released ${formatIsoDay(point.releaseDate)}${point.releaseFrom === "catalog" ? " (date from models.dev)" : ""}`;
}

export function ScoreTimeline({ timeline, unit, unitLabel, benchmarkName, sourceName, usageAvailable }: ScoreTimelineProps) {
  const groupId = useId().replace(/:/g, "");
  const { points } = timeline;
  const [index, setIndex] = useState<number | null>(null);

  const base = useMemo(() => barBase(unit, points.map((point) => point.score)), [points, unit]);
  const rows = useMemo(
    () =>
      points.map((point) => {
        const height = Math.max(0, point.score - base);
        const used = !usageAvailable || Boolean(point.activeUsers);
        return {
          id: point.key,
          model: point.name,
          [KEY_USED]: used ? height : 0,
          [KEY_UNUSED]: used ? 0 : height,
          released: releaseTick(point.releaseDate),
        };
      }),
    [points, base, usageAvailable],
  );
  const max = useMemo(() => {
    let top = 0;
    for (const row of rows) {
      top = Math.max(top, row[KEY_USED], row[KEY_UNUSED]);
    }
    return top;
  }, [rows]);
  const ticks = useMemo(() => plotTicks(max, base, unit), [max, base, unit]);
  const config = useMemo(
    () => ({
      [KEY_USED]: { label: "Used on Tokscale", color: "blue" as const },
      [KEY_UNUSED]: { label: "No Tokscale usage", color: "blue" as const },
    }),
    [],
  );

  const topIndex = useMemo(() => {
    let best = 0;
    points.forEach((point, i) => {
      if (point.score > points[best].score) best = i;
    });
    return best;
  }, [points]);

  const skipTick = useCallback(
    (value: unknown) => ticks.find((tick) => Math.abs(tick.value - Number(value)) < 1e-9)?.label ?? "",
    [ticks],
  );

  if (points.length < TIMELINE_MIN_POINTS) {
    const shortNote = `Not enough matched models with a known release date to draw a chart (${points.length} of ${timeline.models}). The results table lists every score.`;
    return <SectionNote>{shortNote}</SectionNote>;
  }

  const last = points.length - 1;
  const active = index ?? topIndex;
  const point = points[active];
  const first = points[0];
  const newest = points[last];

  const catalogNote =
    timeline.catalogDates === 0
      ? null
      : timeline.catalogDates === points.length
        ? `Release dates are from models.dev${sourceName ? `, because ${sourceName} does not list them for this benchmark` : ""}.`
        : `Release dates are from ${sourceName ?? "the source"}; ${timeline.catalogDates} ${timeline.catalogDates === 1 ? "comes" : "come"} from models.dev where ${sourceName ?? "the source"} lists none.`;
  const undatedNote =
    timeline.undated > 0
      ? `${timeline.undated} ${timeline.undated === 1 ? "model has" : "models have"} no known release date and ${timeline.undated === 1 ? "is" : "are"} left out.`
      : null;

  const onKey = (event: KeyboardEvent<HTMLDivElement>) => {
    const step = event.shiftKey ? 5 : 1;
    if (event.key === "ArrowLeft") setIndex(Math.max(0, active - step));
    else if (event.key === "ArrowRight") setIndex(Math.min(last, active + step));
    else if (event.key === "Home") setIndex(0);
    else if (event.key === "End") setIndex(last);
    else return;
    event.preventDefault();
  };

  const usage = usageText(point, usageAvailable);
  const range = `${points.length} models, released ${formatIsoDay(first.releaseDate)} to ${formatIsoDay(newest.releaseDate)}.`;
  const summary = `${range} Use the left and right arrow keys to read each model in release order.`;
  const caption = `${range} ${[catalogNote, undatedNote].filter(Boolean).join(" ")}`.trim();
  const tableRows = [...points].reverse();

  return (
    <div>
      <Head>
        <HeadText>
          <SectionNote>
            One bar per model, in release order, at that model&apos;s best result on {benchmarkName}. The axis carries the
            benchmark&apos;s own unit; a dimmed bar marks a model with no Tokscale usage in the last 30 days.
          </SectionNote>
        </HeadText>
        {usageAvailable ? (
          <Legend aria-label="Legend">
            <li>
              <DitherSwatch color="blue" height={10} />
              Used on Tokscale
            </li>
            <li>
              <DimmedSwatch>
                <DitherSwatch color="blue" height={10} />
              </DimmedSwatch>
              No Tokscale usage in the last 30 days
            </li>
          </Legend>
        ) : null}
      </Head>

      <Readout aria-live="polite">
        {point.name}: {point.scoreText}{" "}
        <span>
          · {releaseText(point)}
          {point.conditions ? ` · ${point.conditions}` : ""}
          {usage ? ` · ${usage}` : ""}
        </span>
      </Readout>

      <Frame
        role="group"
        tabIndex={0}
        aria-label={`${benchmarkName}: best score by model release date`}
        aria-describedby={`${groupId}-summary`}
        onPointerLeave={() => setIndex(null)}
        onBlur={() => setIndex(null)}
        onKeyDown={onKey}
      >
        <Plot $height={CHART_HEIGHT} aria-hidden="true">
          <BarChart
            data={rows}
            config={config}
            stackType="stacked"
            margins={CHART_MARGINS}
            markerIndex={active}
            onHoverChange={setIndex}
            {...CHART_PROPS}
          >
            <Grid horizontal tickCount={Y_TICKS} />
            <YAxis tickCount={Y_TICKS} tickFormatter={skipTick} />
            <XAxis dataKey="released" maxTicks={5} />
            <Bar dataKey={KEY_USED} variant={FLAT_VARIANT} />
            <Bar dataKey={KEY_UNUSED} variant={FLAT_VARIANT} opacity={0.45} />
            <ActiveMarker />
            <Tooltip
              labelKey="model"
              valueFormatter={(value, name) =>
                name.startsWith("used:") ? `${point.scoreText} ${unitLabel}` : Number.isFinite(value) && value > 0 ? point.scoreText : ""
              }
            />
          </BarChart>
        </Plot>
      </Frame>

      <VisuallyHidden id={`${groupId}-summary`}>{summary}</VisuallyHidden>

      <Caption>{caption}</Caption>

      <DataDetails>
        <summary>Show data</summary>
        <DataScroll role="region" aria-label={`Chart data for ${benchmarkName}`} tabIndex={0}>
          <DataTable>
            <caption>
              <VisuallyHidden>
                Best score per model on {benchmarkName}, in release order, newest release first
              </VisuallyHidden>
            </caption>
            <thead>
              <tr>
                <th scope="col">Model</th>
                <th scope="col">Released</th>
                <th scope="col" className="num">
                  Score
                </th>
                {usageAvailable ? (
                  <th scope="col" className="num">
                    Tokscale users, 30 days
                  </th>
                ) : null}
              </tr>
            </thead>
            <tbody>
              {tableRows.map((p) => (
                <tr key={p.key} data-row-link={p.href ? "" : undefined}>
                  <th scope="row">
                    {p.href ? (
                      <DataLink href={p.href} data-row-primary="">
                        {p.name}
                      </DataLink>
                    ) : (
                      p.name
                    )}
                  </th>
                  <td>
                    {formatIsoDay(p.releaseDate)}
                    {p.releaseFrom === "catalog" ? <DataSub>models.dev</DataSub> : null}
                  </td>
                  <td className="num">
                    {p.scoreText}
                    {p.conditions ? <DataSub>{p.conditions}</DataSub> : null}
                  </td>
                  {usageAvailable ? (
                    <td className="num">
                      {p.activeUsers ? (
                        formatCount(p.activeUsers)
                      ) : (
                        <>
                          <span aria-hidden="true">{"\u2014"}</span>
                          <VisuallyHidden>No Tokscale usage in the last 30 days</VisuallyHidden>
                        </>
                      )}
                    </td>
                  ) : null}
                </tr>
              ))}
            </tbody>
          </DataTable>
        </DataScroll>
      </DataDetails>
    </div>
  );
}
