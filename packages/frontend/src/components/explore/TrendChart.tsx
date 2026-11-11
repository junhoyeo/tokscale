"use client";

/**
 * Daily usage trend for one model. Plain SVG, no chart dependency.
 *
 * Accessibility: the chart is a focusable group. Arrow keys (and pointer
 * hover) move a readout that is announced politely, and the full series is
 * available as a table under "Show daily values".
 */
import { useId, useMemo, useState, type KeyboardEvent, type PointerEvent } from "react";
import styled from "styled-components";
import { ButtonSegments, SectionNote, SectionTitle, VisuallyHidden } from "./ExploreUI";
import { buildTrendGeometry, formatCount, formatDay, formatTokens } from "./format";

type Metric = "activeUsers" | "tokens";

const VIEW_W = 1000;
const VIEW_H = 200;

const Head = styled.div`
  display: flex;
  flex-wrap: wrap;
  align-items: flex-end;
  justify-content: space-between;
  gap: 12px;
`;

const Readout = styled.p`
  min-height: 1.5em;
  margin: 12px 0 16px;
  color: var(--service-text);
  font-size: 0.875rem;
  font-variant-numeric: tabular-nums;

  > span {
    color: var(--service-text-muted);
  }
`;

const Frame = styled.div`
  position: relative;
  height: 220px;
  margin-left: 52px;
  border-bottom: 1px solid var(--service-border-strong);
  cursor: crosshair;
  touch-action: pan-y;

  &:focus-visible {
    border-radius: 4px;
    outline: 2px solid var(--service-focus);
    outline-offset: 4px;
  }

  @media (max-width: 640px) {
    height: 180px;
    margin-left: 44px;
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
  left: -52px;
  width: 44px;
  color: var(--service-text-muted);
  font-size: 0.6875rem;
  font-variant-numeric: tabular-nums;
  text-align: right;
  transform: translateY(-50%);

  @media (max-width: 640px) {
    left: -44px;
    width: 38px;
  }
`;

const XLabels = styled.div`
  display: flex;
  justify-content: space-between;
  margin: 6px 0 0 52px;
  color: var(--service-text-muted);
  font-size: 0.75rem;
  font-variant-numeric: tabular-nums;

  @media (max-width: 640px) {
    margin-left: 44px;
  }
`;

const Cursor = styled.span`
  position: absolute;
  top: 0;
  bottom: 0;
  width: 1px;
  background: rgba(255, 255, 255, 0.28);
  pointer-events: none;
`;

const Dot = styled.span`
  position: absolute;
  width: 9px;
  height: 9px;
  border: 2px solid var(--service-canvas);
  border-radius: 50%;
  background: var(--service-accent-hover);
  pointer-events: none;
  transform: translate(-50%, -50%);
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

const DataTableWrap = styled.div`
  max-height: 320px;
  overflow-y: auto;
  border-top: 1px solid var(--service-border);
`;

const DataTable = styled.table`
  width: 100%;
  border-collapse: collapse;
  font-size: 0.8125rem;
  font-variant-numeric: tabular-nums;

  th,
  td {
    padding: 6px 8px 6px 0;
    border-bottom: 1px solid var(--service-border);
    text-align: right;
  }

  th:first-child,
  td:first-child {
    text-align: left;
  }

  th {
    color: var(--service-text-muted);
    font-weight: 500;
  }
