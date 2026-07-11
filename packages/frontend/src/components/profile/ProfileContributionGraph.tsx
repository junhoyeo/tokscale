"use client";

import {
  Fragment,
  useId,
  useMemo,
  useRef,
  useState,
  type FocusEvent,
  type KeyboardEvent,
  type PointerEvent,
} from "react";
import styled, { css } from "styled-components";
import { SOURCE_COLORS, SOURCE_DISPLAY_NAMES } from "@/lib/constants";
import type {
  ClientContribution,
  ClientType,
  DailyContribution,
  TokenBreakdown,
} from "@/lib/types";
import {
  colorPalettes,
  DEFAULT_PALETTE,
  getPalette,
  getPaletteNames,
  type ColorPaletteName,
  type GraphColorPalette,
} from "@/lib/themes";
import { formatCurrency, formatTokenCount } from "@/lib/utils";

export interface ProfileContributionGraphProps {
  breakdownId?: string;
  className?: string;
  contributions: DailyContribution[];
  description?: string;
  onPaletteChange?: (palette: ColorPaletteName) => void;
  onSelectedDateChange?: (date: string | null) => void;
  onViewChange?: (view: ProfileContributionView) => void;
  paletteName?: ColorPaletteName;
  persistentSelection?: boolean;
  rangeEnd?: string | null;
  rangeStart?: string | null;
  selectedDate?: string | null;
  showBreakdown?: boolean;
  view?: ProfileContributionView;
}

export type ProfileContributionView = "2d" | "3d";

export interface ContributionSelectionState {
  date: string | null;
  rangeIdentity: string;
}

export function resolveContributionSelectedDate(
  requested: ContributionSelectionState | null,
  rangeIdentity: string,
  defaultDate: string | null,
): string | null {
  return requested?.rangeIdentity === rangeIdentity && requested.date
    ? requested.date
    : defaultDate;
}

export function reconcileContributionSelectionRange(
  selection: ContributionSelectionState,
  rangeIdentity: string,
): ContributionSelectionState {
  return selection.rangeIdentity === rangeIdentity
    ? selection
    : { date: null, rangeIdentity };
}

export interface ContributionCell {
  date: string;
  intensity: 0 | 1 | 2 | 3 | 4;
  inRange: boolean;
  tokens: number;
}

interface MonthMarker {
  compactVisible: boolean;
  label: string;
  weekIndex: number;
}

interface ContributionTooltipState {
  cell: ContributionCell;
  day: DailyContribution;
  left: number;
  top: number;
}

export interface ContributionModelDetail {
  cost: number;
  messages: number;
  modelId: string;
  providerId: string | null;
  tokens: TokenBreakdown;
  totalTokens: number;
}

export interface ContributionClientDetail {
  client: ClientType;
  cost: number;
  messages: number;
  models: ContributionModelDetail[];
  tokens: TokenBreakdown;
  totalTokens: number;
}

export interface ProfileContributionBreakdownProps {
  className?: string;
  day: DailyContribution;
  id: string;
  onClose?: () => void;
  paletteName?: ColorPaletteName;
}

export interface ContributionCalendar {
  activeDays: number;
  cells: ContributionCell[];
  endDate: string | null;
  freeTokenDays: number;
  highestDay: ContributionCell | null;
  monthMarkers: MonthMarker[];
  startDate: string | null;
  weekCount: number;
}

export interface ContributionIsometricCell {
  cell: ContributionCell;
  centerX: number;
  centerY: number;
  dayIndex: number;
  height: number;
  weekIndex: number;
}

export interface ContributionIsometricGeometry {
  cells: ContributionIsometricCell[];
  viewBox: { height: number; width: number };
}

type ContributionNavigationKey =
  | "ArrowDown"
  | "ArrowLeft"
  | "ArrowRight"
  | "ArrowUp"
  | "End"
  | "Home";

const DAY_MS = 24 * 60 * 60 * 1000;
const DATE_PATTERN = /^(\d{4})-(\d{2})-(\d{2})$/;

const dayFormatter = new Intl.DateTimeFormat("en-US", {
  day: "numeric",
  month: "short",
  timeZone: "UTC",
  year: "numeric",
});

const fullDayFormatter = new Intl.DateTimeFormat("en-US", {
  day: "numeric",
  month: "long",
  timeZone: "UTC",
  weekday: "long",
  year: "numeric",
});

const monthFormatter = new Intl.DateTimeFormat("en-US", {
  month: "short",
  timeZone: "UTC",
});

const tokenFormatter = new Intl.NumberFormat("en-US", {
  maximumFractionDigits: 0,
});

const EMPTY_TOKEN_BREAKDOWN: TokenBreakdown = {
  cacheRead: 0,
  cacheWrite: 0,
  input: 0,
  output: 0,
  reasoning: 0,
};

const TOKEN_CATEGORIES = [
  ["Input", "input"],
  ["Output", "output"],
  ["Cache read", "cacheRead"],
  ["Cache write", "cacheWrite"],
  ["Reasoning", "reasoning"],
] as const;

function parseUtcDate(date: string): number | null {
  const match = DATE_PATTERN.exec(date);
  if (!match) return null;

  const year = Number(match[1]);
  const month = Number(match[2]) - 1;
  const day = Number(match[3]);
  const timestamp = Date.UTC(year, month, day);
  const parsed = new Date(timestamp);

  if (
    parsed.getUTCFullYear() !== year ||
    parsed.getUTCMonth() !== month ||
    parsed.getUTCDate() !== day
  ) {
    return null;
  }

  return timestamp;
}

function toDateKey(timestamp: number): string {
  return new Date(timestamp).toISOString().slice(0, 10);
}

function safeTokens(value: number): number {
  return Number.isFinite(value) ? Math.max(0, value) : 0;
}

function safeCost(value: number): number {
  return Number.isFinite(value) ? Math.max(0, value) : 0;
}

function safeMessages(value: number): number {
  return Number.isFinite(value) ? Math.max(0, Math.trunc(value)) : 0;
}

function sanitizeTokenBreakdown(tokens: TokenBreakdown): TokenBreakdown {
  return {
    cacheRead: safeTokens(tokens.cacheRead),
    cacheWrite: safeTokens(tokens.cacheWrite),
    input: safeTokens(tokens.input),
    output: safeTokens(tokens.output),
    reasoning: safeTokens(tokens.reasoning),
  };
}

function addTokenBreakdowns(
  left: TokenBreakdown,
  right: TokenBreakdown,
): TokenBreakdown {
  return {
    cacheRead: left.cacheRead + right.cacheRead,
    cacheWrite: left.cacheWrite + right.cacheWrite,
    input: left.input + right.input,
    output: left.output + right.output,
    reasoning: left.reasoning + right.reasoning,
  };
}

function totalBreakdownTokens(tokens: TokenBreakdown): number {
  return (
    tokens.input +
    tokens.output +
    tokens.cacheRead +
    tokens.cacheWrite +
    tokens.reasoning
  );
}

export function mergeDailyContributions(
  contributions: readonly DailyContribution[],
): Map<string, DailyContribution> {
  const days = new Map<string, DailyContribution>();

  for (const contribution of contributions) {
    if (parseUtcDate(contribution.date) === null) continue;

    const existing = days.get(contribution.date);
    const tokens = sanitizeTokenBreakdown(contribution.tokenBreakdown);
    if (!existing) {
      days.set(contribution.date, {
        ...contribution,
        clients: [...contribution.clients],
        intensity: contribution.intensity,
        tokenBreakdown: tokens,
        totals: {
          cost: safeCost(contribution.totals.cost),
          messages: safeMessages(contribution.totals.messages),
          tokens: safeTokens(contribution.totals.tokens),
        },
      });
      continue;
    }

    days.set(contribution.date, {
      ...existing,
      clients: [...existing.clients, ...contribution.clients],
      intensity: Math.max(existing.intensity, contribution.intensity) as
        | 0
        | 1
        | 2
        | 3
        | 4,
      timestampMs: existing.timestampMs ?? contribution.timestampMs,
      tokenBreakdown: addTokenBreakdowns(existing.tokenBreakdown, tokens),
      totals: {
        cost: existing.totals.cost + safeCost(contribution.totals.cost),
        messages:
          existing.totals.messages + safeMessages(contribution.totals.messages),
        tokens: existing.totals.tokens + safeTokens(contribution.totals.tokens),
      },
    });
  }

  return days;
}

