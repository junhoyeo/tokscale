"use client";

import {
  useEffect,
  useId,
  useMemo,
  useRef,
  useState,
  useSyncExternalStore,
  type KeyboardEvent,
  type PointerEvent,
} from "react";
import styled from "styled-components";
import { DitherCheckbox } from "dither-kit/DitherCheckbox";
import { Area } from "dither-kit/Area";
import { Grid } from "dither-kit/Grid";
import { XAxis } from "dither-kit/XAxis";
import { YAxis } from "dither-kit/YAxis";
import { defineCartesianChart } from "dither-kit/cartesian-root";
import { useChart } from "dither-kit/chart-context";
import { backingSize, paintColumn, resample } from "dither-kit/dither-paint";
import {
  clearRasterBuffer,
  createRasterBuffer,
  putRasterBuffer,
} from "dither-kit/raster";
import type { Seed } from "dither-kit/palette";
import { Select } from "@/components/ui/Select";
import { SOURCE_DISPLAY_NAMES } from "@/lib/constants";
import type { DailyContribution } from "@/lib/types";
import { formatCurrency, formatDate, formatNumber } from "@/lib/utils";
import { DitherSelectionGroup } from "./ditherControls";
import {
  ALL_USAGE_PROVIDERS,
  MAX_LEGEND_MODELS,
  aggregateDailyUsage,
  buildUsageChartData,
  getActiveTooltipRows,
  getUsageProviderTotals,
  providerColor,
  reverseUsageChartData,
  selectLegendModels,
  toTrailingAverage,
  type AggregatedUsageDay,
  type UsageProviderTotal,
  type UsageMetric,
  type UsageProviderFilter,
  type UsageProviderId,
  type UsageTooltipRow,
  type UsageView,
} from "./usageChartData";

export interface ProfileUsageChartProps {
  contributions: DailyContribution[];
  initialMetric?: UsageMetric;
  description?: string;
  averageWindowDays?: number;
  rangeStart?: string | null;
  rangeEnd?: string | null;
}

/**
 * Charts are the kit's cartesian engine, but the profile needs a fill that is
 * never animated (the data-terminal contract: no chart animation, nothing
 * performs an entrance) and repaints synchronously when a filter changes.
 * `defineCartesianChart` is the kit's own extension point: same root, same
 * scales, same `<Area>`/`<Grid>`/axis parts, with a painter on its engine
 * exports (`paintColumn`, `backingSize`, `resample`, `RasterBuffer`).
 */
const UsageAreaChart = defineCartesianChart("area", UsageDitherCanvas);

const CHART_MARGINS = { top: 12, right: 12, bottom: 26, left: 44 } as const;
const CHART_CELL = 2;
const MAX_TOOLTIP_MODELS = 8;
const MAX_TOOLTIP_PROVIDERS = 3;
const TOOLTIP_WIDTH = 320;
const TOOLTIP_GAP = 12;
const TOOLTIP_EDGE = 8;
const TOOLTIP_VIEWPORT_EDGE = 16;
const TOOLTIP_MAX_HEIGHT = 416;
const NEWEST_FIRST_STORAGE_KEY = "tokscale:usage-newest-first";
const CROSSHAIR_ALPHA = 0.55;

const subscribeUsageChartMounted = () => () => {};

type InteractionMode = "idle" | "hover" | "committed";

interface ProviderCostRow {
  provider: UsageProviderId;
  label: string;
  color: string;
  value: number;
}

function clamp(value: number, minimum: number, maximum: number): number {
  return Math.max(minimum, Math.min(maximum, value));
}

function providerName(provider: UsageProviderId): string {
  if (provider === "unattributed") return "Unattributed";
  return SOURCE_DISPLAY_NAMES[provider] ?? provider;
}

function formatMetric(value: number, metric: UsageMetric): string {
  return metric === "tokens" ? formatNumber(value) : formatCurrency(value);
}

function formatAxisValue(value: number, metric: UsageMetric): string {
  if (!Number.isFinite(value)) return "";
  if (metric === "cost") {
    if (value === 0) return "$0";
    if (value >= 1_000) return `$${(value / 1_000).toFixed(1)}k`;
    if (value >= 1) return `$${value.toFixed(0)}`;
    return `$${value.toFixed(2)}`;
  }
  if (value === 0) return "0";
  if (value >= 1_000_000_000) {
    return `${(value / 1_000_000_000).toFixed(value >= 10_000_000_000 ? 0 : 1)}B`;
  }
  if (value >= 1_000_000) {
    return `${(value / 1_000_000).toFixed(value >= 10_000_000 ? 0 : 1)}M`;
  }
  if (value >= 1_000) return `${(value / 1_000).toFixed(0)}k`;
  return value.toFixed(0);
}

