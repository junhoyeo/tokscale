"use client";

import {
  useEffect,
  useId,
  useMemo,
  useRef,
  useState,
  type KeyboardEvent,
  type PointerEvent,
} from "react";
import styled, { css } from "styled-components";
import { SOURCE_DISPLAY_NAMES } from "@/lib/constants";
import type { DailyContribution } from "@/lib/types";
import { formatCurrency, formatDate, formatNumber } from "@/lib/utils";
import {
  ALL_USAGE_PROVIDERS,
  aggregateDailyUsage,
  buildUsageChartData,
  getActiveTooltipRows,
  getUsageProviderTotals,
  providerColor,
  toTrailingAverage,
  type UsageChartSeries,
  type UsageMetric,
  type UsageProviderFilter,
  type UsageProviderId,
  type UsageTooltipRow,
  type UsageView,
} from "./usageChartData";
import {
  createNonCrossingStackGeometry,
  pointToChartPercent,
  type CubicValueBoundary,
} from "./usageChartGeometry";

export interface ProfileUsageChartProps {
  contributions: DailyContribution[];
  initialMetric?: UsageMetric;
  description?: string;
  averageWindowDays?: number;
  rangeStart?: string | null;
  rangeEnd?: string | null;
}

const VIEWBOX_WIDTH = 848;
const VIEWBOX_HEIGHT = 256;
const PLOT_LEFT = 0;
const PLOT_RIGHT = 0;
const PLOT_TOP = 8;
const PLOT_BOTTOM = 30;
const PLOT_WIDTH = VIEWBOX_WIDTH - PLOT_LEFT - PLOT_RIGHT;
const PLOT_HEIGHT = VIEWBOX_HEIGHT - PLOT_TOP - PLOT_BOTTOM;
const GRID_STEPS = 4;
const MAX_TOOLTIP_MODELS = 8;
const MAX_TOOLTIP_PROVIDERS = 3;
const TOOLTIP_WIDTH = 320;
const TOOLTIP_GAP = 12;
const TOOLTIP_EDGE = 8;
const TOOLTIP_VIEWPORT_EDGE = 16;
const TOOLTIP_MAX_HEIGHT = 416;

type InteractionMode = "idle" | "hover" | "committed";

interface ChartLayer {
  series: UsageChartSeries;
  areaPath: string;
  linePath: string;
  upperValues: number[];
}

interface ChartStack {
  layers: ChartLayer[];
  maximum: number;
}

interface ProviderCostRow {
  provider: UsageProviderId;
  label: string;
  color: string;
  value: number;
}

function xForIndex(index: number, pointCount: number): number {
  if (pointCount <= 1) return PLOT_LEFT + PLOT_WIDTH / 2;
  return PLOT_LEFT + (index / (pointCount - 1)) * PLOT_WIDTH;
}

function yForValue(value: number, maximum: number): number {
  const safeMaximum = maximum > 0 ? maximum : 1;
  const finiteValue = Number.isFinite(value) ? Math.max(0, value) : 0;
  return PLOT_TOP + PLOT_HEIGHT - (finiteValue / safeMaximum) * PLOT_HEIGHT;
}

function clamp(value: number, minimum: number, maximum: number): number {
  return Math.max(minimum, Math.min(maximum, value));
}

function curvePath(boundary: CubicValueBoundary, maximum: number): string {
  const pointCount = boundary.values.length;
  if (pointCount === 0) return "";
  if (pointCount === 1) {
    const x = xForIndex(0, 1);
    const y = yForValue(boundary.values[0] ?? 0, maximum);
    return `M ${x - 4} ${y} L ${x + 4} ${y}`;
  }
  return [
    `M ${xForIndex(0, pointCount)} ${yForValue(boundary.values[0] ?? 0, maximum)}`,
    ...boundary.segments.map((segment) => {
      const fromX = xForIndex(segment.index, pointCount);
      const toX = xForIndex(segment.index + 1, pointCount);
      const third = (toX - fromX) / 3;
      return `C ${fromX + third} ${yForValue(segment.control1, maximum)} ${toX - third} ${yForValue(segment.control2, maximum)} ${toX} ${yForValue(segment.to, maximum)}`;
    }),
  ].join(" ");
}