function createEmptyContribution(cell: ContributionCell): DailyContribution {
  return {
    clients: [],
    date: cell.date,
    intensity: cell.intensity,
    tokenBreakdown: { ...EMPTY_TOKEN_BREAKDOWN },
    totals: { cost: 0, messages: 0, tokens: cell.tokens },
  };
}

export function getContributionDayForDate(
  contributions: readonly DailyContribution[],
  date: string | null,
): DailyContribution | null {
  if (!date || parseUtcDate(date) === null) return null;

  return (
    mergeDailyContributions(contributions).get(date) ??
    createEmptyContribution({ date, inRange: true, intensity: 0, tokens: 0 })
  );
}

function addModelDetail(
  models: Map<string, ContributionModelDetail>,
  detail: ContributionModelDetail,
) {
  const key = `${detail.providerId ?? ""}\u0000${detail.modelId}`;
  const existing = models.get(key);
  if (!existing) {
    models.set(key, detail);
    return;
  }

  const tokens = addTokenBreakdowns(existing.tokens, detail.tokens);
  models.set(key, {
    ...existing,
    cost: existing.cost + detail.cost,
    messages: existing.messages + detail.messages,
    tokens,
    totalTokens: existing.totalTokens + detail.totalTokens,
  });
}

function modelsForClient(
  contribution: ClientContribution,
): ContributionModelDetail[] {
  const nestedModels = Object.entries(contribution.models ?? {});
  if (nestedModels.length > 0) {
    return nestedModels.map(([modelId, model]) => {
      const tokens = sanitizeTokenBreakdown({
        cacheRead: model.cacheRead,
        cacheWrite: model.cacheWrite,
        input: model.input,
        output: model.output,
        reasoning: model.reasoning,
      });
      return {
        cost: safeCost(model.cost),
        messages: safeMessages(model.messages),
        modelId,
        providerId: contribution.providerId?.trim() || null,
        tokens,
        totalTokens: Math.max(
          safeTokens(model.tokens),
          totalBreakdownTokens(tokens),
        ),
      };
    });
  }

  const modelId = contribution.modelId.trim();
  if (!modelId) return [];

  const tokens = sanitizeTokenBreakdown(contribution.tokens);
  return [
    {
      cost: safeCost(contribution.cost),
      messages: safeMessages(contribution.messages),
      modelId,
      providerId: contribution.providerId?.trim() || null,
      tokens,
      totalTokens: totalBreakdownTokens(tokens),
    },
  ];
}

export function createContributionClientDetails(
  day: DailyContribution,
): ContributionClientDetail[] {
  const clients = new Map<
    ClientType,
    Omit<ContributionClientDetail, "models"> & {
      models: Map<string, ContributionModelDetail>;
    }
  >();

  for (const contribution of day.clients) {
    const contributionTokens = sanitizeTokenBreakdown(contribution.tokens);
    const existing = clients.get(contribution.client) ?? {
      client: contribution.client,
      cost: 0,
      messages: 0,
      models: new Map<string, ContributionModelDetail>(),
      tokens: { ...EMPTY_TOKEN_BREAKDOWN },
      totalTokens: 0,
    };
    existing.cost += safeCost(contribution.cost);
    existing.messages += safeMessages(contribution.messages);
    existing.tokens = addTokenBreakdowns(existing.tokens, contributionTokens);
    existing.totalTokens = totalBreakdownTokens(existing.tokens);

    for (const model of modelsForClient(contribution)) {
      addModelDetail(existing.models, model);
    }
    clients.set(contribution.client, existing);
  }

  return [...clients.values()]
    .map((client) => {
      const models = [...client.models.values()].sort(
        (left, right) =>
          right.cost - left.cost ||
          right.totalTokens - left.totalTokens ||
          left.modelId.localeCompare(right.modelId),
      );
      const modelTokens = models.reduce(
        (total, model) => total + model.totalTokens,
        0,
      );
      const modelMessages = models.reduce(
        (total, model) => total + model.messages,
        0,
      );
      return {
        ...client,
        messages: client.messages || modelMessages,
        models,
        totalTokens: client.totalTokens || modelTokens,
      };
    })
    .sort(
      (left, right) =>
        right.cost - left.cost ||
        right.totalTokens - left.totalTokens ||
        String(left.client).localeCompare(String(right.client)),
    );
}

export function getContributionDayMessageCount(
  day: DailyContribution,
  clients: readonly ContributionClientDetail[] = createContributionClientDetails(
    day,
  ),
): number {
  const recordedTotal = safeMessages(day.totals.messages);
  return (
    recordedTotal ||
    clients.reduce((total, client) => total + client.messages, 0)
  );
}

function tokenIntensity(tokens: number, maxTokens: number): 0 | 1 | 2 | 3 | 4 {
  if (tokens <= 0 || maxTokens <= 0) return 0;
  const ratio = tokens / maxTokens;
  if (ratio >= 0.75) return 4;
  if (ratio >= 0.5) return 3;
  if (ratio >= 0.25) return 2;
  return 1;
}

export function createContributionCalendar(
  contributions: readonly DailyContribution[],
  rangeStart?: string | null,
  rangeEnd?: string | null,
): ContributionCalendar {
  const contributionsByDate = new Map<
    string,
    { cost: number; timestamp: number; tokens: number }
  >();

  for (const contribution of contributions) {
    const timestamp = parseUtcDate(contribution.date);
    if (timestamp === null) continue;

    const tokens = safeTokens(contribution.totals.tokens);
    const existing = contributionsByDate.get(contribution.date);
    contributionsByDate.set(contribution.date, {
      cost:
        (existing?.cost ?? 0) +
        (Number.isFinite(contribution.totals.cost)
          ? Math.max(0, contribution.totals.cost)
          : 0),
      timestamp,
      tokens: (existing?.tokens ?? 0) + tokens,
    });
  }

  const sorted = [...contributionsByDate.values()].sort(
    (left, right) => left.timestamp - right.timestamp,
  );
  const requestedStart = rangeStart ? parseUtcDate(rangeStart) : null;
  const requestedEnd = rangeEnd ? parseUtcDate(rangeEnd) : null;
  const hasRequestedRange =
    requestedStart !== null &&
    requestedEnd !== null &&
    requestedEnd >= requestedStart;

  if (sorted.length === 0 && !hasRequestedRange) {
    return {
      activeDays: 0,
      cells: [],
      endDate: null,
      freeTokenDays: 0,
      highestDay: null,
      monthMarkers: [],
      startDate: null,
      weekCount: 0,
    };
  }

  const firstTimestamp = hasRequestedRange
    ? requestedStart
    : sorted[0].timestamp;
  const lastTimestamp = hasRequestedRange
    ? requestedEnd
    : sorted[sorted.length - 1].timestamp;
  const scopedContributions = sorted.filter(
    ({ timestamp }) =>
      timestamp >= firstTimestamp && timestamp <= lastTimestamp,
  );
  const maxTokens = Math.max(
    0,
    ...scopedContributions.map(({ tokens }) => tokens),
  );
  const calendarStart =
    firstTimestamp - new Date(firstTimestamp).getUTCDay() * DAY_MS;
  const calendarEnd =
    lastTimestamp + (6 - new Date(lastTimestamp).getUTCDay()) * DAY_MS;
  const dayCount = Math.round((calendarEnd - calendarStart) / DAY_MS) + 1;
  const weekCount = dayCount / 7;
  const cells: ContributionCell[] = [];

  for (let offset = 0; offset < dayCount; offset += 1) {
    const timestamp = calendarStart + offset * DAY_MS;
    const date = toDateKey(timestamp);
    const contribution = contributionsByDate.get(date);
    const inRange = timestamp >= firstTimestamp && timestamp <= lastTimestamp;

    cells.push({
      date,
      inRange,
      intensity: inRange
        ? tokenIntensity(contribution?.tokens ?? 0, maxTokens)
        : 0,
      tokens: inRange ? (contribution?.tokens ?? 0) : 0,
    });
  }

  const monthMarkers: MonthMarker[] = [];
  const markerWeeks = new Set<number>();
  let cursor = firstTimestamp;

  while (cursor <= lastTimestamp) {
    const date = new Date(cursor);
    const weekIndex = Math.floor((cursor - calendarStart) / (DAY_MS * 7));

    if (!markerWeeks.has(weekIndex)) {
      const month = date.getUTCMonth();
      const marker = {
        compactVisible: monthMarkers.length === 0 || month % 3 === 0,
        label: monthFormatter.format(date),
        weekIndex,
      };
      const previous = monthMarkers.at(-1);
      if (previous && weekIndex - previous.weekIndex < 3) {
        // A short partial first month can land beside the next label. Prefer
        // the first full month rather than allowing labels to collide.
        if (previous.weekIndex === 0)
          monthMarkers[monthMarkers.length - 1] = marker;
      } else {
        monthMarkers.push(marker);
      }
      markerWeeks.add(weekIndex);
    }

    cursor = Date.UTC(date.getUTCFullYear(), date.getUTCMonth() + 1, 1);
  }

  return {
    activeDays: scopedContributions.filter(({ tokens }) => tokens > 0).length,
    cells,
    endDate: toDateKey(lastTimestamp),
    freeTokenDays: scopedContributions.filter(
      ({ cost, tokens }) => tokens > 0 && cost === 0,
    ).length,
    highestDay:
      [...cells]
        .filter(({ inRange, tokens }) => inRange && tokens > 0)
        .sort(
          (left, right) =>
            right.tokens - left.tokens || left.date.localeCompare(right.date),
        )[0] ?? null,
    monthMarkers,
    startDate: toDateKey(firstTimestamp),
    weekCount,
  };
}