function metricLabel(metric: UsageMetric): string {
  return metric === "tokens" ? "Tokens" : "Cost";
}

function viewLabel(view: UsageView, averageWindowDays: number): string {
  return view === "average" ? `${averageWindowDays}d average` : "Daily";
}

function formatAxisDate(value: string, compact: boolean): string {
  const full = formatDate(value);
  return compact ? full.replace(/, \d{4}$/, "") : full;
}

function nearestIndex(px: number, length: number, plotWidth: number): number {
  if (length <= 1 || plotWidth <= 0) return 0;
  const ratio = Math.max(0, Math.min(1, px / plotWidth));
  return Math.round(ratio * (length - 1));
}

function tooltipLeft(activeOffset: number, plotWidth: number): number {
  if (!(plotWidth > 0)) return TOOLTIP_EDGE;

  const width = Math.min(
    TOOLTIP_WIDTH,
    Math.max(0, plotWidth - TOOLTIP_EDGE * 2),
  );
  const activePixel = (activeOffset / 100) * plotWidth;
  const right = activePixel + TOOLTIP_GAP;
  const left = activePixel - TOOLTIP_GAP - width;
  const rightFits = right + width <= plotWidth - TOOLTIP_EDGE;
  const leftFits = left >= TOOLTIP_EDGE;
  const preferred = rightFits
    ? right
    : leftFits
      ? left
      : plotWidth - activePixel >= activePixel
        ? right
        : left;

  return clamp(
    preferred,
    TOOLTIP_EDGE,
    Math.max(TOOLTIP_EDGE, plotWidth - TOOLTIP_EDGE - width),
  );
}

function tooltipLabels(rows: readonly UsageTooltipRow[]): Map<string, string> {
  const counts = new Map<string, number>();
  for (const { series } of rows) {
    counts.set(series.label, (counts.get(series.label) ?? 0) + 1);
  }

  return new Map(
    rows.map(({ series }) => [
      series.id,
      (counts.get(series.label) ?? 0) > 1
        ? `${series.label} · ${series.providerLabel}`
        : series.label,
    ]),
  );
}

function getProviderCostRows(
  days: readonly AggregatedUsageDay[],
  providerTotals: readonly UsageProviderTotal[],
  providerFilter: UsageProviderFilter,
  activeIndex: number,
  view: UsageView,
  averageWindowDays: number,
): ProviderCostRow[] {
  return providerTotals
    .filter(
      ({ provider }) =>
        providerFilter === ALL_USAGE_PROVIDERS || provider === providerFilter,
    )
    .map(({ provider }) => {
      const rawValues = days.map(
        (day) =>
          day.providers.find((item) => item.provider === provider)?.cost ?? 0,
      );
      const values =
        view === "average"
          ? toTrailingAverage(rawValues, averageWindowDays)
          : rawValues;
      return {
        provider,
        label: providerName(provider),
        color: providerColor(provider),
        value: values[activeIndex] ?? 0,
      };
    })
    .filter(({ value }) => value >= 0.005)
    .sort(
      (left, right) =>
        right.value - left.value || left.label.localeCompare(right.label),
    );
}

/**
 * Synchronous dither painter for the kit's area root. It mirrors the kit's
 * own cartesian painter — each series is resampled to backing columns, then
 * `paintColumn` scatters its band — without the RAF entrance/tween loop, so a
 * scoped change lands in the same frame and nothing animates.
 */