`;

export interface TrendPoint {
  date: string;
  activeUsers: number;
  tokens: number;
}

const METRIC_OPTIONS = [
  { value: "activeUsers" as const, label: "Active users" },
  { value: "tokens" as const, label: "Tokens" },
];

function formatMetric(metric: Metric, value: number): string {
  return metric === "tokens" ? `${formatTokens(value)} tokens` : `${formatCount(value)} active users`;
}

function axisValue(metric: Metric, value: number): string {
  return metric === "tokens" ? formatTokens(value) : formatCount(value);
}

export function TrendChart({ trend, modelName }: { trend: TrendPoint[]; modelName: string }) {
  const [metric, setMetric] = useState<Metric>("activeUsers");
  const [index, setIndex] = useState<number | null>(null);
  const titleId = useId();

  const values = useMemo(() => trend.map((point) => point[metric]), [trend, metric]);
  const geometry = useMemo(() => buildTrendGeometry(values, VIEW_W, VIEW_H), [values]);

  if (trend.length === 0) {
    return (
      <div>
        <SectionTitle>Daily usage</SectionTitle>
        <SectionNote>No daily data in this period.</SectionNote>
      </div>
    );
  }

  const last = trend.length - 1;
  const active = index ?? last;
  const point = geometry.points[active];
  const activeDay = trend[active];

  const onPointer = (event: PointerEvent<HTMLDivElement>) => {
    const rect = event.currentTarget.getBoundingClientRect();
    if (rect.width <= 0) return;
    const ratio = Math.min(1, Math.max(0, (event.clientX - rect.left) / rect.width));
    setIndex(Math.round(ratio * last));
  };

  const onKey = (event: KeyboardEvent<HTMLDivElement>) => {
    const step = event.shiftKey ? 7 : 1;
    if (event.key === "ArrowLeft") setIndex(Math.max(0, active - step));
    else if (event.key === "ArrowRight") setIndex(Math.min(last, active + step));
    else if (event.key === "Home") setIndex(0);
    else if (event.key === "End") setIndex(last);
    else return;
    event.preventDefault();
  };

  const metricLabel = metric === "tokens" ? "tokens" : "active users";
  const midDate = trend[Math.floor(last / 2)]?.date;

  return (
    <div>
      <Head>
        <div>
          <SectionTitle id={titleId}>Daily {metricLabel}</SectionTitle>
          <SectionNote>
            {metric === "tokens"
              ? "Tokens on this model per UTC day, cache included."
              : "Distinct accounts that used this model on each UTC day."}
          </SectionNote>
        </div>
        <ButtonSegments label="Chart metric" value={metric} options={METRIC_OPTIONS} onChange={setMetric} />
      </Head>

      <Readout aria-live="polite">
        {formatMetric(metric, activeDay[metric])} <span>on {formatDay(activeDay.date)}</span>
      </Readout>

      <Frame
        role="group"
        tabIndex={0}
        aria-labelledby={titleId}
        aria-describedby={`${titleId}-help`}
        onPointerMove={onPointer}
        onPointerDown={onPointer}
        onPointerLeave={() => setIndex(null)}
        onKeyDown={onKey}
      >
        <VisuallyHidden id={`${titleId}-help`}>
          {`Daily ${metricLabel} for ${modelName}, ${formatDay(trend[0].date)} to ${formatDay(trend[last].date)}. Peak ${axisValue(metric, Math.max(...values))}. Use the left and right arrow keys to read each day.`}
        </VisuallyHidden>
        <Svg viewBox={`0 0 ${VIEW_W} ${VIEW_H}`} preserveAspectRatio="none" aria-hidden="true" focusable="false">
          {[0, 0.5].map((fraction) => (
            <line
              key={fraction}
              x1={0}
              x2={VIEW_W}
              y1={VIEW_H * fraction}
              y2={VIEW_H * fraction}
              stroke="rgba(255,255,255,0.08)"
              strokeWidth={1}
              vectorEffect="non-scaling-stroke"
            />
          ))}
          <path d={geometry.area} fill="rgba(47, 143, 255, 0.12)" />
          <path
            d={geometry.line}
            fill="none"
            stroke="var(--service-accent-hover)"
            strokeWidth={2}
            strokeLinejoin="round"
            vectorEffect="non-scaling-stroke"
          />
        </Svg>
        <YLabel style={{ top: 0 }}>{axisValue(metric, geometry.max)}</YLabel>
        <YLabel style={{ top: "50%" }}>{axisValue(metric, geometry.max / 2)}</YLabel>
        <YLabel style={{ top: "100%" }}>0</YLabel>
        {point ? (
          <>
            <Cursor style={{ left: `${(point.x / VIEW_W) * 100}%` }} />
            <Dot style={{ left: `${(point.x / VIEW_W) * 100}%`, top: `${(point.y / VIEW_H) * 100}%` }} />
          </>
        ) : null}
      </Frame>
      <XLabels aria-hidden="true">
        <span>{formatDay(trend[0].date)}</span>
        {midDate && trend.length > 2 ? <span>{formatDay(midDate)}</span> : null}
        <span>{formatDay(trend[last].date)}</span>
      </XLabels>

      <DataDetails>
        <summary>Show daily values</summary>
        <DataTableWrap>
          <DataTable>
            <caption>
              <VisuallyHidden>Daily active users and tokens for {modelName}</VisuallyHidden>
            </caption>
            <thead>
              <tr>
                <th scope="col">Date (UTC)</th>
                <th scope="col">Active users</th>
                <th scope="col">Tokens</th>
              </tr>
            </thead>
            <tbody>
              {trend.map((day) => (
                <tr key={day.date}>
                  <td>{formatDay(day.date)}</td>
                  <td>{formatCount(day.activeUsers)}</td>
                  <td>{formatTokens(day.tokens)}</td>
                </tr>
              ))}
            </tbody>
          </DataTable>
        </DataTableWrap>
      </DataDetails>
    </div>
  );
}