export function getDefaultContributionDate(
  contributions: readonly DailyContribution[],
  rangeStart?: string | null,
  rangeEnd?: string | null,
): string | null {
  return createContributionCalendar(contributions, rangeStart, rangeEnd)
    .endDate;
}

const ISOMETRIC_CELL_WIDTH = 7.5;
const ISOMETRIC_CELL_DEPTH = 3.75;
const ISOMETRIC_MARGIN = 12;
const ISOMETRIC_MIN_HEIGHT = 1.5;
const ISOMETRIC_ACTIVE_MIN_HEIGHT = 4;
const ISOMETRIC_MAX_HEIGHT = 26;

export function createContributionIsometricGeometry(
  calendar: ContributionCalendar,
): ContributionIsometricGeometry {
  const maxTokens = Math.max(
    0,
    ...calendar.cells
      .filter(({ inRange }) => inRange)
      .map(({ tokens }) => tokens),
  );
  const finalWeek = Math.max(0, calendar.weekCount - 1);
  const originX = ISOMETRIC_MARGIN + 6 * ISOMETRIC_CELL_WIDTH;
  const originY = ISOMETRIC_MARGIN + ISOMETRIC_MAX_HEIGHT;
  const cells = calendar.cells.flatMap((cell, index) => {
    if (!cell.inRange) return [];

    const weekIndex = Math.floor(index / 7);
    const dayIndex = index % 7;
    const ratio = maxTokens > 0 ? cell.tokens / maxTokens : 0;
    const height =
      cell.tokens > 0
        ? ISOMETRIC_ACTIVE_MIN_HEIGHT +
          ratio * (ISOMETRIC_MAX_HEIGHT - ISOMETRIC_ACTIVE_MIN_HEIGHT)
        : ISOMETRIC_MIN_HEIGHT;

    return [
      {
        cell,
        centerX: originX + (weekIndex - dayIndex) * ISOMETRIC_CELL_WIDTH,
        centerY: originY + (weekIndex + dayIndex) * ISOMETRIC_CELL_DEPTH,
        dayIndex,
        height,
        weekIndex,
      },
    ];
  });

  return {
    cells,
    viewBox: {
      height:
        originY +
        (finalWeek + 6) * ISOMETRIC_CELL_DEPTH +
        ISOMETRIC_CELL_DEPTH * 2 +
        ISOMETRIC_MARGIN,
      width:
        originX +
        finalWeek * ISOMETRIC_CELL_WIDTH +
        ISOMETRIC_CELL_WIDTH +
        ISOMETRIC_MARGIN,
    },
  };
}

function contributionCubeFaces({
  centerX,
  centerY,
  height,
}: ContributionIsometricCell): {
  left: string;
  right: string;
  top: string;
} {
  const topY = centerY - height;
  const leftX = centerX - ISOMETRIC_CELL_WIDTH;
  const rightX = centerX + ISOMETRIC_CELL_WIDTH;
  const middleY = topY + ISOMETRIC_CELL_DEPTH;
  const bottomTopY = topY + ISOMETRIC_CELL_DEPTH * 2;
  const middleBottomY = centerY + ISOMETRIC_CELL_DEPTH;
  const bottomY = centerY + ISOMETRIC_CELL_DEPTH * 2;

  return {
    left: `${leftX},${middleY} ${centerX},${bottomTopY} ${centerX},${bottomY} ${leftX},${middleBottomY}`,
    right: `${rightX},${middleY} ${centerX},${bottomTopY} ${centerX},${bottomY} ${rightX},${middleBottomY}`,
    top: `${centerX},${topY} ${rightX},${middleY} ${centerX},${bottomTopY} ${leftX},${middleY}`,
  };
}

function shadeContributionColor(color: string, percentage: number): string {
  return `color-mix(in srgb, ${color} ${percentage}%, #000)`;
}

export function getContributionFocusDate(
  cells: readonly ContributionCell[],
  currentDate: string | null,
  key: ContributionNavigationKey,
): string | null {
  const dates = cells.filter(({ inRange }) => inRange).map(({ date }) => date);
  if (dates.length === 0) return null;

  const currentIndex = currentDate ? dates.indexOf(currentDate) : -1;
  const safeIndex = currentIndex >= 0 ? currentIndex : dates.length - 1;
  let nextIndex = safeIndex;

  switch (key) {
    case "ArrowLeft":
      nextIndex -= 1;
      break;
    case "ArrowRight":
      nextIndex += 1;
      break;
    case "ArrowUp":
      nextIndex -= 7;
      break;
    case "ArrowDown":
      nextIndex += 7;
      break;
    case "Home":
      nextIndex = 0;
      break;
    case "End":
      nextIndex = dates.length - 1;
      break;
  }

  return dates[Math.max(0, Math.min(dates.length - 1, nextIndex))] ?? null;
}

function formatRange(startDate: string | null, endDate: string | null): string {
  if (!startDate || !endDate) return "No activity yet";

  const start = parseUtcDate(startDate);
  const end = parseUtcDate(endDate);
  if (start === null || end === null) return "No activity yet";
  if (start === end) return dayFormatter.format(start);
  return `${dayFormatter.format(start)} – ${dayFormatter.format(end)}`;
}

function cellTitle(cell: ContributionCell): string {
  const timestamp = parseUtcDate(cell.date);
  const date = timestamp === null ? cell.date : dayFormatter.format(timestamp);
  const tokenLabel = cell.tokens === 1 ? "token" : "tokens";
  return `${date}: ${tokenFormatter.format(cell.tokens)} ${tokenLabel}`;
}

function mixHexWithWhite(color: string, colorWeight: number): string {
  const match = /^#([0-9a-f]{6})$/i.exec(color);
  if (!match) return color;

  const channels = [0, 2, 4].map((offset) =>
    Number.parseInt(match[1].slice(offset, offset + 2), 16),
  );
  return `#${channels
    .map((channel) =>
      Math.round(channel * colorWeight + 255 * (1 - colorWeight))
        .toString(16)
        .padStart(2, "0"),
    )
    .join("")}`;
}

export function getContributionColor(
  palette: GraphColorPalette,
  level: ContributionCell["intensity"],
): string {
  if (level === 0) return "var(--service-surface-muted)";

  // These palettes were authored for light canvases, where higher intensity
  // is darker. Reverse the ramp on the service's dark canvas so activity
  // gains contrast as it increases.
  const baseColor =
    [palette.grade4, palette.grade3, palette.grade2, palette.grade1][
      level - 1
    ] ?? palette.grade1;
  const darkCanvasWeights = [0.55, 0.7, 0.85, 1] as const;
  return mixHexWithWhite(baseColor, darkCanvasWeights[level - 1] ?? 1);
}

const Figure = styled.figure`
  width: 100%;
  min-width: 0;
  max-width: 100%;
  display: flex;
  flex-direction: column;
  margin: 0;
  overflow: hidden;
  color: var(--service-text);
  background: var(--service-surface);
  border: 1px solid var(--service-border);
  border-radius: 0.75rem;
  container-type: inline-size;
`;

