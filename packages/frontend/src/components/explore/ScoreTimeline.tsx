"use client";

/**
 * Best score per model against the model's release date. SVG draws the grid
 * and the best-so-far line; dots and labels are HTML so they stay round and
 * sharp at any width.
 *
 * Accessibility: the plot is a focusable group with a text summary. Arrow
 * keys (and pointer hover) move a readout through the models in release
 * order, announced politely, and every point is in the "Show data" table.
 */
import Link from "next/link";
import { useId, useMemo, useState, type KeyboardEvent, type PointerEvent } from "react";
import styled from "styled-components";
import type { BenchmarkUnit } from "@/lib/benchmarks/definitions";
import { TIMELINE_MIN_POINTS, timelineGeometry, type TimelineData, type TimelinePoint } from "@/lib/benchmarks/detail";
import { formatIsoDay } from "@/lib/benchmarks/format";
import { SectionNote, SectionTitle, VisuallyHidden } from "./ExploreUI";
import { formatCount } from "./format";

const VIEW_W = 1000;
const VIEW_H = 300;
/** Name labels next to the highest few points (wide screens only). */
const LABELED_POINTS = 3;

const Head = styled.div`
  display: flex;
  flex-wrap: wrap;
  align-items: flex-end;
  justify-content: space-between;
  gap: 12px 24px;
`;

const Legend = styled.ul`
  display: flex;
  flex-wrap: wrap;
  gap: 6px 16px;
  margin: 0;
  padding: 0;
  list-style: none;
  color: var(--service-text-muted);
  font-size: 0.75rem;

  li {
    display: inline-flex;
    align-items: center;
    gap: 6px;
  }

  @media (max-width: 640px) {
    font-size: 0.8125rem;
  }
`;

const Swatch = styled.span<{ $kind: "used" | "unused" | "line" }>`
  display: inline-block;
  flex: 0 0 auto;
  ${({ $kind }) =>
    $kind === "line"
      ? `width: 16px; height: 0; border-top: 2px dashed rgba(255, 255, 255, 0.5);`
      : `width: 9px; height: 9px; border-radius: 50%; border: 1.5px solid ${
          $kind === "used" ? "var(--service-accent-hover)" : "rgba(255, 255, 255, 0.55)"
        }; background: ${$kind === "used" ? "var(--service-accent-hover)" : "transparent"};`}
`;

const Readout = styled.p`
  min-height: 1.5em;
  margin: 12px 0 14px;
  color: var(--service-text);
  font-size: 0.875rem;
  font-variant-numeric: tabular-nums;

  > span {
    color: var(--service-text-muted);
  }

  @media (max-width: 640px) {
    min-height: 3em;
  }
`;

const Plot = styled.div`
  position: relative;
  padding-left: 56px;

  @media (max-width: 640px) {
    padding-left: 46px;
  }
`;

const AxisTitle = styled.span`
  display: block;
  margin: 0 0 8px;
  color: var(--service-text-muted);
  font-size: 0.75rem;
`;

const Frame = styled.div`
  position: relative;
  height: 300px;
  border-bottom: 1px solid var(--service-border-strong);
  border-left: 1px solid var(--service-border);
  cursor: crosshair;
  touch-action: pan-y;

  &:focus-visible {
    border-radius: 4px;
    outline: 2px solid var(--service-focus);
    outline-offset: 6px;
  }

  @media (max-width: 640px) {
    height: 220px;
  }
`;

const Svg = styled.svg`
  position: absolute;
  inset: 0;
  width: 100%;
  height: 100%;
  overflow: visible;
`;

const YLabel = styled.span`
  position: absolute;
  left: -56px;
  width: 48px;
  color: var(--service-text-muted);
  font-size: 0.6875rem;
  font-variant-numeric: tabular-nums;
  text-align: right;
  transform: translateY(-50%);
  pointer-events: none;

  @media (max-width: 640px) {
    left: -46px;
    width: 40px;
  }
`;

const Dot = styled.span<{ $used: boolean }>`
  position: absolute;
  width: 9px;
  height: 9px;
  border: 1.5px solid ${({ $used }) => ($used ? "var(--service-accent-hover)" : "rgba(255, 255, 255, 0.55)")};
  border-radius: 50%;
  background: ${({ $used }) => ($used ? "var(--service-accent-hover)" : "var(--service-canvas)")};
  pointer-events: none;
  transform: translate(-50%, -50%);
`;

const ActiveRing = styled.span`
  position: absolute;
  width: 17px;
  height: 17px;
  border: 2px solid var(--service-text);
  border-radius: 50%;
  pointer-events: none;
  transform: translate(-50%, -50%);
`;

const PointLabel = styled.span<{ $side: "left" | "right" }>`
  position: absolute;
  max-width: 220px;
  overflow: hidden;
  color: var(--service-text);
  font-size: 0.75rem;
  text-overflow: ellipsis;
  white-space: nowrap;
  /* A dark halo keeps the name readable where it crosses other dots or the line. */
  text-shadow:
    0 0 3px var(--service-canvas),
    0 0 3px var(--service-canvas),
    0 0 6px var(--service-canvas);
  pointer-events: none;
  transform: ${({ $side }) => ($side === "left" ? "translate(calc(-100% - 10px), -50%)" : "translate(10px, -50%)")};

  @media (max-width: 640px) {
    display: none;
  }
`;