function UsageDitherCanvas() {
  const ctx = useChart();
  const canvasRef = useRef<HTMLCanvasElement | null>(null);

  useEffect(() => {
    const canvas = canvasRef.current;
    if (!canvas || !ctx.ready) return;
    const context = canvas.getContext("2d", { willReadFrequently: true });
    if (!context) return;

    const { cols, rows } = backingSize(ctx.plot.width, ctx.plot.height, CHART_CELL);
    if (canvas.width !== cols || canvas.height !== rows) {
      canvas.width = cols;
      canvas.height = rows;
    }

    const frame = createRasterBuffer(cols, rows);
    clearRasterBuffer(frame);
    const stacked =
      ctx.stackType === "stacked" || ctx.stackType === "percent";
    const height = ctx.plot.height || 1;
    const layers: {
      seed: Seed;
      top: number[];
      floor: number[];
    }[] = [];

    for (const key of ctx.configKeys) {
      const band = ctx.bands[key];
      if (!band) continue;
      const top = resample(
        band.map((pair) => (ctx.y(pair[1]) / height) * (rows - 1)),
        cols,
      );
      const floor = resample(
        band.map((pair) => (ctx.y(pair[0]) / height) * (rows - 1)),
        cols,
      );
      const seed = ctx.seedOf(key);
      const variant = ctx.seriesSpecs[key]?.variant ?? "gradient";
      layers.push({ seed, top, floor });
      for (let x = 0; x < cols; x += 1) {
        paintColumn(frame, x, top[x] ?? 0, floor[x] ?? 0, seed, {
          variant,
          intensity: 0,
          dim: 1,
          stacked,
          sparse: 0,
        });
      }
    }

    const marker = ctx.hoverIndex ?? ctx.markerIndex;
    if (marker != null && ctx.dataLength > 1) {
      const markerX = Math.round((marker / (ctx.dataLength - 1)) * (cols - 1));
      if (markerX >= 0 && markerX < cols) {
        for (const layer of layers) {
          const markerY = Math.round(layer.top[markerX] ?? 0);
          for (let y = Math.max(0, markerY); y < rows; y += 1) {
            const index = (y * cols + markerX) * 4;
            const alpha = CROSSHAIR_ALPHA;
            frame.data[index] = (layer.seed.fill[0] * alpha + 0.5) | 0;
            frame.data[index + 1] = (layer.seed.fill[1] * alpha + 0.5) | 0;
            frame.data[index + 2] = (layer.seed.fill[2] * alpha + 0.5) | 0;
            frame.data[index + 3] = (alpha * 255 + 0.5) | 0;
          }
          for (let dy = -1; dy <= 1; dy += 1) {
            for (let dx = -1; dx <= 1; dx += 1) {
              const px = markerX + dx;
              const py = markerY + dy;
              if (px < 0 || px >= cols || py < 0 || py >= rows) continue;
              const index = (py * cols + px) * 4;
              frame.data[index] = layer.seed.fill[0];
              frame.data[index + 1] = layer.seed.fill[1];
              frame.data[index + 2] = layer.seed.fill[2];
              frame.data[index + 3] = 255;
            }
          }
        }
      }
    }

    putRasterBuffer(context, frame, undefined);
  }, [ctx]);

  return (
    <canvas
      ref={canvasRef}
      aria-hidden="true"
      style={{
        position: "absolute",
        left: `${ctx.margins.left}px`,
        top: `${ctx.margins.top}px`,
        width: `${ctx.plot.width}px`,
        height: `${ctx.plot.height}px`,
        imageRendering: "pixelated",
        pointerEvents: "none",
      }}
    />
  );
}

const Section = styled.section`
  min-width: 0;
  overflow: visible;
  color: var(--service-text);
  background: var(--service-surface);
  border: 1px solid var(--service-border);
  border-radius: var(--radius);
  container-type: inline-size;
`;

const Header = styled.div`
  display: flex;
  align-items: flex-start;
  justify-content: space-between;
  gap: 1rem;
  padding: 0.875rem 1rem 0.75rem;
  border-bottom: 1px solid var(--service-border);

  @container (max-width: 34rem) {
    align-items: stretch;
    flex-direction: column;
    gap: 0.5rem;
  }
`;

const HeadingGroup = styled.div`
  min-width: 0;
`;

const Heading = styled.h2`
  margin: 0;
  font-size: 0.9375rem;
  font-weight: 600;
  letter-spacing: -0.01em;
  color: var(--service-text);
`;

const Description = styled.p`
  max-width: 46ch;
  margin: 0.25rem 0 0;
  font-size: 0.8125rem;
  line-height: 1.5;
  color: var(--service-text-muted);
`;

const Total = styled.div`
  flex: 0 0 auto;
  text-align: right;
  font-variant-numeric: tabular-nums;

  @container (max-width: 34rem) {
    display: flex;
    align-items: baseline;
    justify-content: space-between;
    gap: 0.75rem;
    text-align: left;
  }
`;

const TotalLabel = styled.div`
  font-size: 0.75rem;
  color: var(--service-text-muted);
`;

const TotalValue = styled.div`
  margin-top: 0.125rem;
  font-size: 1.125rem;
  font-weight: 600;
  color: var(--service-text);
`;

const Controls = styled.div`
  display: flex;
  align-items: center;
  justify-content: space-between;
  flex-wrap: wrap;
  gap: 0.5rem 1rem;
  padding: 0.625rem 1rem;
`;

const ControlCluster = styled.div`
  display: flex;
  align-items: center;
  flex-wrap: wrap;
  gap: 0.5rem 0.75rem;

  button.profile-checkbox span {
    font-size: 0.8125rem;
  }
`;

/** The Select trigger is the bordered control; a prefix names the setting. */
const CompactSelect = styled(Select)`
  --select-height: 2rem;
  --select-font-size: 0.8125rem;
  --select-padding-x: 0.625rem;
  --select-background: var(--service-surface-muted);
  --select-border-color: var(--service-border);
  --select-max-width: 15rem;
`;