const Header = styled.figcaption`
  display: flex;
  align-items: flex-start;
  justify-content: space-between;
  gap: 1rem;
  padding: 0.875rem 1rem;
  border-bottom: 1px solid var(--service-border);

  @container (max-width: 28rem) {
    flex-direction: column;
    gap: 0.625rem;
  }
`;

const HeadingGroup = styled.div`
  min-width: 0;
`;

const Heading = styled.h2`
  margin: 0;
  color: var(--service-text);
  font-size: 0.9375rem;
  font-weight: 600;
  letter-spacing: -0.01em;
`;

const Description = styled.p`
  max-width: 46ch;
  margin: 0.25rem 0 0;
  color: var(--service-text-muted);
  font-size: 0.8125rem;
  line-height: 1.45;
`;

const HeaderAside = styled.div`
  display: flex;
  flex: 0 0 auto;
  align-items: flex-start;
  gap: 0.75rem;

  @container (max-width: 28rem) {
    width: 100%;
    align-items: flex-start;
    justify-content: space-between;
  }
`;

const ViewToggle = styled.div`
  display: inline-flex;
  padding: 2px;
  border: 1px solid var(--service-border);
  border-radius: 0.5rem;
  background: var(--service-surface-muted);
`;

const ViewButton = styled.button<{ $active: boolean }>`
  min-width: 2rem;
  height: 1.5rem;
  padding: 0 0.5rem;
  border: 0;
  border-radius: 0.35rem;
  background: ${(props) =>
    props.$active ? "var(--service-surface)" : "transparent"};
  color: ${(props) =>
    props.$active ? "var(--service-text)" : "var(--service-text-muted)"};
  font-size: 0.625rem;
  font-weight: 600;
  cursor: pointer;

  &:focus-visible {
    outline: 2px solid var(--service-focus);
    outline-offset: 1px;
  }
`;

const Summary = styled.div`
  flex: 0 0 auto;
  text-align: right;
  font-variant-numeric: tabular-nums;

  @container (max-width: 28rem) {
    display: block;
    width: auto;
    margin-left: auto;
    text-align: right;
  }
`;

const ActiveDays = styled.div`
  color: var(--service-text);
  font-size: 0.8125rem;
  font-weight: 600;
`;

const Range = styled.div`
  margin-top: 0.125rem;
  color: var(--service-text-muted);
  font-size: 0.6875rem;
  white-space: nowrap;
`;

const CalendarBody = styled.div`
  position: relative;
  display: flex;
  flex-direction: column;
  justify-content: center;
  min-width: 0;
  padding: 0.875rem 1rem 0.75rem;

  @container (max-width: 24rem) {
    padding-right: 0.75rem;
    padding-left: 0.75rem;
  }
`;

const IsometricBody = styled.div`
  position: relative;
  display: grid;
  min-width: 0;
  min-height: 12rem;
  padding: 0.75rem 1rem;
  place-items: center;
  overflow: hidden;

  @container (max-width: 24rem) {
    min-height: 10rem;
    padding-right: 0.75rem;
    padding-left: 0.75rem;
  }
`;

const IsometricSvg = styled.svg`
  display: block;
  width: 100%;
  max-height: 17rem;
  overflow: visible;
`;

const IsometricCell = styled.g<{ $active: boolean; $selected: boolean }>`
  cursor: pointer;
  outline: none;

  polygon {
    transition:
      stroke 120ms ease,
      filter 120ms ease;
  }

  &:hover polygon,
  &:focus-visible polygon {
    filter: brightness(1.12);
  }

  @media (prefers-reduced-motion: reduce) {
    polygon {
      transition: none;
    }
  }
`;

const IsometricTop = styled.polygon<{
  $active: boolean;
  $selected: boolean;
}>`
  stroke: ${(props) =>
    props.$selected
      ? "var(--service-focus)"
      : props.$active
        ? "var(--service-text)"
        : "rgba(255, 255, 255, 0.08)"};
  stroke-width: ${(props) => (props.$active || props.$selected ? 1.4 : 0.55)};
  vector-effect: non-scaling-stroke;
`;

const MonthRow = styled.div<{ $weeks: number }>`
  display: grid;
  grid-template-columns: repeat(${(props) => props.$weeks}, minmax(0, 1fr));
  box-sizing: border-box;
  width: 100%;
  max-width: ${(props) =>
    1.75 + props.$weeks * (props.$weeks <= 5 ? 1.25 : 0.875)}rem;
  min-width: 0;
  height: 1.125rem;
  padding-left: 1.75rem;
  color: var(--service-text-muted);

  @container (max-width: 22rem) {
    max-width: ${(props) =>
      props.$weeks * (props.$weeks <= 5 ? 1.25 : 0.875)}rem;
    padding-left: 0;
  }
`;

const Month = styled.span<{
  $compactVisible: boolean;
  $week: number;
}>`
  position: relative;
  grid-column: ${(props) => props.$week + 1};
  min-width: 0;
  font-size: 0.625rem;
  font-variant-numeric: tabular-nums;
  line-height: 1;
  white-space: nowrap;

  &::after {
    position: absolute;
    top: 0.75rem;
    left: 0;
    width: 1px;
    height: 0.25rem;
    background: var(--service-border-strong);
    content: "";
  }

  @container (max-width: 32rem) {
    ${(props) =>
      !props.$compactVisible &&
      css`
        display: none;
      `}
  }
`;

const CalendarRow = styled.div`
  display: grid;
  grid-template-columns: 1.25rem minmax(0, 1fr);
  align-items: stretch;
  gap: 0.5rem;
  min-width: 0;

  @container (max-width: 22rem) {
    grid-template-columns: minmax(0, 1fr);
  }
`;

const DayLabels = styled.div`
  display: grid;
  grid-template-rows: repeat(7, minmax(0, 1fr));
  gap: clamp(1px, 0.25cqw, 2px);
  color: var(--service-text-muted);
  font-size: 0.625rem;
  line-height: 1;

  @container (max-width: 22rem) {
    display: none;
  }
`;

const DayLabel = styled.span<{ $row: number }>`
  grid-row: ${(props) => props.$row};
  align-self: center;
`;

const Grid = styled.div<{ $weeks: number }>`
  display: grid;
  grid-auto-flow: column;
  grid-template-columns: repeat(${(props) => props.$weeks}, minmax(0, 1fr));
  grid-template-rows: repeat(7, auto);
  gap: clamp(1px, 0.25cqw, 2px);
  width: 100%;
  max-width: ${(props) => props.$weeks * (props.$weeks <= 5 ? 1.25 : 0.875)}rem;
  min-width: 0;
`;

const Cell = styled.button<{
  $active: boolean;
  $color: string;
  $inRange: boolean;
  $selected: boolean;
}>`
  display: block;
  min-width: 0;
  padding: 0;
  aspect-ratio: 1;
  visibility: ${(props) => (props.$inRange ? "visible" : "hidden")};
  background: ${(props) => props.$color};
  border: 0;
  border-radius: clamp(1px, 0.25cqw, 2px);
  box-shadow:
    inset 0 0 0 1px rgba(255, 255, 255, 0.035),
    ${(props) =>
      props.$selected
        ? "0 0 0 2px var(--service-focus), 0 0 0 4px var(--service-canvas)"
        : props.$active
          ? "0 0 0 2px var(--service-text), 0 0 0 4px var(--service-canvas)"
          : "0 0 0 0 transparent"};
  cursor: pointer;

  &:focus-visible {
    position: relative;
    z-index: 2;
    outline: 2px solid var(--service-focus);
    outline-offset: 1px;
  }
`;

const Footer = styled.div`
  display: flex;
  align-items: center;
  justify-content: space-between;
  gap: 0.75rem;
  padding: 0 1rem 0.875rem;

  @container (max-width: 24rem) {
    padding-right: 0.75rem;
    padding-left: 0.75rem;
  }
`;

const PaletteControl = styled.label`
  position: relative;
  display: inline-flex;
  min-width: 0;
  align-items: center;
  gap: 0.375rem;
  color: var(--service-text-muted);
  font-size: 0.6875rem;

  &::after {
    position: absolute;
    right: 0.625rem;
    width: 0.3125rem;
    height: 0.3125rem;
    border-right: 1px solid var(--service-text-muted);
    border-bottom: 1px solid var(--service-text-muted);
    content: "";
    pointer-events: none;
    transform: translateY(-0.125rem) rotate(45deg);
  }
`;