const XLabels = styled.div`
  position: relative;
  height: 1.5em;
  margin-top: 6px;
  color: var(--service-text-muted);
  font-size: 0.75rem;
  font-variant-numeric: tabular-nums;
`;

const XLabel = styled.span`
  position: absolute;
  top: 0;
  white-space: nowrap;
`;

const XTitle = styled.span`
  display: block;
  margin-top: 4px;
  color: var(--service-text-muted);
  font-size: 0.75rem;
  text-align: center;
`;

const Footnote = styled.p`
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
  font-size: 0.8125rem;
  font-variant-numeric: tabular-nums;

  th,
  td {
    padding: 6px 12px 6px 0;
    border-bottom: 1px solid var(--service-border);
    text-align: left;
    vertical-align: top;
  }

  th[scope="col"] {
    position: sticky;
    /* Above the row links and the links raised over them (globals.css). */
    z-index: 3;
    top: 0;
    background: var(--service-canvas);
    color: var(--service-text-muted);
    font-weight: 500;
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
    font-size: 0.875rem;
  }
`;

const DataSub = styled.span`
  display: block;
  color: var(--service-text-muted);
  font-size: 0.75rem;
`;

const DataLink = styled(Link)`
  color: var(--service-text);
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

interface ScoreTimelineProps {
  timeline: TimelineData;
  unit: BenchmarkUnit;
  unitLabel: string;
  benchmarkName: string;
  /** Who lists release dates in the benchmark's snapshot, e.g. "Epoch AI". */
  sourceName: string | null;
  /** False when Tokscale usage could not be loaded: dots are then not split by usage. */
  usageAvailable: boolean;
}

function usageText(point: TimelinePoint, usageAvailable: boolean): string {
  if (!usageAvailable) return "";
  return point.activeUsers ? `${formatCount(point.activeUsers)} active users on Tokscale in the last 30 days` : "No Tokscale usage in the last 30 days";
}

function releaseText(point: TimelinePoint): string {
  return `released ${formatIsoDay(point.releaseDate)}${point.releaseFrom === "catalog" ? " (date from models.dev)" : ""}`;
}

/** Highest-scoring points, skipping any that would sit on top of a label already placed. */
function labeledIndexes(points: readonly TimelinePoint[], positions: ReadonlyArray<{ x: number; y: number }>): Set<number> {
  const order = points.map((_, i) => i).sort((a, b) => points[b].score - points[a].score);
  const chosen: number[] = [];
  for (const i of order) {
    if (chosen.length >= LABELED_POINTS) break;
    const clash = chosen.some((j) => Math.abs(positions[i].y - positions[j].y) < 0.08 && Math.abs(positions[i].x - positions[j].x) < 0.3);
    if (!clash) chosen.push(i);
  }
  return new Set(chosen);
}

export function ScoreTimeline({ timeline, unit, unitLabel, benchmarkName, sourceName, usageAvailable }: ScoreTimelineProps) {
  const titleId = useId();
  const { points } = timeline;
  const geometry = useMemo(() => timelineGeometry(points, unit), [points, unit]);
  const topIndex = useMemo(() => {
    let best = 0;
    points.forEach((p, i) => {
      if (p.score > points[best].score) best = i;
    });
    return best;
  }, [points]);
  const labeled = useMemo(() => labeledIndexes(points, geometry.positions), [points, geometry.positions]);
  const [index, setIndex] = useState<number | null>(null);

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

  const header = (
    <div>
      <SectionTitle id={titleId}>Score by release date</SectionTitle>
      <SectionNote>
        Each dot is one model&apos;s best result on {benchmarkName}, placed at the model&apos;s release date. The dashed line is the
        best score among models released up to that date.
      </SectionNote>
    </div>
  );

  if (points.length < TIMELINE_MIN_POINTS) {
    return (
      <div>
        {header}
        <Footnote>
          Not enough matched models with a known release date to draw a chart ({points.length} of {timeline.models}). The
          results table lists every score.
        </Footnote>
      </div>
    );
  }

  const last = points.length - 1;
  const active = index ?? topIndex;
  const point = points[active];
  const position = geometry.positions[active];
  const first = points[0];
  const newest = points[last];
  const top = points[topIndex];

  const onPointer = (event: PointerEvent<HTMLDivElement>) => {
    const rect = event.currentTarget.getBoundingClientRect();
    if (rect.width <= 0 || rect.height <= 0) return;
    const px = event.clientX - rect.left;
    const py = event.clientY - rect.top;
    let nearest = 0;
    let distance = Number.POSITIVE_INFINITY;
    geometry.positions.forEach((p, i) => {
      const d = (p.x * rect.width - px) ** 2 + (p.y * rect.height - py) ** 2;
      if (d < distance) {
        distance = d;
        nearest = i;
      }
    });
    setIndex(nearest);
  };

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
  const summary = `${points.length} models, released ${formatIsoDay(first.releaseDate)} to ${formatIsoDay(newest.releaseDate)}. Highest: ${top.name}, ${top.scoreText}, ${releaseText(top)}. Use the left and right arrow keys to read each model in release order.`;
  const tableRows = [...points].reverse();

  return (
    <div>
      <Head>
        {header}
        <Legend aria-label="Legend">
          {usageAvailable ? (
            <>
              <li>
                <Swatch $kind="used" aria-hidden="true" />
                Used on Tokscale in the last 30 days
              </li>
              <li>
                <Swatch $kind="unused" aria-hidden="true" />
                No Tokscale usage in the last 30 days
              </li>
            </>
          ) : null}
          <li>
            <Swatch $kind="line" aria-hidden="true" />
            Best score so far
          </li>
        </Legend>
      </Head>

      <Readout aria-live="polite">
        {point.name}: {point.scoreText}{" "}
        <span>
          · {releaseText(point)}
          {point.conditions ? ` · ${point.conditions}` : ""}
          {usage ? ` · ${usage}` : ""}
        </span>
      </Readout>

      <Plot>
        <AxisTitle aria-hidden="true">{unitLabel}</AxisTitle>
        <Frame
          role="group"
          tabIndex={0}
          aria-labelledby={titleId}
          aria-describedby={`${titleId}-summary`}
          onPointerMove={onPointer}
          onPointerDown={onPointer}
          onPointerLeave={() => setIndex(null)}
          onKeyDown={onKey}
        >
          <VisuallyHidden id={`${titleId}-summary`}>{summary}</VisuallyHidden>
          <Svg viewBox={`0 0 ${VIEW_W} ${VIEW_H}`} preserveAspectRatio="none" aria-hidden="true" focusable="false">
            {geometry.y.ticks.map((tick) => {
              const y = (1 - (tick.value - geometry.y.min) / (geometry.y.max - geometry.y.min)) * VIEW_H;
              const zero = unit === "effect" && Math.abs(tick.value) < 1e-9;
              return (
                <line
                  key={tick.value}
                  x1={0}
                  x2={VIEW_W}
                  y1={y}
                  y2={y}
                  stroke={zero ? "rgba(255,255,255,0.3)" : "rgba(255,255,255,0.08)"}
                  strokeWidth={1}
                  vectorEffect="non-scaling-stroke"
                />
              );
            })}
            {geometry.frontier.length > 1 ? (
              <polyline
                points={geometry.frontier.map((p) => `${(p.x * VIEW_W).toFixed(1)},${(p.y * VIEW_H).toFixed(1)}`).join(" ")}
                fill="none"
                stroke="rgba(255,255,255,0.5)"
                strokeWidth={1.5}
                strokeDasharray="4 4"
                vectorEffect="non-scaling-stroke"
              />
            ) : null}
          </Svg>
          {geometry.y.ticks.map((tick) => (
            <YLabel key={tick.value} style={{ top: `${(1 - (tick.value - geometry.y.min) / (geometry.y.max - geometry.y.min)) * 100}%` }}>
              {tick.label}
            </YLabel>
          ))}
          {points.map((p, i) => (
            <Dot
              key={p.key}
              $used={!usageAvailable || Boolean(p.activeUsers)}
              style={{ left: `${geometry.positions[i].x * 100}%`, top: `${geometry.positions[i].y * 100}%` }}
            />
          ))}
          {points.map((p, i) =>
            labeled.has(i) ? (
              <PointLabel
                key={p.key}
                aria-hidden="true"
                $side={geometry.positions[i].x > 0.6 ? "left" : "right"}
                style={{ left: `${geometry.positions[i].x * 100}%`, top: `${geometry.positions[i].y * 100}%` }}
              >
                {p.name}
              </PointLabel>
            ) : null,
          )}
          <ActiveRing style={{ left: `${position.x * 100}%`, top: `${position.y * 100}%` }} />
        </Frame>
        <XLabels aria-hidden="true">
          {geometry.x.ticks.map((tick) => {
            const x = (tick.value - geometry.x.min) / (geometry.x.max - geometry.x.min);
            const shift = x < 0.08 ? "0" : x > 0.92 ? "-100%" : "-50%";
            return (
              <XLabel key={tick.value} style={{ left: `${x * 100}%`, transform: `translateX(${shift})` }}>
                {tick.label}
              </XLabel>
            );
          })}
        </XLabels>
        <XTitle aria-hidden="true">Model release date</XTitle>
      </Plot>

      {catalogNote || undatedNote ? <Footnote>{[catalogNote, undatedNote].filter(Boolean).join(" ")}</Footnote> : null}

      <DataDetails>
        <summary>Show data</summary>
        <DataScroll role="region" aria-label={`Chart data for ${benchmarkName}`} tabIndex={0}>
          <DataTable>
            <caption>
              <VisuallyHidden>
                Best score per model on {benchmarkName} by release date, newest release first
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