function stackedAreaPath(
  lower: CubicValueBoundary,
  upper: CubicValueBoundary,
  maximum: number,
): string {
  const pointCount = upper.values.length;
  if (pointCount === 0) return "";
  if (pointCount === 1) {
    const x = xForIndex(0, 1);
    return [
      `M ${x - 4} ${yForValue(lower.values[0] ?? 0, maximum)}`,
      `L ${x - 4} ${yForValue(upper.values[0] ?? 0, maximum)}`,
      `L ${x + 4} ${yForValue(upper.values[0] ?? 0, maximum)}`,
      `L ${x + 4} ${yForValue(lower.values[0] ?? 0, maximum)}`,
      "Z",
    ].join(" ");
  }

  return [
    `M ${xForIndex(0, pointCount)} ${yForValue(upper.values[0] ?? 0, maximum)}`,
    ...upper.segments.map((segment) => {
      const fromX = xForIndex(segment.index, pointCount);
      const toX = xForIndex(segment.index + 1, pointCount);
      const third = (toX - fromX) / 3;
      return `C ${fromX + third} ${yForValue(segment.control1, maximum)} ${toX - third} ${yForValue(segment.control2, maximum)} ${toX} ${yForValue(segment.to, maximum)}`;
    }),
    `L ${xForIndex(pointCount - 1, pointCount)} ${yForValue(lower.values.at(-1) ?? 0, maximum)}`,
    ...[...lower.segments].reverse().map((segment) => {
      const fromX = xForIndex(segment.index, pointCount);
      const toX = xForIndex(segment.index + 1, pointCount);
      const third = (toX - fromX) / 3;
      return `C ${toX - third} ${yForValue(segment.control2, maximum)} ${fromX + third} ${yForValue(segment.control1, maximum)} ${fromX} ${yForValue(segment.from, maximum)}`;
    }),
    "Z",
  ].join(" ");
}

function createChartStack(
  series: readonly UsageChartSeries[],
  pointCount: number,
  baselineMaximum: number,
): ChartStack {
  const geometry = createNonCrossingStackGeometry(
    series.map(({ values }) => values),
    pointCount,
  );
  const maximum = Math.max(baselineMaximum, geometry.maximum);
  const layers = series.map((item, index) => {
    const layerGeometry = geometry.layers[index];
    if (!layerGeometry) {
      return {
        series: item,
        areaPath: "",
        linePath: "",
        upperValues: [],
      };
    }
    return {
      series: item,
      areaPath: stackedAreaPath(
        layerGeometry.lower,
        layerGeometry.upper,
        maximum,
      ),
      linePath: curvePath(layerGeometry.upper, maximum),
      upperValues: layerGeometry.upper.values,
    };
  });

  return { layers, maximum };
}

function providerName(provider: UsageProviderId): string {
  if (provider === "unattributed") return "Unattributed";
  return SOURCE_DISPLAY_NAMES[provider] ?? provider;
}

function formatMetric(value: number, metric: UsageMetric): string {
  return metric === "tokens" ? formatNumber(value) : formatCurrency(value);
}

function metricLabel(metric: UsageMetric): string {
  return metric === "tokens" ? "Tokens" : "Cost";
}