const PaletteSelect = styled.select`
  max-width: 7.5rem;
  height: 1.75rem;
  padding: 0 1.5rem 0 0.5rem;
  appearance: none;
  color: var(--service-text);
  background: var(--service-surface-muted);
  border: 1px solid var(--service-border);
  border-radius: 0.4375rem;
  font: inherit;
  cursor: pointer;

  &:focus-visible {
    outline: 2px solid var(--service-focus);
    outline-offset: 1px;
  }
`;

const PalettePreview = styled.span`
  display: inline-grid;
  grid-template-columns: repeat(4, 0.375rem);
  gap: 1px;
  overflow: hidden;
  border-radius: 2px;
`;

const PalettePreviewSwatch = styled.span<{ $color: string }>`
  width: 0.375rem;
  height: 0.75rem;
  background: ${(props) => props.$color};
`;

const Legend = styled.div`
  display: inline-flex;
  align-items: center;
  gap: 0.3125rem;
  color: var(--service-text-muted);
  font-size: 0.6875rem;
`;

const LegendSwatches = styled.span`
  display: inline-grid;
  grid-template-columns: repeat(5, 0.625rem);
  gap: 0.1875rem;
`;

const LegendSwatch = styled.span<{ $color: string }>`
  width: 0.625rem;
  height: 0.625rem;
  background: ${(props) => props.$color};
  border-radius: 2px;
  box-shadow: inset 0 0 0 1px rgba(255, 255, 255, 0.05);
`;

const CellTooltip = styled.div<{
  $left: number;
  $top: number;
}>`
  position: fixed;
  z-index: 80;
  top: ${(props) => props.$top}px;
  left: ${(props) => props.$left}px;
  display: grid;
  width: min(17.5rem, calc(100vw - 1.5rem));
  max-height: calc(100dvh - 1.5rem);
  gap: 0.5rem;
  overflow: hidden;
  padding: 0.75rem;
  color: var(--service-text);
  background: var(--service-surface-muted);
  border: 1px solid var(--service-border-strong);
  border-radius: 0.625rem;
  box-shadow: 0 12px 32px rgb(0 0 0 / 0.34);
  font-variant-numeric: tabular-nums;
  pointer-events: none;
`;

const CellTooltipDate = styled.span`
  color: var(--service-text-muted);
  font-size: 0.6875rem;
`;

const TooltipTotal = styled.div`
  display: flex;
  align-items: baseline;
  justify-content: space-between;
  gap: 0.75rem;
`;

const TooltipTotalLabel = styled.span`
  color: var(--service-text-muted);
  font-size: 0.6875rem;
`;

const CellTooltipValue = styled.strong`
  color: var(--service-text);
  font-size: 1rem;
  font-weight: 600;
  letter-spacing: -0.02em;
`;

const TooltipDivider = styled.span`
  display: block;
  height: 1px;
  background: var(--service-border);
`;

const TooltipMetricGrid = styled.div`
  display: grid;
  grid-template-columns: minmax(0, 1fr) auto;
  gap: 0.25rem 0.75rem;
  font-size: 0.6875rem;
`;

const TooltipMetricLabel = styled.span`
  min-width: 0;
  overflow: hidden;
  color: var(--service-text-muted);
  text-overflow: ellipsis;
  white-space: nowrap;
`;

const TooltipMetricValue = styled.span`
  color: var(--service-text);
  font-family: var(--font-mono);
  font-weight: 600;
  text-align: right;
`;

const TooltipSectionLabel = styled.span`
  color: var(--service-text-muted);
  font-size: 0.625rem;
  font-weight: 600;
  letter-spacing: 0.06em;
  text-transform: uppercase;
`;

const DetailPanel = styled.section<{ $standalone: boolean }>`
  overflow: hidden;
  border: 1px solid var(--service-border);
  border-width: ${(props) => (props.$standalone ? "1px" : "1px 0 0")};
  border-radius: ${(props) => (props.$standalone ? "0.75rem" : "0")};
  background: color-mix(
    in srgb,
    var(--service-surface-muted) 42%,
    var(--service-surface)
  );
  container-type: inline-size;
`;

const DetailHeader = styled.header`
  display: flex;
  align-items: flex-start;
  justify-content: space-between;
  gap: 0.75rem;
  padding: 0.875rem 1rem;
  border-bottom: 1px solid var(--service-border);
`;

const DetailEyebrow = styled.div`
  margin-bottom: 0.125rem;
  color: var(--service-text-muted);
  font-size: 0.625rem;
  font-weight: 600;
  letter-spacing: 0.07em;
  text-transform: uppercase;
`;

const DetailTitle = styled.h3`
  margin: 0;
  color: var(--service-text);
  font-size: 0.875rem;
  font-weight: 600;
  letter-spacing: -0.01em;
`;

const DetailClose = styled.button`
  display: inline-grid;
  flex: 0 0 auto;
  width: 1.75rem;
  height: 1.75rem;
  padding: 0;
  place-items: center;
  color: var(--service-text-muted);
  background: transparent;
  border: 1px solid transparent;
  border-radius: 0.4375rem;
  cursor: pointer;

  &:hover {
    color: var(--service-text);
    background: var(--service-surface-muted);
    border-color: var(--service-border);
  }

  &:focus-visible {
    outline: 2px solid var(--service-focus);
    outline-offset: 1px;
  }
`;

const DetailBody = styled.div`
  display: grid;
  gap: 1rem;
  padding: 1rem;
`;

const DetailSummary = styled.div`
  display: grid;
  grid-template-columns: repeat(3, minmax(0, 1fr));
  overflow: hidden;
  border: 1px solid var(--service-border);
  border-radius: 0.5rem;

  > div {
    padding: 0.625rem;
    background: transparent;
    border: 0;
    border-radius: 0;
  }

  > div + div {
    border-left: 1px solid var(--service-border);
  }
`;

const DetailMetric = styled.div`
  min-width: 0;
  padding: 0.625rem 0.75rem;
`;

const DetailMetricLabel = styled.div`
  margin-bottom: 0.125rem;
  color: var(--service-text-muted);
  font-size: 0.625rem;
`;

const DetailMetricValue = styled.div`
  overflow: hidden;
  color: var(--service-text);
  font-family: var(--font-mono);
  font-size: 0.8125rem;
  font-weight: 600;
  text-overflow: ellipsis;
  white-space: nowrap;
`;

const TokenDetailGrid = styled.div`
  display: grid;
  grid-template-columns: repeat(5, minmax(0, 1fr));
  overflow: hidden;
  border: 1px solid var(--service-border);
  border-radius: 0.5rem;

  > div + div {
    border-left: 1px solid var(--service-border);
  }

  @container (max-width: 36rem) {
    grid-template-columns: repeat(3, minmax(0, 1fr));

    > div:nth-child(4) {
      border-left: 0;
    }

    > div:nth-child(n + 4) {
      border-top: 1px solid var(--service-border);
    }
  }

  @container (max-width: 24rem) {
    grid-template-columns: repeat(2, minmax(0, 1fr));

    > div:nth-child(3) {
      border-left: 0;
    }

    > div:nth-child(4) {
      border-left: 1px solid var(--service-border);
    }

    > div:nth-child(odd) {
      border-left: 0;
    }

    > div:nth-child(n + 3) {
      border-top: 1px solid var(--service-border);
    }
  }
`;

const DetailSection = styled.section`
  display: grid;
  gap: 0.5rem;
`;

const DetailSectionTitle = styled.h4`
  margin: 0;
  color: var(--service-text-muted);
  font-size: 0.6875rem;
  font-weight: 600;
`;

const ClientList = styled.div<{ $standalone: boolean }>`
  display: grid;
  max-height: ${(props) => (props.$standalone ? "none" : "25rem")};
  overflow: ${(props) => (props.$standalone ? "visible" : "auto")};
  border: 1px solid var(--service-border);
  border-radius: 0.625rem;

  &:focus-visible {
    outline: 2px solid var(--service-focus);
    outline-offset: 2px;
  }

  @container (max-width: 28rem) {
    max-height: none;
    overflow: visible;
  }
`;