const PlotRegion = styled.div`
  position: relative;
  min-width: 0;
  padding: 0 1rem;

  @container (max-width: 34rem) {
    padding-right: 0.75rem;
    padding-left: 0.75rem;
  }
`;

const Plot = styled.div`
  position: relative;
  min-width: 0;
  height: 16rem;
  touch-action: pan-y;

  @container (max-width: 34rem) {
    height: 14rem;
  }

  @container (min-width: 70rem) {
    height: 18rem;
  }

  svg text {
    font-size: 12px;
  }
`;

const InteractionLayer = styled.div`
  position: absolute;
  inset: 0;
  z-index: 2;
  cursor: crosshair;

  &:focus-visible {
    outline: 2px solid var(--service-focus);
    outline-offset: 2px;
  }
`;

const ChartMount = styled.div`
  position: absolute;
  inset: 0;
`;

const EmptyState = styled.div`
  display: grid;
  min-height: 16rem;
  place-items: center;
  color: var(--service-text-muted);
  font-size: 0.8125rem;

  @container (max-width: 34rem) {
    min-height: 14rem;
  }
`;

const DateRangeRow = styled.div`
  padding: 0.375rem 0.25rem 0.625rem;
  color: var(--service-text-muted);
  font-size: 0.75rem;
  font-variant-numeric: tabular-nums;
`;

const Legend = styled.ul`
  display: flex;
  align-items: center;
  flex-wrap: wrap;
  gap: 0.375rem 1rem;
  margin: 0;
  padding: 0 1rem 0.75rem;
  color: var(--service-text-muted);
  list-style: none;
`;

const LegendItem = styled.li`
  display: inline-flex;
  align-items: center;
  min-width: 0;
  gap: 0.375rem;
  font-size: 0.75rem;
`;

const Swatch = styled.span<{ $color: string }>`
  flex: 0 0 auto;
  width: 0.5rem;
  height: 0.5rem;
  background: ${(props) => props.$color};
  border-radius: 1px;
`;

const TooltipSurface = styled.div<{ $left: number; $maxHeight: number }>`
  position: absolute;
  z-index: 5;
  top: 0.5rem;
  left: ${(props) => props.$left}px;
  box-sizing: border-box;
  width: min(20rem, calc(100% - 1rem));
  max-height: ${(props) => props.$maxHeight}px;
  overflow-x: hidden;
  overflow-y: auto;
  padding: 0.625rem 0.75rem;
  color: var(--service-text);
  background: var(--service-surface-muted);
  border: 1px solid var(--service-border-strong);
  border-radius: var(--radius);
  box-shadow: 0 16px 40px rgb(0 0 0 / 0.4);
  overscroll-behavior: contain;
  scrollbar-width: thin;

  @container (max-width: 34rem) {
    display: none;
  }

  @media (pointer: coarse) {
    display: none;
  }
`;

const BreakdownHeader = styled.div`
  display: flex;
  align-items: baseline;
  justify-content: space-between;
  gap: 1rem;
  margin-bottom: 0.375rem;
  font-size: 0.8125rem;
`;

const BreakdownDate = styled.span`
  color: var(--service-text);
  font-weight: 600;
  font-variant-numeric: tabular-nums;
`;

const BreakdownMode = styled.span`
  color: var(--service-text-muted);
`;

const BreakdownList = styled.ul`
  display: grid;
  gap: 0.25rem;
  margin: 0;
  padding: 0;
  list-style: none;
`;

const BreakdownRow = styled.li`
  display: grid;
  grid-template-columns: auto minmax(0, 1fr) auto;
  align-items: center;
  gap: 0.375rem;
  color: var(--service-text-muted);
  font-size: 0.8125rem;
  line-height: 1.125rem;
`;

const BreakdownName = styled.span`
  overflow: hidden;
  text-overflow: ellipsis;
  white-space: nowrap;
`;

const BreakdownValue = styled.span`
  color: var(--service-text);
  font-variant-numeric: tabular-nums;
`;

const MoreRow = styled.div`
  display: flex;
  justify-content: space-between;
  gap: 1rem;
  margin-top: 0.25rem;
  color: var(--service-text-muted);
  font-size: 0.8125rem;
  font-variant-numeric: tabular-nums;
`;

const CostSection = styled.div`
  display: grid;
  gap: 0.25rem;
  margin-top: 0.5rem;
  padding-top: 0.5rem;
  border-top: 1px solid var(--service-border);
`;

const CostHeading = styled.div`
  color: var(--service-text-muted);
  font-size: 0.75rem;
`;

const BreakdownTotal = styled.div`
  display: flex;
  justify-content: space-between;
  gap: 1rem;
  margin-top: 0.5rem;
  padding-top: 0.5rem;
  color: var(--service-text);
  border-top: 1px solid var(--service-border);
  font-size: 0.8125rem;
  font-weight: 600;
  font-variant-numeric: tabular-nums;
`;