function viewLabel(view: UsageView, averageWindowDays: number): string {
  return view === "average" ? `${averageWindowDays}d average` : "Daily";
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
  days: ReturnType<typeof aggregateDailyUsage>,
  providerTotals: ReturnType<typeof getUsageProviderTotals>,
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

const Section = styled.section`
  min-width: 0;
  overflow: visible;
  color: var(--service-text);
  background: var(--service-surface);
  border: 1px solid var(--service-border);
  border-radius: 12px;
  container-type: inline-size;
`;

const Header = styled.div`
  display: flex;
  align-items: flex-start;
  justify-content: space-between;
  gap: 1rem;
  padding: 1rem 1rem 0.875rem;
  border-bottom: 1px solid var(--service-border);

  @container (max-width: 34rem) {
    align-items: stretch;
    flex-direction: column;
    gap: 0.75rem;
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
    text-align: left;
  }
`;

const TotalLabel = styled.div`
  font-size: 0.6875rem;
  color: var(--service-text-muted);
`;

const TotalValue = styled.div`
  margin-top: 0.125rem;
  font-size: 1rem;
  font-weight: 600;
  color: var(--service-text);
`;

const Controls = styled.div`
  display: flex;
  align-items: center;
  justify-content: space-between;
  flex-wrap: wrap;
  gap: 0.5rem;
  padding: 0.625rem 1rem;
`;

const ControlCluster = styled.div`
  display: flex;
  align-items: center;
  flex-wrap: wrap;
  gap: 0.5rem;
`;

const MetricControl = styled.div`
  display: inline-flex;
  align-items: center;
  padding: 0.125rem;
  background: var(--service-surface-muted);
  border: 1px solid var(--service-border);
  border-radius: 0.5rem;
`;

const MetricButton = styled.button<{ $active: boolean }>`
  position: relative;
  min-height: 1.75rem;
  padding: 0.25rem 0.625rem;
  color: var(--service-text-muted);
  background: transparent;
  border: 0;
  border-radius: 0.375rem;
  font: inherit;
  font-size: 0.75rem;
  font-weight: 500;
  cursor: pointer;

  ${(props) =>
    props.$active &&
    css`
      color: var(--service-accent);
      background: var(--service-accent-soft);
    `}

  &:focus-visible {
    outline: 2px solid var(--service-focus);
    outline-offset: 1px;
  }

  @media (pointer: coarse) {
    min-height: 2.75rem;
  }
`;

const SelectControl = styled.label`
  position: relative;
  display: inline-flex;
  align-items: center;
  gap: 0.375rem;
  min-height: 2rem;
  padding-left: 0.625rem;
  color: var(--service-text-muted);
  background: var(--service-surface-muted);
  border: 1px solid var(--service-border);
  border-radius: 0.5rem;
  font-size: 0.75rem;

  &::after {
    content: "";
    position: absolute;
    right: 0.625rem;
    width: 0.375rem;
    height: 0.375rem;
    border-right: 1px solid var(--service-text-muted);
    border-bottom: 1px solid var(--service-text-muted);
    transform: translateY(-0.125rem) rotate(45deg);
    pointer-events: none;
  }

  &:hover {
    border-color: var(--service-border-strong);
  }

  &:focus-within {
    outline: 2px solid var(--service-focus);
    outline-offset: 1px;
  }

  @media (pointer: coarse) {
    min-height: 2.75rem;
  }
`;

const SelectCaption = styled.span`
  flex: 0 0 auto;
`;

const CompactSelect = styled.select`
  min-width: 0;
  max-width: 11rem;
  height: 100%;
  padding: 0.25rem 1.75rem 0.25rem 0;
  appearance: none;
  color: var(--service-text);
  background: transparent;
  border: 0;
  outline: 0;
  font: inherit;
  font-weight: 500;
  cursor: pointer;
`;

const PlotRegion = styled.div`
  min-width: 0;
  padding: 0.5rem 1rem 0;

  @container (max-width: 34rem) {
    padding-right: 0.75rem;
    padding-left: 0.75rem;
  }
`;

const InteractivePlot = styled.div`
  position: relative;
  min-width: 0;
  height: 16rem;
  overflow: visible;
  touch-action: pan-y;
  cursor: crosshair;

  &:focus-visible {
    outline: 2px solid var(--service-focus);
    outline-offset: 2px;
  }

  @container (max-width: 34rem) {
    height: 14rem;
  }
`;

const ChartSvg = styled.svg`
  position: absolute;
  inset: 0;
  display: block;
  width: 100%;
  height: 100%;
  min-width: 0;
  overflow: visible;
`;

const GridLine = styled.line`
  stroke: rgb(255 255 255 / 0.035);
  stroke-width: 1;
  vector-effect: non-scaling-stroke;
`;

const LayerArea = styled.path<{ $color: string }>`
  fill: ${(props) => props.$color};
  fill-opacity: 0.4;
  stroke: none;
`;

const LayerLine = styled.path<{ $color: string }>`
  fill: none;
  stroke: ${(props) => props.$color};
  stroke-width: 1;
  stroke-opacity: 1;
  stroke-linecap: round;
  stroke-linejoin: round;
  vector-effect: non-scaling-stroke;
`;

const ActiveRule = styled.line`
  stroke: var(--service-border-strong);
  stroke-width: 1;
  vector-effect: non-scaling-stroke;
`;

const ActivePoint = styled.span<{
  $color: string;
  $left: number;
  $top: number;
}>`
  position: absolute;
  z-index: 2;
  top: ${(props) => props.$top}%;
  left: ${(props) => props.$left}%;
  width: 10px;
  height: 10px;
  background: var(--service-surface);
  border: 2px solid ${(props) => props.$color};
  border-radius: 50%;
  pointer-events: none;
  transform: translate(-50%, -50%);
`;

const DateRange = styled.div`
  position: absolute;
  right: 0;
  bottom: 0.25rem;
  left: 0;
  display: flex;
  justify-content: space-between;
  gap: 1rem;
  color: var(--service-text-muted);
  font-size: 0.6875rem;
  font-variant-numeric: tabular-nums;
  pointer-events: none;

  @container (max-width: 34rem) {
    justify-content: flex-end;

    span:first-child:not(:last-child) {
      display: none;
    }
  }
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

const Legend = styled.ul`
  display: flex;
  align-items: center;
  flex-wrap: wrap;
  gap: 0.375rem 1rem;
  margin: 0;
  padding: 0.625rem 1rem 0.875rem;
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
  border-radius: 999px;
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
  padding: 0.625rem;
  color: var(--service-text);
  background:
    linear-gradient(var(--service-surface-muted) 30%, transparent) center top,
    linear-gradient(transparent, var(--service-surface-muted) 70%) center bottom,
    radial-gradient(farthest-side at 50% 0, rgb(0 0 0 / 0.24), transparent)
      center top,
    radial-gradient(farthest-side at 50% 100%, rgb(0 0 0 / 0.3), transparent)
      center bottom,
    var(--service-surface-muted);
  background-attachment: local, local, scroll, scroll, scroll;
  background-repeat: no-repeat;
  background-size:
    100% 1rem,
    100% 1rem,
    100% 0.5rem,
    100% 0.5rem,
    100% 100%;
  border: 1px solid var(--service-border-strong);
  border-radius: 0.625rem;
  box-shadow: 0 18px 48px rgb(0 0 0 / 0.34);
  overscroll-behavior: contain;
  pointer-events: auto;
  scrollbar-color: color-mix(in srgb, var(--service-accent) 65%, transparent)
    transparent;
  scrollbar-width: thin;

  &::-webkit-scrollbar {
    width: 6px;
  }

  &::-webkit-scrollbar-thumb {
    background: color-mix(in srgb, var(--service-accent) 65%, transparent);
    border-radius: 999px;
  }

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
  border-top: 1px solid var(--service-border-strong);
`;

const CostHeading = styled.div`
  color: var(--service-text-muted);
  font-size: 0.6875rem;
`;

const BreakdownTotal = styled.div<{ $sticky: boolean }>`
  display: flex;
  justify-content: space-between;
  gap: 1rem;
  margin-top: 0.5rem;
  padding-top: 0.5rem;
  color: var(--service-text);
  border-top: 1px solid var(--service-border-strong);
  font-size: 0.8125rem;
  font-weight: 600;
  font-variant-numeric: tabular-nums;

  ${(props) =>
    props.$sticky &&
    css`
      position: sticky;
      bottom: -0.625rem;
      z-index: 1;
      margin-right: -0.625rem;
      margin-bottom: -0.625rem;
      margin-left: -0.625rem;
      padding: 0.625rem;
      background: var(--service-surface-muted);
      box-shadow: 0 -8px 16px rgb(0 0 0 / 0.18);
    `}
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
  stickyTotal?: boolean;
  total: number;
}