const ClientSection = styled.section`
  min-width: 0;
  padding: 0.75rem;

  & + & {
    border-top: 1px solid var(--service-border);
  }
`;

const ClientHeader = styled.div`
  display: flex;
  align-items: center;
  justify-content: space-between;
  gap: 0.75rem;
`;

const ClientIdentity = styled.div`
  display: flex;
  min-width: 0;
  align-items: center;
  gap: 0.5rem;
`;

const ClientDot = styled.span<{ $color: string }>`
  flex: 0 0 auto;
  width: 0.5rem;
  height: 0.5rem;
  background: ${(props) => props.$color};
  border: 1px solid rgba(255, 255, 255, 0.28);
  border-radius: 999px;
`;

const ClientName = styled.strong`
  overflow: hidden;
  color: var(--service-text);
  font-size: 0.75rem;
  font-weight: 600;
  text-overflow: ellipsis;
  white-space: nowrap;
`;

const ClientTotal = styled.span`
  flex: 0 0 auto;
  color: var(--service-text-muted);
  font-family: var(--font-mono);
  font-size: 0.6875rem;
  font-variant-numeric: tabular-nums;
`;

const ModelList = styled.div`
  display: grid;
  gap: 0.375rem;
  margin-top: 0.625rem;
  padding-left: 1rem;
`;

const ModelRow = styled.div`
  display: grid;
  min-width: 0;
  grid-template-columns: minmax(0, 1fr) auto;
  gap: 0.125rem 0.75rem;
  padding-left: 0.625rem;
  border-left: 1px solid var(--service-border-strong);
`;

const ModelName = styled.span`
  overflow: hidden;
  color: var(--service-text);
  font-family: var(--font-mono);
  font-size: 0.6875rem;
  text-overflow: ellipsis;
  white-space: nowrap;
`;

const ModelValue = styled.span`
  color: var(--service-text);
  font-family: var(--font-mono);
  font-size: 0.6875rem;
  font-variant-numeric: tabular-nums;
`;

const ModelMeta = styled.span`
  grid-column: 1 / -1;
  color: var(--service-text-muted);
  font-size: 0.6875rem;
  line-height: 1.45;
`;

const NoDayActivity = styled.p`
  margin: 0;
  color: var(--service-text-muted);
  font-size: 0.75rem;
`;

function formatFullDay(date: string): string {
  const timestamp = parseUtcDate(date);
  return timestamp === null ? date : fullDayFormatter.format(timestamp);
}

function getClientName(client: ClientType): string {
  return SOURCE_DISPLAY_NAMES[client] ?? client;
}

function getClientColor(
  client: ClientType,
  palette: GraphColorPalette,
): string {
  return SOURCE_COLORS[client] ?? palette.grade2;
}

function modelMeta(model: ContributionModelDetail): string {
  const metrics = TOKEN_CATEGORIES.flatMap(([label, key]) =>
    model.tokens[key] > 0
      ? [`${label} ${formatTokenCount(model.tokens[key])}`]
      : [],
  );
  if (model.messages > 0) {
    metrics.push(
      `${model.messages.toLocaleString("en-US")} ${
        model.messages === 1 ? "message" : "messages"
      }`,
    );
  }
  return metrics.join(" · ");
}

function ContributionDayTooltip({ day }: { day: DailyContribution }) {
  const clients = createContributionClientDetails(day);
  const messageCount = getContributionDayMessageCount(day, clients);
  const visibleCategories = TOKEN_CATEGORIES.filter(
    ([, key]) => day.tokenBreakdown[key] > 0,
  );

  return (
    <>
      <CellTooltipDate>{formatFullDay(day.date)}</CellTooltipDate>
      <TooltipTotal>
        <TooltipTotalLabel>Total tokens</TooltipTotalLabel>
        <CellTooltipValue>
          {formatTokenCount(day.totals.tokens)}
        </CellTooltipValue>
      </TooltipTotal>
      <TooltipDivider />
      {visibleCategories.length > 0 && (
        <TooltipMetricGrid>
          {visibleCategories.map(([label, key]) => (
            <Fragment key={key}>
              <TooltipMetricLabel>{label}</TooltipMetricLabel>
              <TooltipMetricValue>
                {formatTokenCount(day.tokenBreakdown[key])}
              </TooltipMetricValue>
            </Fragment>
          ))}
        </TooltipMetricGrid>
      )}
      <TooltipMetricGrid>
        <TooltipMetricLabel>Cost</TooltipMetricLabel>
        <TooltipMetricValue>
          {formatCurrency(day.totals.cost)}
        </TooltipMetricValue>
        <TooltipMetricLabel>Messages</TooltipMetricLabel>
        <TooltipMetricValue>
          {messageCount.toLocaleString("en-US")}
        </TooltipMetricValue>
      </TooltipMetricGrid>
      {clients.length > 0 && (
        <>
          <TooltipDivider />
          <TooltipSectionLabel>Clients</TooltipSectionLabel>
          <TooltipMetricGrid>
            {clients.slice(0, 3).map((client) => (
              <Fragment key={client.client}>
                <TooltipMetricLabel>
                  {getClientName(client.client)}
                </TooltipMetricLabel>
                <TooltipMetricValue>
                  {formatTokenCount(client.totalTokens)}
                </TooltipMetricValue>
              </Fragment>
            ))}
            {clients.length > 3 && (
              <>
                <TooltipMetricLabel>
                  +{clients.length - 3} more
                </TooltipMetricLabel>
                <TooltipMetricValue>Click for detail</TooltipMetricValue>
              </>
            )}
          </TooltipMetricGrid>
        </>
      )}
    </>
  );
}

function ContributionDayBreakdown({
  className,
  day,
  id,
  onClose,
  palette,
  standalone = false,
}: {
  className?: string;
  day: DailyContribution;
  id: string;
  onClose?: () => void;
  palette: GraphColorPalette;
  standalone?: boolean;
}) {
  const headingId = `${id}-heading`;
  const clients = createContributionClientDetails(day);
  const messageCount = getContributionDayMessageCount(day, clients);

  return (
    <DetailPanel
      id={id}
      aria-labelledby={headingId}
      className={className}
      $standalone={standalone}
    >
      <DetailHeader>
        <div>
          <DetailEyebrow>Day breakdown</DetailEyebrow>
          <DetailTitle id={headingId}>{formatFullDay(day.date)}</DetailTitle>
        </div>
        {onClose && (
          <DetailClose
            type="button"
            onClick={onClose}
            aria-label="Close day breakdown"
          >
            <svg
              aria-hidden="true"
              fill="none"
              height="14"
              viewBox="0 0 14 14"
              width="14"
            >
              <path
                d="M3 3l8 8M11 3l-8 8"
                stroke="currentColor"
                strokeLinecap="round"
                strokeWidth="1.5"
              />
            </svg>
          </DetailClose>
        )}
      </DetailHeader>
      <DetailBody>
        <DetailSummary>
          <DetailMetric>
            <DetailMetricLabel>Total tokens</DetailMetricLabel>
            <DetailMetricValue>
              {formatTokenCount(day.totals.tokens)}
            </DetailMetricValue>
          </DetailMetric>
          <DetailMetric>
            <DetailMetricLabel>Cost</DetailMetricLabel>
            <DetailMetricValue>
              {formatCurrency(day.totals.cost)}
            </DetailMetricValue>
          </DetailMetric>
          <DetailMetric>
            <DetailMetricLabel>Messages</DetailMetricLabel>
            <DetailMetricValue>
              {messageCount.toLocaleString("en-US")}
            </DetailMetricValue>
          </DetailMetric>
        </DetailSummary>

        <DetailSection>
          <DetailSectionTitle>Token categories</DetailSectionTitle>
          <TokenDetailGrid>
            {TOKEN_CATEGORIES.map(([label, key]) => (
              <DetailMetric key={key}>
                <DetailMetricLabel>{label}</DetailMetricLabel>
                <DetailMetricValue>
                  {formatTokenCount(day.tokenBreakdown[key])}
                </DetailMetricValue>
              </DetailMetric>
            ))}
          </TokenDetailGrid>
        </DetailSection>

        <DetailSection>
          <DetailSectionTitle>Clients and models</DetailSectionTitle>
          {clients.length > 0 ? (
            <ClientList
              $standalone={standalone}
              tabIndex={standalone ? undefined : 0}
              aria-label="Client and model details"
            >
              {clients.map((client) => (
                <ClientSection key={client.client}>
                  <ClientHeader>
                    <ClientIdentity>
                      <ClientDot
                        $color={getClientColor(client.client, palette)}
                        aria-hidden="true"
                      />
                      <ClientName>{getClientName(client.client)}</ClientName>
                    </ClientIdentity>
                    <ClientTotal>
                      {formatTokenCount(client.totalTokens)} ·{" "}
                      {formatCurrency(client.cost)}
                    </ClientTotal>
                  </ClientHeader>
                  {client.models.length > 0 && (
                    <ModelList>
                      {client.models.map((model) => (
                        <ModelRow
                          key={`${model.providerId ?? ""}-${model.modelId}`}
                        >
                          <ModelName title={model.modelId}>
                            {model.modelId}
                          </ModelName>
                          <ModelValue>{formatCurrency(model.cost)}</ModelValue>
                          <ModelMeta>
                            {model.providerId && `${model.providerId} · `}
                            {formatTokenCount(model.totalTokens)} tokens
                            {modelMeta(model) && ` · ${modelMeta(model)}`}
                          </ModelMeta>
                        </ModelRow>
                      ))}
                    </ModelList>
                  )}
                </ClientSection>
              ))}
            </ClientList>
          ) : (
            <NoDayActivity>
              No client or model detail was recorded for this day.
            </NoDayActivity>
          )}
        </DetailSection>
      </DetailBody>
    </DetailPanel>
  );
}