const PinnedBreakdown = styled.div`
  display: none;
  padding: 0.75rem 1rem 0.875rem;
  border-top: 1px solid var(--service-border);

  @container (max-width: 34rem) {
    display: block;
  }

  @media (pointer: coarse) {
    display: block;
  }
`;

const VisuallyHidden = styled.span`
  position: absolute;
  width: 1px;
  height: 1px;
  padding: 0;
  margin: -1px;
  overflow: hidden;
  clip: rect(0, 0, 0, 0);
  white-space: nowrap;
  border: 0;
`;

interface BreakdownProps {
  date: string;
  mode: string;
  metric: UsageMetric;
  rows: UsageTooltipRow[];
  providerCosts: ProviderCostRow[];
  total: number;
}

function BreakdownContent({
  date,
  mode,
  metric,
  rows,
  providerCosts,
  total,
}: BreakdownProps) {
  const labels = tooltipLabels(rows);
  const visibleRows = rows.slice(0, MAX_TOOLTIP_MODELS);
  const hiddenRows = rows.slice(MAX_TOOLTIP_MODELS);
  const hiddenValue = hiddenRows.reduce((sum, row) => sum + row.value, 0);
  const visibleCosts = providerCosts.slice(0, MAX_TOOLTIP_PROVIDERS);
  const hiddenCosts = providerCosts.slice(MAX_TOOLTIP_PROVIDERS);

  return (
    <>
      <BreakdownHeader>
        <BreakdownDate>{date}</BreakdownDate>
        <BreakdownMode>{mode}</BreakdownMode>
      </BreakdownHeader>
      <BreakdownList>
        {visibleRows.map(({ series, value }) => {
          const name = labels.get(series.id) ?? series.label;
          return (
            <BreakdownRow key={series.id}>
              <Swatch $color={series.color} aria-hidden="true" />
              <BreakdownName title={name}>{name}</BreakdownName>
              <BreakdownValue>{formatMetric(value, metric)}</BreakdownValue>
            </BreakdownRow>
          );
        })}
      </BreakdownList>
      {hiddenRows.length > 0 && (
        <MoreRow>
          <span>+{hiddenRows.length} more models</span>
          <span>{formatMetric(hiddenValue, metric)}</span>
        </MoreRow>
      )}
      {metric === "tokens" && providerCosts.length > 0 && (
        <CostSection>
          <CostHeading>Cost by provider</CostHeading>
          <BreakdownList>
            {visibleCosts.map((row) => (
              <BreakdownRow key={row.provider}>
                <Swatch $color={row.color} aria-hidden="true" />
                <BreakdownName>{row.label}</BreakdownName>
                <BreakdownValue>{formatCurrency(row.value)}</BreakdownValue>
              </BreakdownRow>
            ))}
          </BreakdownList>
          {hiddenCosts.length > 0 && (
            <MoreRow>
              <span>+{hiddenCosts.length} more providers</span>
              <span>
                {formatCurrency(
                  hiddenCosts.reduce((sum, row) => sum + row.value, 0),
                )}
              </span>
            </MoreRow>
          )}
        </CostSection>
      )}
      <BreakdownTotal>
        <span>Total {metricLabel(metric).toLowerCase()}</span>
        <span>{formatMetric(total, metric)}</span>
      </BreakdownTotal>
    </>
  );
}