function BreakdownContent({
  date,
  mode,
  metric,
  rows,
  providerCosts,
  stickyTotal = false,
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
      <BreakdownTotal $sticky={stickyTotal}>
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
  const chartTitleId = useId();
  const chartDescriptionId = useId();
  const keyboardInstructionsId = useId();
  const [metric, setMetric] = useState<UsageMetric>(initialMetric);
  const [view, setView] = useState<UsageView>("average");
  const [providerFilter, setProviderFilter] =
    useState<UsageProviderFilter>(ALL_USAGE_PROVIDERS);
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
  const chartData = useMemo(
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
  const chartStack = useMemo(
    () =>
      createChartStack(
        chartData.series,
        chartData.dates.length,
        chartData.maxDailyTotal,
      ),
    [chartData],
  );
  const { layers } = chartStack;
  const chartMaximum = chartStack.maximum;

  useEffect(() => {
    const plot = plotRef.current;
    if (!plot) return;

    const measure = () => {
      const bounds = plot.getBoundingClientRect();
      const nextWidth = bounds.width;
      const availableHeight =
        window.innerHeight -
        Math.max(
          TOOLTIP_EDGE,
          bounds.top + TOOLTIP_EDGE + TOOLTIP_VIEWPORT_EDGE,
        );
      setPlotWidth((currentWidth) =>
        currentWidth === nextWidth ? currentWidth : nextWidth,
      );
      setTooltipMaxHeight((currentHeight) => {
        const nextHeight = clamp(availableHeight, 0, TOOLTIP_MAX_HEIGHT);
        return currentHeight === nextHeight ? currentHeight : nextHeight;
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

  const requestedActiveIndex = activeDate
    ? chartData.dates.indexOf(activeDate)
    : -1;
  const activeIndex =
    requestedActiveIndex >= 0
      ? requestedActiveIndex
      : chartData.dates.length - 1;
  const currentDate = chartData.dates[activeIndex] ?? null;
  const currentTotal = chartData.dailyTotals[activeIndex] ?? 0;
  const activeRows = useMemo(
    () => getActiveTooltipRows(chartData.series, activeIndex),
    [chartData.series, activeIndex],
  );
  const providerCostRows = useMemo(
    () =>
      getProviderCostRows(
        days,
        costProviderTotals,
        selectedProvider,
        activeIndex,
        view,
        chartData.averageWindowDays,
      ),
    [
      days,
      costProviderTotals,
      selectedProvider,
      activeIndex,
      view,
      chartData.averageWindowDays,
    ],
  );

  const providerLegend = useMemo(
    () =>
      providerTotals
        .filter(
          ({ provider }) =>
            selectedProvider === ALL_USAGE_PROVIDERS ||
            provider === selectedProvider,
        )
        .sort(
          (left, right) =>
            right[metric] - left[metric] ||
            left.provider.localeCompare(right.provider),
        ),
    [providerTotals, selectedProvider, metric],
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
    const viewBoxX =
      ((event.clientX - bounds.left) / bounds.width) * VIEWBOX_WIDTH;
    const progress = Math.max(
      0,
      Math.min(1, (viewBoxX - PLOT_LEFT) / PLOT_WIDTH),
    );
    setActiveIndex(
      Math.round(progress * (chartData.dates.length - 1)),
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

  const activeX =
    activeIndex >= 0
      ? xForIndex(activeIndex, chartData.dates.length)
      : PLOT_LEFT;
  const activeOffset = (activeX / VIEWBOX_WIDTH) * 100;
  const activeTooltipLeft = tooltipLeft(activeOffset, plotWidth);
  const modeLabel = viewLabel(view, chartData.averageWindowDays);
  const chartTitle = `${modeLabel} ${metricLabel(metric).toLowerCase()} usage by model and provider`;
  const chartDescription = `${chartTitle} from ${
    chartData.dates[0] ? formatDate(chartData.dates[0]) : "no start date"
  } to ${
    chartData.dates.at(-1)
      ? formatDate(chartData.dates.at(-1) as string)
      : "no end date"
  }. Raw range total: ${formatMetric(chartData.total, metric)}.`;
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
          <MetricControl aria-label="Usage metric">
            {(["tokens", "cost"] as const).map((option) => (
              <MetricButton
                key={option}
                type="button"
                $active={metric === option}
                aria-pressed={metric === option}
                onClick={() => setMetric(option)}
              >
                {metricLabel(option)}
              </MetricButton>
            ))}
          </MetricControl>

          <SelectControl>
            <SelectCaption>Display</SelectCaption>
            <CompactSelect
              aria-label="Usage display"
              value={view}
              onChange={(event) =>
                setView(event.currentTarget.value as UsageView)
              }
            >
              <option value="average">
                {chartData.averageWindowDays}d average
              </option>
              <option value="daily">Daily</option>
            </CompactSelect>
          </SelectControl>
        </ControlCluster>

        <SelectControl>
          <SelectCaption>Provider</SelectCaption>
          <CompactSelect
            name="profile-usage-provider"
            aria-label="Usage provider"
            value={selectedProvider}
            onChange={(event) =>
              setProviderFilter(
                event.currentTarget.value as UsageProviderFilter,
              )
            }
          >
            <option value={ALL_USAGE_PROVIDERS}>All</option>
            {providerTotals.map(({ provider }) => (
              <option key={provider} value={provider}>
                {providerName(provider)}
              </option>
            ))}
          </CompactSelect>
        </SelectControl>
      </Controls>

      <PlotRegion>
        {chartData.dates.length > 0 ? (
          <InteractivePlot
            ref={plotRef}
            tabIndex={0}
            role="group"
            aria-describedby={keyboardInstructionsId}
            aria-label={`Interactive ${metricLabel(metric).toLowerCase()} chart`}
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
          >
            <ChartSvg
              viewBox={`0 0 ${VIEWBOX_WIDTH} ${VIEWBOX_HEIGHT}`}
              preserveAspectRatio="none"
              role="img"
              aria-labelledby={`${chartTitleId} ${chartDescriptionId}`}
            >
              <title id={chartTitleId}>{chartTitle}</title>
              <desc id={chartDescriptionId}>{chartDescription}</desc>

              {Array.from({ length: GRID_STEPS + 1 }, (_, index) => {
                const value = (chartMaximum * index) / GRID_STEPS;
                const y = yForValue(value, chartMaximum);
                return (
                  <GridLine
                    key={index}
                    x1={PLOT_LEFT}
                    x2={PLOT_LEFT + PLOT_WIDTH}
                    y1={y}
                    y2={y}
                  />
                );
              })}

              {layers.map((layer) => (
                <g key={layer.series.id}>
                  <LayerArea d={layer.areaPath} $color={layer.series.color} />
                  <LayerLine d={layer.linePath} $color={layer.series.color} />
                </g>
              ))}

              {isInspecting && (
                <ActiveRule
                  x1={activeX}
                  x2={activeX}
                  y1={PLOT_TOP}
                  y2={PLOT_TOP + PLOT_HEIGHT}
                />
              )}
            </ChartSvg>

            {isInspecting &&
              layers.map((layer) => {
                if ((layer.series.values[activeIndex] ?? 0) <= 0) return null;
                const position = pointToChartPercent(
                  activeX,
                  yForValue(layer.upperValues[activeIndex] ?? 0, chartMaximum),
                  VIEWBOX_WIDTH,
                  VIEWBOX_HEIGHT,
                );
                return (
                  <ActivePoint
                    key={layer.series.id}
                    aria-hidden="true"
                    data-profile-usage-point
                    $color={layer.series.color}
                    $left={position.left}
                    $top={position.top}
                  />
                );
              })}

            <DateRange aria-label="Chart date range">
              <span>{formatDate(chartData.dates[0])}</span>
              {chartData.dates.length > 1 && (
                <span>{formatDate(chartData.dates.at(-1) as string)}</span>
              )}
            </DateRange>

            {isInspecting && currentDate && (
              <TooltipSurface
                role="tooltip"
                data-profile-usage-tooltip
                tabIndex={interactionMode === "committed" ? 0 : undefined}
                $left={activeTooltipLeft}
                $maxHeight={tooltipMaxHeight}
                onPointerMove={(event) => event.stopPropagation()}
                onPointerDown={(event) => event.stopPropagation()}
                onKeyDown={(event) => event.stopPropagation()}
              >
                <BreakdownContent
                  date={currentDate}
                  mode={modeLabel}
                  metric={metric}
                  rows={activeRows}
                  providerCosts={providerCostRows}
                  stickyTotal
                  total={currentTotal}
                />
              </TooltipSurface>
            )}
          </InteractivePlot>
        ) : (
          <EmptyState>No usage data yet.</EmptyState>
        )}
        <VisuallyHidden id={keyboardInstructionsId}>
          Use Left Arrow and Right Arrow to inspect adjacent days. Use Home and
          End to jump to the first or last day. Press Escape to close the
          inspection.
        </VisuallyHidden>
      </PlotRegion>

      {providerLegend.length > 0 && (
        <Legend role="list" aria-label="Usage providers">
          {providerLegend.map(({ provider }) => (
            <LegendItem key={provider}>
              <Swatch $color={providerColor(provider)} aria-hidden="true" />
              <span>{providerName(provider)}</span>
            </LegendItem>
          ))}
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
