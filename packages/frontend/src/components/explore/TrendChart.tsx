"use client";

/**
 * Daily usage trend for one model: a dither-kit area chart in the site's one
 * data hue, with dates on the x axis and a compact count scale.
 *
 * Accessibility: the chart frame is a focusable group. Arrow keys move the day
 * (Shift moves a week, Home and End jump to the ends) and drive the kit's
 * crosshair through `markerIndex`; pointer hover moves the same day through
 * `onHoverChange`. A polite readout above the chart announces the day, and the
 * full series is a table under "Show daily values". The canvas is decorative.
 */
import { useId, useMemo, useState, type KeyboardEvent } from "react";
import styled from "styled-components";
import { AreaChart } from "dither-kit/area-chart";
import { Area } from "dither-kit/Area";
import { Grid } from "dither-kit/Grid";
import { Tooltip } from "dither-kit/Tooltip";
import { XAxis } from "dither-kit/XAxis";
import { YAxis } from "dither-kit/YAxis";
import { CHART_PROPS, ChartBox, chartFrame, compactCount } from "./ExploreCharts";
import { ButtonSegments, SectionNote, SectionTitle, VisuallyHidden } from "./ExploreUI";
import { formatCount, formatDay, formatTokens } from "./format";
import { useMediaQuery } from "@/lib/useMediaQuery";

type Metric = "activeUsers" | "tokens";

/** Room for the y scale on the left and the dates underneath the plot. */
const CHART_MARGINS = { top: 12, right: 16, bottom: 28, left: 44 };

/** One series per metric, drawn in the site's data hue. */
const METRIC_CONFIG: Record<Metric, Record<string, { label: string; color: string }>> = {
  activeUsers: { activeUsers: { label: "Active users", color: "blue" } },
  tokens: { tokens: { label: "Tokens", color: "blue" } },
};

const Head = styled.div`
  display: flex;
  flex-wrap: wrap;
  align-items: flex-end;
  justify-content: space-between;
  gap: 12px;
`;

const Readout = styled.p`
  min-height: 1.5em;
  margin: 12px 0;
  color: var(--service-text);
  font-size: 0.875rem;
  font-variant-numeric: tabular-nums;

  > span {
    color: var(--service-text-muted);
  }
`;

/* The focusable group the keys drive; the kit root inside it tracks the pointer. */
const Frame = styled.div`
  position: relative;
  cursor: crosshair;
  touch-action: pan-y;

  &:focus-visible {
    border-radius: 2px;
    outline: 2px solid var(--service-focus);
    outline-offset: 4px;
  }
`;

const Plot = styled(ChartBox)`
  @media (max-width: 640px) {
    height: 180px;
  }
`;

const PlotFrame = styled.div`
  ${chartFrame}
`;

const DataDetails = styled.details`
  margin-top: 8px;

  > summary {
    display: inline-flex;
    min-height: 32px;
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
    height: 32px;
    padding: 0 0 0 16px;
    border-bottom: 1px solid var(--service-border);
    text-align: right;
  }

  th:first-child,
  td:first-child {
    padding-left: 0;
    text-align: left;
  }

  th {
    position: sticky;
    top: 0;
    background: var(--service-canvas);
    color: var(--service-text-muted);
    font-weight: 400;
  }

  td {
    color: var(--service-text);
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

/** The readout, peak and tooltip format: exact users, compact tokens. */
function metricValue(metric: Metric, value: number): string {
  return metric === "tokens" ? formatTokens(value) : formatCount(value);
}

/** Narrower plots: the all-time ticks show the year, so they get fewer of them. */
const NARROW_PLOT = 520;

/** At least this many days: the tick carries its year, or dates from two years read out of order. */
const LONG_SPAN_DAYS = 330;

/**
 * "Mon D" for a window inside one year, "Mon 2025" where an all-time axis has
 * no room for the day, and "Mon D, YYYY" everywhere else.
 */
function dateTick(date: unknown, longSpan: boolean, narrow: boolean): string {
  const day = formatDay(String(date));
  if (!longSpan) return day.replace(/, \d{4}$/, "");
  return narrow ? day.replace(/^(\w+) \d+, /, "$1 ") : day;
}

export function TrendChart({ trend, modelName }: { trend: TrendPoint[]; modelName: string }) {
  const [metric, setMetric] = useState<Metric>("activeUsers");
  const [index, setIndex] = useState<number | null>(null);
  const titleId = useId();
  const helpId = `${titleId}-help`;
  const narrow = useMediaQuery(`(max-width: ${NARROW_PLOT}px)`);

  // Each row carries both metrics; the config picks the one series to draw. `day` heads the tooltip.
  const data = useMemo(() => trend.map((point) => ({ ...point, day: formatDay(point.date) })), [trend]);

  if (trend.length === 0) {
    return (
      <div>
        <SectionTitle>Daily usage</SectionTitle>
        <SectionNote>No daily data in this period.</SectionNote>
      </div>
    );
  }

  const last = trend.length - 1;
  // Clamped: a period switch can shorten the series under a kept index.
  const active = index === null ? last : Math.min(index, last);
  const activeDay = trend[active];
  const peak = trend.reduce((max, point) => Math.max(max, point[metric]), 0);
  const longSpan = trend.length > LONG_SPAN_DAYS;
  /*
   * Whole-number ticks: a count at most the peak keeps the tick step at 1, so a
   * series of 3 users reads 0/1/2/3 and never 0.2/0.4. Every real series asks
   * for the full four; a tiny one asks for fewer rather than fractions.
   */
  const tickCount = Math.min(4, Math.max(1, Math.ceil(peak)));

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
        {`${metricValue(metric, activeDay[metric])} ${metricLabel}`} <span>on {formatDay(activeDay.date)}</span>
      </Readout>

      <Frame role="group" tabIndex={0} aria-labelledby={titleId} aria-describedby={helpId} onKeyDown={onKey}>
        <VisuallyHidden id={helpId}>
          {`Daily ${metricLabel} for ${modelName}, ${formatDay(trend[0].date)} to ${formatDay(trend[last].date)}. Peak ${metricValue(metric, peak)}. Use the left and right arrow keys to read each day.`}
        </VisuallyHidden>
        <Plot $height={220} aria-hidden="true">
          <PlotFrame>
            <AreaChart
              {...CHART_PROPS}
              data={data}
              config={METRIC_CONFIG[metric]}
              margins={CHART_MARGINS}
              markerIndex={active}
              onHoverChange={setIndex}
            >
              <Grid horizontal tickCount={tickCount} />
              <XAxis
                dataKey="date"
                maxTicks={longSpan && narrow ? 4 : 5}
                tickFormatter={(date) => dateTick(date, longSpan, narrow)}
              />
              <YAxis tickCount={tickCount} tickFormatter={compactCount} />
              <Area dataKey={metric} />
              <Tooltip labelKey="day" valueFormatter={(value) => metricValue(metric, value)} />
            </AreaChart>
          </PlotFrame>
        </Plot>
      </Frame>

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