export function ProfileContributionBreakdown({
  className,
  day,
  id,
  onClose,
  paletteName = DEFAULT_PALETTE,
}: ProfileContributionBreakdownProps) {
  return (
    <ContributionDayBreakdown
      className={className}
      day={day}
      id={id}
      onClose={onClose}
      palette={getPalette(paletteName)}
      standalone
    />
  );
}

const EmptyState = styled.div`
  padding: 1.5rem 1rem;
  color: var(--service-text-muted);
  font-size: 0.8125rem;
  text-align: center;
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

export function ProfileContributionGraph({
  breakdownId: providedBreakdownId,
  className,
  contributions,
  description = "Daily token activity across the available history.",
  onPaletteChange,
  onSelectedDateChange,
  onViewChange,
  paletteName: providedPaletteName,
  persistentSelection = false,
  rangeEnd,
  rangeStart,
  selectedDate: providedSelectedDate,
  showBreakdown = true,
  view: providedView,
}: ProfileContributionGraphProps) {
  const titleId = useId();
  const descriptionId = useId();
  const tooltipId = useId();
  const generatedBreakdownId = useId();
  const breakdownId = providedBreakdownId ?? generatedBreakdownId;
  const calendarId = useId();
  const calendarInstructionsId = useId();
  const cellRefs = useRef(new Map<string, { focus: () => void }>());
  const [tooltip, setTooltip] = useState<ContributionTooltipState | null>(null);
  const [keyboardDate, setKeyboardDate] = useState<string | null>(null);
  const [internalSelectedDate, setInternalSelectedDate] = useState<
    string | null
  >(null);
  const [internalPaletteName, setInternalPaletteName] =
    useState<ColorPaletteName>(DEFAULT_PALETTE);
  const [internalView, setInternalView] =
    useState<ProfileContributionView>("2d");
  const selectedDate =
    providedSelectedDate === undefined
      ? internalSelectedDate
      : providedSelectedDate;
  const paletteName = providedPaletteName ?? internalPaletteName;
  const view = providedView ?? internalView;
  const palette = useMemo(() => getPalette(paletteName), [paletteName]);
  const contributionDays = useMemo(
    () => mergeDailyContributions(contributions),
    [contributions],
  );
  const calendar = useMemo(
    () => createContributionCalendar(contributions, rangeStart, rangeEnd),
    [contributions, rangeStart, rangeEnd],
  );
  const activeDayLabel = `${calendar.activeDays.toLocaleString("en-US")} active ${
    calendar.activeDays === 1 ? "day" : "days"
  }`;
  const accessibleDetail = calendar.highestDay
    ? `Highest activity: ${cellTitle(calendar.highestDay)}. ${calendar.freeTokenDays.toLocaleString(
        "en-US",
      )} active days used tokens with no recorded cost.`
    : "No active contribution days are available.";
  const inRangeDates = useMemo(
    () =>
      calendar.cells.filter(({ inRange }) => inRange).map(({ date }) => date),
    [calendar.cells],
  );
  const tabbableDate =
    keyboardDate && inRangeDates.includes(keyboardDate)
      ? keyboardDate
      : (calendar.endDate ?? inRangeDates.at(-1) ?? null);
  const selectedCell = selectedDate
    ? (calendar.cells.find(
        ({ date, inRange }) => inRange && date === selectedDate,
      ) ?? null)
    : null;
  const selectedDay = selectedCell
    ? (contributionDays.get(selectedCell.date) ??
      createEmptyContribution(selectedCell))
    : null;
  const isometricGeometry = useMemo(
    () => createContributionIsometricGeometry(calendar),
    [calendar],
  );

  const commitSelectedDate = (date: string | null) => {
    if (providedSelectedDate === undefined) setInternalSelectedDate(date);
    onSelectedDateChange?.(date);
  };

  const commitPalette = (name: ColorPaletteName) => {
    if (providedPaletteName === undefined) setInternalPaletteName(name);
    onPaletteChange?.(name);
  };

  const commitView = (nextView: ProfileContributionView) => {
    if (providedView === undefined) setInternalView(nextView);
    onViewChange?.(nextView);
  };

  const positionCellTooltip = (cell: ContributionCell, target: Element) => {
    if (
      !cell.inRange ||
      cell.date === selectedDate ||
      typeof window === "undefined"
    ) {
      setTooltip(null);
      return;
    }

    const cellBounds = target.getBoundingClientRect();
    const gutter = 12;
    const tooltipWidth = Math.min(280, window.innerWidth - gutter * 2);
    const estimatedHeight = Math.min(360, window.innerHeight - gutter * 2);
    const cellCenter = cellBounds.left + cellBounds.width / 2;
    const left = Math.max(
      gutter,
      Math.min(
        window.innerWidth - tooltipWidth - gutter,
        cellCenter - tooltipWidth / 2,
      ),
    );
    const preferredTop = cellBounds.top - estimatedHeight - 8;
    const fallbackTop = cellBounds.bottom + 8;
    const top = Math.max(
      gutter,
      Math.min(
        window.innerHeight - estimatedHeight - gutter,
        preferredTop >= gutter ? preferredTop : fallbackTop,
      ),
    );

    setTooltip({
      cell,
      day: contributionDays.get(cell.date) ?? createEmptyContribution(cell),
      left,
      top,
    });
  };

  const handleCellPointerEnter = (
    cell: ContributionCell,
    event: PointerEvent<Element>,
  ) => positionCellTooltip(cell, event.currentTarget);

  const handleCellFocus = (
    cell: ContributionCell,
    event: FocusEvent<Element>,
  ) => {
    setKeyboardDate(cell.date);
    positionCellTooltip(cell, event.currentTarget);
  };

  const handleCellKeyDown = (
    cell: ContributionCell,
    event: KeyboardEvent<Element>,
  ) => {
    if (event.key === "Escape") {
      event.preventDefault();
      setTooltip(null);
      if (!persistentSelection) commitSelectedDate(null);
      return;
    }

    if (event.key === "Enter" || event.key === " ") {
      event.preventDefault();
      setTooltip(null);
      commitSelectedDate(
        selectedDate === cell.date && !persistentSelection ? null : cell.date,
      );
      return;
    }

    if (
      ![
        "ArrowDown",
        "ArrowLeft",
        "ArrowRight",
        "ArrowUp",
        "End",
        "Home",
      ].includes(event.key)
    ) {
      return;
    }

    event.preventDefault();
    const nextDate = getContributionFocusDate(
      calendar.cells,
      cell.date,
      event.key as ContributionNavigationKey,
    );
    if (!nextDate) return;

    setKeyboardDate(nextDate);
    cellRefs.current.get(nextDate)?.focus();
  };

  const closeSelectedDay = () => {
    const date = selectedDate;
    commitSelectedDate(null);
    if (date) requestAnimationFrame(() => cellRefs.current.get(date)?.focus());
  };

  const selectCell = (cell: ContributionCell) => {
    if (!cell.inRange) return;
    setTooltip(null);
    setKeyboardDate(cell.date);
    commitSelectedDate(
      selectedDate === cell.date && !persistentSelection ? null : cell.date,
    );
  };

  return (
    <Figure
      aria-describedby={descriptionId}
      aria-labelledby={titleId}
      className={className}
    >
      <Header>
        <HeadingGroup>
          <Heading id={titleId}>Contributions</Heading>
          <Description id={descriptionId}>{description}</Description>
        </HeadingGroup>
        <HeaderAside>
          <ViewToggle role="group" aria-label="Contribution graph view">
            {(["2d", "3d"] as const).map((option) => (
              <ViewButton
                key={option}
                type="button"
                $active={view === option}
                aria-controls={calendarId}
                aria-pressed={view === option}
                onClick={() => commitView(option)}
              >
                {option.toUpperCase()}
              </ViewButton>
            ))}
          </ViewToggle>
          <Summary>
            <ActiveDays>{activeDayLabel}</ActiveDays>
            <Range>{formatRange(calendar.startDate, calendar.endDate)}</Range>
          </Summary>
        </HeaderAside>
      </Header>
      <VisuallyHidden>{accessibleDetail}</VisuallyHidden>

      {calendar.weekCount > 0 ? (
        <>
          {view === "2d" ? (
            <CalendarBody id={calendarId}>
              <MonthRow $weeks={calendar.weekCount} aria-hidden="true">
                {calendar.monthMarkers.map((marker) => (
                  <Month
                    key={`${marker.weekIndex}-${marker.label}`}
                    $compactVisible={marker.compactVisible}
                    $week={marker.weekIndex}
                  >
                    {marker.label}
                  </Month>
                ))}
              </MonthRow>
              <CalendarRow>
                <DayLabels aria-hidden="true">
                  <DayLabel $row={2}>Mon</DayLabel>
                  <DayLabel $row={4}>Wed</DayLabel>
                  <DayLabel $row={6}>Fri</DayLabel>
                </DayLabels>
                <Grid
                  $weeks={calendar.weekCount}
                  role="group"
                  aria-label="Daily token contributions"
                  aria-describedby={calendarInstructionsId}
                >
                  {calendar.cells.map((cell) => (
                    <Cell
                      key={cell.date}
                      type="button"
                      ref={(node) => {
                        if (node) cellRefs.current.set(cell.date, node);
                        else cellRefs.current.delete(cell.date);
                      }}
                      disabled={!cell.inRange}
                      tabIndex={
                        cell.inRange && cell.date === tabbableDate ? 0 : -1
                      }
                      aria-hidden={cell.inRange ? undefined : true}
                      aria-label={cell.inRange ? cellTitle(cell) : undefined}
                      aria-current={
                        cell.inRange && cell.date === selectedDate
                          ? "date"
                          : undefined
                      }
                      aria-pressed={
                        cell.inRange ? cell.date === selectedDate : undefined
                      }
                      aria-controls={
                        cell.inRange && cell.date === selectedDate
                          ? breakdownId
                          : undefined
                      }
                      aria-describedby={
                        tooltip?.cell.date === cell.date ? tooltipId : undefined
                      }
                      data-contribution-date={
                        cell.inRange ? cell.date : undefined
                      }
                      $active={tooltip?.cell.date === cell.date}
                      $color={getContributionColor(palette, cell.intensity)}
                      $inRange={cell.inRange}
                      $selected={cell.date === selectedDate}
                      onClick={() => selectCell(cell)}
                      onPointerEnter={(event) =>
                        handleCellPointerEnter(cell, event)
                      }
                      onPointerLeave={(event) => {
                        if (document.activeElement !== event.currentTarget) {
                          setTooltip(null);
                        }
                      }}
                      onFocus={(event) => handleCellFocus(cell, event)}
                      onBlur={() => setTooltip(null)}
                      onKeyDown={(event) => handleCellKeyDown(cell, event)}
                    />
                  ))}
                </Grid>
              </CalendarRow>
            </CalendarBody>
          ) : (
            <IsometricBody id={calendarId}>
              <IsometricSvg
                viewBox={`0 0 ${isometricGeometry.viewBox.width} ${isometricGeometry.viewBox.height}`}
                role="group"
                aria-label="Isometric daily token contributions"
                aria-describedby={calendarInstructionsId}
                preserveAspectRatio="xMidYMid meet"
              >
                {isometricGeometry.cells.map((geometry) => {
                  const { cell } = geometry;
                  const faces = contributionCubeFaces(geometry);
                  const color = getContributionColor(palette, cell.intensity);
                  const active = tooltip?.cell.date === cell.date;
                  const selected = selectedDate === cell.date;

                  return (
                    <IsometricCell
                      key={cell.date}
                      ref={(node) => {
                        if (node) cellRefs.current.set(cell.date, node);
                        else cellRefs.current.delete(cell.date);
                      }}
                      role="button"
                      tabIndex={cell.date === tabbableDate ? 0 : -1}
                      aria-label={cellTitle(cell)}
                      aria-current={selected ? "date" : undefined}
                      aria-pressed={selected}
                      aria-controls={selected ? breakdownId : undefined}
                      aria-describedby={active ? tooltipId : undefined}
                      data-contribution-date={cell.date}
                      data-contribution-view="3d"
                      $active={active}
                      $selected={selected}
                      onClick={() => selectCell(cell)}
                      onPointerEnter={(event) =>
                        handleCellPointerEnter(cell, event)
                      }
                      onPointerLeave={(event) => {
                        if (document.activeElement !== event.currentTarget) {
                          setTooltip(null);
                        }
                      }}
                      onFocus={(event) => handleCellFocus(cell, event)}
                      onBlur={() => setTooltip(null)}
                      onKeyDown={(event) => handleCellKeyDown(cell, event)}
                    >
                      <polygon
                        points={faces.left}
                        fill={shadeContributionColor(color, 58)}
                      />
                      <polygon
                        points={faces.right}
                        fill={shadeContributionColor(color, 72)}
                      />
                      <IsometricTop
                        points={faces.top}
                        fill={color}
                        $active={active}
                        $selected={selected}
                      />
                    </IsometricCell>
                  );
                })}
              </IsometricSvg>
            </IsometricBody>
          )}
          {tooltip && tooltip.cell.date !== selectedDate && (
            <CellTooltip
              id={tooltipId}
              role="tooltip"
              data-contribution-tooltip
              $left={tooltip.left}
              $top={tooltip.top}
            >
              <ContributionDayTooltip day={tooltip.day} />
            </CellTooltip>
          )}
          <Footer>
            <PaletteControl>
              <span>Color</span>
              <PalettePreview aria-hidden="true">
                {([1, 2, 3, 4] as const).map((level) => (
                  <PalettePreviewSwatch
                    key={level}
                    $color={getContributionColor(palette, level)}
                  />
                ))}
              </PalettePreview>
              <PaletteSelect
                aria-label="Contribution graph color"
                value={paletteName}
                onChange={(event) =>
                  commitPalette(event.currentTarget.value as ColorPaletteName)
                }
              >
                {getPaletteNames().map((name) => (
                  <option key={name} value={name}>
                    {colorPalettes[name].name}
                  </option>
                ))}
              </PaletteSelect>
            </PaletteControl>
            <Legend aria-label="Contribution intensity, low to high">
              <span>Low</span>
              <LegendSwatches>
                {[0, 1, 2, 3, 4].map((level) => (
                  <LegendSwatch
                    key={level}
                    $color={getContributionColor(
                      palette,
                      level as ContributionCell["intensity"],
                    )}
                  />
                ))}
              </LegendSwatches>
              <span>High</span>
            </Legend>
          </Footer>
          {showBreakdown && selectedDay && (
            <ContributionDayBreakdown
              day={selectedDay}
              id={breakdownId}
              onClose={closeSelectedDay}
              palette={palette}
            />
          )}
        </>
      ) : (
        <EmptyState>No contribution data is available.</EmptyState>
      )}
      <VisuallyHidden id={calendarInstructionsId}>
        Use arrow keys to inspect adjacent days, Home and End to jump to the
        range boundaries, Enter or Space to select the detailed day breakdown,
        and Escape to close the floating tooltip.
      </VisuallyHidden>
    </Figure>
  );
}