export function ProfileUsageChart({
  contributions,
  initialMetric = "tokens",
  description = "Model activity, grouped by coding provider.",
  averageWindowDays = 30,
  rangeStart = null,
  rangeEnd = null,
}: ProfileUsageChartProps) {
  const headingId = useId();
  const keyboardInstructionsId = useId();
  const [metric, setMetric] = useState<UsageMetric>(initialMetric);
  const [view, setView] = useState<UsageView>("average");
  const [providerFilter, setProviderFilter] =
    useState<UsageProviderFilter>(ALL_USAGE_PROVIDERS);
  // Reversal is resolved after mount so the server render and the first client
  // paint stay chronological (no hydration mismatch). A persisted explicit
  // choice is the only way to start newest-first.
  const isMounted = useSyncExternalStore(
    subscribeUsageChartMounted,
    () => true,
    () => false,
  );
  // Lazy initializer: reads once on the client; the server (and the hydration
  // render, via the isMounted gate below) always resolves chronological.
  const [storedNewestFirst, setStoredNewestFirst] = useState<boolean | null>(
    () => {
      if (typeof window === "undefined") return null;
      try {
        const stored = window.localStorage.getItem(NEWEST_FIRST_STORAGE_KEY);
        return stored === "1" ? true : stored === "0" ? false : null;
      } catch {
        // localStorage may be unavailable (private mode / disabled).
        return null;
      }
    },
  );
  const newestFirst = isMounted && storedNewestFirst === true;
  const commitNewestFirst = (next: boolean) => {
    setStoredNewestFirst(next);
    try {
      window.localStorage.setItem(NEWEST_FIRST_STORAGE_KEY, next ? "1" : "0");
    } catch {
      // Ignore persistence failures; the in-memory choice still applies.
    }
  };
  const [activeDate, setActiveDate] = useState<string | null>(null);
  const [announcedDate, setAnnouncedDate] = useState<string | null>(null);
  const [interactionMode, setInteractionMode] =
    useState<InteractionMode>("idle");
  const plotRef = useRef<HTMLDivElement>(null);
  const [plotWidth, setPlotWidth] = useState(0);
  const [tooltipMaxHeight, setTooltipMaxHeight] = useState(TOOLTIP_MAX_HEIGHT);

  const days = useMemo(
    () =>
      aggregateDailyUsage(
        contributions,
        rangeStart ?? undefined,
        rangeEnd ?? undefined,
      ),
    [contributions, rangeStart, rangeEnd],
  );
  const providerTotals = useMemo(
    () => getUsageProviderTotals(days, metric),
    [days, metric],
  );
  const costProviderTotals = useMemo(
    () => getUsageProviderTotals(days, "cost"),
    [days],
  );
  const selectedProvider =
    providerFilter === ALL_USAGE_PROVIDERS ||
    providerTotals.some(({ provider }) => provider === providerFilter)
      ? providerFilter
      : ALL_USAGE_PROVIDERS;
  const chronologicalChartData = useMemo(
    () =>
      buildUsageChartData(
        days,
        metric,
        selectedProvider,
        view,
        averageWindowDays,
      ),
    [days, metric, selectedProvider, view, averageWindowDays],
  );
  // Everything below renders in visual order: with "Newest first" on, the
  // whole per-day pipeline (dates, series, totals) is mirrored once here so
  // pointer, keyboard, and tooltip index math needs no special cases.
  const chartData = useMemo(
    () =>
      newestFirst
        ? reverseUsageChartData(chronologicalChartData)
        : chronologicalChartData,
    [chronologicalChartData, newestFirst],
  );
  const chartRows = useMemo(
    () =>
      chartData.dates.map((date, index) => {
        const row: Record<string, unknown> = { date };
        for (const series of chartData.series) {
          row[series.id] = series.values[index] ?? 0;
        }
        return row;
      }),
    [chartData],
  );
  const chartConfig = useMemo(
    () =>
      Object.fromEntries(
        chartData.series.map((series) => [
          series.id,
          { label: series.label, color: series.color },
        ]),
      ),
    [chartData.series],
  );

  useEffect(() => {
    const plot = plotRef.current;
    if (!plot) return;

    const measure = () => {
      const bounds = plot.getBoundingClientRect();
      const availableHeight =
        window.innerHeight -
        Math.max(
          TOOLTIP_EDGE,
          bounds.top + TOOLTIP_EDGE + TOOLTIP_VIEWPORT_EDGE,
        );
      setPlotWidth((current) => (current === bounds.width ? current : bounds.width));
      setTooltipMaxHeight((current) => {
        const next = clamp(availableHeight, 0, TOOLTIP_MAX_HEIGHT);
        return current === next ? current : next;
      });
    };
    measure();

    window.addEventListener("resize", measure);
    window.addEventListener("scroll", measure, true);

    if (typeof ResizeObserver === "undefined") {
      return () => {
        window.removeEventListener("resize", measure);
        window.removeEventListener("scroll", measure, true);
      };
    }

    const observer = new ResizeObserver(measure);
    observer.observe(plot);
    return () => {
      observer.disconnect();
      window.removeEventListener("resize", measure);
      window.removeEventListener("scroll", measure, true);
    };
  }, [chartData.dates.length]);

  const plotPixelWidth = Math.max(
    0,
    plotWidth - CHART_MARGINS.left - CHART_MARGINS.right,
  );
  const requestedActiveIndex = activeDate
    ? chartData.dates.indexOf(activeDate)
    : -1;
  // The idle inspection target is always the newest day, whichever edge it
  // renders on.
  const activeIndex =
    requestedActiveIndex >= 0
      ? requestedActiveIndex
      : newestFirst
        ? 0
        : chartData.dates.length - 1;
  const currentDate = chartData.dates[activeIndex] ?? null;
  const currentTotal = chartData.dailyTotals[activeIndex] ?? 0;
  const activeRows = useMemo(
    () => getActiveTooltipRows(chartData.series, activeIndex),
    [chartData.series, activeIndex],
  );
  // `days` stays chronological, so the visual index is mapped back before
  // indexing per-provider cost values.
  const chronologicalActiveIndex = newestFirst
    ? chartData.dates.length - 1 - activeIndex
    : activeIndex;
  const providerCostRows = useMemo(
    () =>
      getProviderCostRows(
        days,
        costProviderTotals,
        selectedProvider,
        chronologicalActiveIndex,
        view,
        chartData.averageWindowDays,
      ),
    [
      days,
      costProviderTotals,
      selectedProvider,
      chronologicalActiveIndex,
      view,
      chartData.averageWindowDays,
    ],
  );

  const modelLegend = useMemo(
    () => selectLegendModels(chartData.series, MAX_LEGEND_MODELS),
    [chartData.series],
  );

  const setActiveIndex = (index: number, announce = false) => {
    if (chartData.dates.length === 0) return;
    const nextIndex = Math.max(0, Math.min(chartData.dates.length - 1, index));
    const date = chartData.dates[nextIndex];
    setActiveDate(date);
    if (announce) setAnnouncedDate(date);
  };

  const handlePointer = (
    event: PointerEvent<HTMLDivElement>,
    announce = false,
  ) => {
    if (chartData.dates.length === 0) return;
    const bounds = event.currentTarget.getBoundingClientRect();
    const pointer = event.clientX - bounds.left - CHART_MARGINS.left;
    setActiveIndex(
      nearestIndex(pointer, chartData.dates.length, plotPixelWidth),
      announce,
    );
  };

  const handleKeyDown = (event: KeyboardEvent<HTMLDivElement>) => {
    if (chartData.dates.length === 0) return;

    switch (event.key) {
      case "ArrowLeft":
        event.preventDefault();
        setInteractionMode("committed");
        setActiveIndex(activeIndex - 1, true);
        break;
      case "ArrowRight":
        event.preventDefault();
        setInteractionMode("committed");
        setActiveIndex(activeIndex + 1, true);
        break;
      case "Home":
        event.preventDefault();
        setInteractionMode("committed");
        setActiveIndex(0, true);
        break;
      case "End":
        event.preventDefault();
        setInteractionMode("committed");
        setActiveIndex(chartData.dates.length - 1, true);
        break;
      case "Escape":
        event.preventDefault();
        setInteractionMode("idle");
        setAnnouncedDate(null);
        break;
    }
  };

  const activeOffset =
    chartData.dates.length > 1
      ? (activeIndex / (chartData.dates.length - 1)) * 100
      : 0;
  const activeTooltipLeft = tooltipLeft(activeOffset, plotPixelWidth);
  const modeLabel = viewLabel(view, chartData.averageWindowDays);
  const announcedIndex = announcedDate
    ? chartData.dates.indexOf(announcedDate)
    : -1;
  const announcedRows =
    announcedIndex >= 0
      ? getActiveTooltipRows(chartData.series, announcedIndex)
      : [];
  const announcedLabels = tooltipLabels(announcedRows);
  const announcement =
    announcedIndex >= 0
      ? `${formatDate(chartData.dates[announcedIndex])}, ${modeLabel}: ${formatMetric(
          chartData.dailyTotals[announcedIndex] ?? 0,
          metric,
        )}. ${announcedRows
          .slice(0, 3)
          .map(
            ({ series, value }) =>
              `${announcedLabels.get(series.id) ?? series.label} ${formatMetric(value, metric)}`,
          )
          .join(", ")}${
          announcedRows.length > 3
            ? `, plus ${announcedRows.length - 3} more models`
            : ""
        }`
      : "";
  const isInspecting = interactionMode !== "idle" && currentDate !== null;
  const showChart = isMounted && plotWidth > 0;

  return (
    <Section aria-labelledby={headingId}>
      <Header>
        <HeadingGroup>
          <Heading id={headingId}>Usage over time</Heading>
          <Description>{description}</Description>
        </HeadingGroup>
        <Total>
          <TotalLabel>
            Range total {metricLabel(metric).toLowerCase()}
          </TotalLabel>
          <TotalValue title={chartData.total.toLocaleString("en-US")}>
            {formatMetric(chartData.total, metric)}
          </TotalValue>
        </Total>
      </Header>

      <Controls>
        <ControlCluster>
          <DitherSelectionGroup
            label="Usage metric"
            value={metric}
            options={[
              { value: "tokens", label: "Tokens" },
              { value: "cost", label: "Cost" },
            ]}
            onChange={(next) => setMetric(next as UsageMetric)}
          />

          <CompactSelect
            prefix="Display"
            aria-label="Usage display"
            value={view}
            options={[
              {
                value: "average",
                label: `${chartData.averageWindowDays}d average`,
              },
              { value: "daily", label: "Daily" },
            ]}
            onChange={(next) => setView(next as UsageView)}
          />
        </ControlCluster>

        <ControlCluster>
          <DitherCheckbox
            value={newestFirst}
            onChange={commitNewestFirst}
            color="#2f8fff"
            className="profile-checkbox"
          >
            Newest first
          </DitherCheckbox>
          <CompactSelect
            prefix="Provider"
            name="profile-usage-provider"
            aria-label="Usage provider"
            value={selectedProvider}
            options={[
              { value: ALL_USAGE_PROVIDERS, label: "All" },
              ...providerTotals.map(({ provider }) => ({
                value: provider,
                label: providerName(provider),
              })),
            ]}
            onChange={(next) => setProviderFilter(next as UsageProviderFilter)}
          />
        </ControlCluster>
      </Controls>

      <PlotRegion>
        {chartData.dates.length > 0 ? (
          <Plot ref={plotRef}>
            {showChart && (
              <ChartMount>
                <UsageAreaChart
                  data={chartRows}
                  config={chartConfig}
                  stackType="stacked"
                  margins={CHART_MARGINS}
                  animate={false}
                  sparkles={false}
                  hoverLift={false}
                  interactive={false}
                  markerIndex={isInspecting ? activeIndex : null}
                >
                  <Grid
                    horizontal
                    strokeDasharray="3 3"
                    tickCount={4}
                  />
                  <YAxis
                    tickCount={4}
                    tickFormatter={(value) => formatAxisValue(value, metric)}
                  />
                  <XAxis
                    dataKey="date"
                    maxTicks={plotPixelWidth < 480 ? 4 : 6}
                    tickFormatter={(value) =>
                      formatAxisDate(String(value), plotPixelWidth < 480)
                    }
                  />
                  {chartData.series.map((series) => (
                    <Area key={series.id} dataKey={series.id} />
                  ))}
                </UsageAreaChart>
              </ChartMount>
            )}
            <InteractionLayer
              tabIndex={0}
              role="group"
              aria-describedby={keyboardInstructionsId}
              aria-label={`Interactive ${metricLabel(metric).toLowerCase()} chart, ${chartData.dates.length} days`}
              onKeyDown={handleKeyDown}
              onPointerMove={(event) => {
                if (event.pointerType === "mouse") {
                  setInteractionMode("hover");
                  handlePointer(event);
                }
              }}
              onPointerLeave={() =>
                setInteractionMode((mode) => (mode === "hover" ? "idle" : mode))
              }
              onPointerDown={(event) => {
                setInteractionMode("committed");
                handlePointer(event, true);
              }}
            />

            {isInspecting && currentDate && (
              <TooltipSurface
                role="tooltip"
                data-profile-usage-tooltip
                $left={activeTooltipLeft}
                $maxHeight={tooltipMaxHeight}
                onPointerMove={(event) => event.stopPropagation()}
                onPointerDown={(event) => event.stopPropagation()}
              >
                <BreakdownContent
                  date={currentDate}
                  mode={modeLabel}
                  metric={metric}
                  rows={activeRows}
                  providerCosts={providerCostRows}
                  total={currentTotal}
                />
              </TooltipSurface>
            )}
          </Plot>
        ) : (
          <EmptyState>No usage data in this range.</EmptyState>
        )}

        {chartData.dates.length > 0 && (
          <DateRangeRow role="group" aria-label="Chart date range">
            <span>
              {formatDate(chartData.dates[0])}
              {chartData.dates.length > 1
                ? ` – ${formatDate(chartData.dates.at(-1) as string)}`
                : ""}
            </span>
          </DateRangeRow>
        )}
        <VisuallyHidden id={keyboardInstructionsId}>
          Use Left Arrow and Right Arrow to inspect adjacent days. Use Home and
          End to jump to the first or last day. Press Escape to close the
          inspection.
        </VisuallyHidden>
      </PlotRegion>

      {modelLegend.visible.length > 0 && (
        <Legend role="list" aria-label="Usage models">
          {modelLegend.visible.map((entry) => (
            <LegendItem key={entry.id}>
              <Swatch $color={entry.color} aria-hidden="true" />
              <span>{entry.label}</span>
            </LegendItem>
          ))}
          {modelLegend.hiddenCount > 0 && (
            <LegendItem>
              <span>+{modelLegend.hiddenCount} more</span>
            </LegendItem>
          )}
        </Legend>
      )}

      {interactionMode === "committed" && currentDate && (
        <PinnedBreakdown aria-label={`Usage on ${currentDate}`}>
          <BreakdownContent
            date={currentDate}
            mode={modeLabel}
            metric={metric}
            rows={activeRows}
            providerCosts={providerCostRows}
            total={currentTotal}
          />
        </PinnedBreakdown>
      )}
      <VisuallyHidden role="status" aria-live="polite" aria-atomic="true">
        {announcement}
      </VisuallyHidden>
    </Section>
  );
}

