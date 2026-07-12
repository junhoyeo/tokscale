import { SOURCE_COLORS, SOURCE_DISPLAY_NAMES } from "@/lib/constants";
import type {
  ClientType,
  DailyContribution,
  ModelBreakdownData,
  TokenBreakdown,
} from "@/lib/types";

export const ALL_USAGE_PROVIDERS = "all" as const;
export const OTHER_USAGE_PROVIDERS = "other" as const;
export const UNATTRIBUTED_USAGE_PROVIDER = "unattributed" as const;

export const BLANK_USAGE_MODEL = "__blank-model__" as const;
export const PROVIDER_REMAINDER_USAGE_MODEL = "__provider-remainder__" as const;
export const DAILY_REMAINDER_USAGE_MODEL = "__daily-remainder__" as const;
export const SERIES_REMAINDER_USAGE_MODEL = "__series-remainder__" as const;

export const DEFAULT_USAGE_AVERAGE_WINDOW = 30;
export const MAX_USAGE_CHART_SERIES = 128;

/** @deprecated Model-aware charts use MAX_USAGE_CHART_SERIES instead. */
export const MAX_VISIBLE_USAGE_PROVIDERS = 6;

export type UsageMetric = "tokens" | "cost";
export type UsageView = "average" | "daily";
export type UsageProviderId = ClientType | typeof UNATTRIBUTED_USAGE_PROVIDER;
export type UsageProviderFilter = UsageProviderId | typeof ALL_USAGE_PROVIDERS;
export type UsageSeriesId = string;
export type UsageSeriesKind =
  | "model"
  | "blank-model"
  | "synthetic"
  | "provider-remainder"
  | "daily-remainder"
  | "series-remainder";

export interface AggregatedModelUsage {
  model: string;
  kind: Exclude<UsageSeriesKind, "series-remainder">;
  tokens: number;
  cost: number;
}

export interface AggregatedProviderUsage {
  provider: UsageProviderId;
  tokens: number;
  cost: number;
  models: AggregatedModelUsage[];
}

export interface AggregatedUsageDay {
  date: string;
  providers: AggregatedProviderUsage[];
}

export interface UsageProviderTotal {
  provider: UsageProviderId;
  tokens: number;
  cost: number;
}

export interface UsageChartSeries {
  id: UsageSeriesId;
  label: string;
  providerLabel: string;
  provider: UsageProviderId | null;
  model: string;
  kind: UsageSeriesKind;
  color: string;
  providers: UsageProviderId[];
  /** Raw values are retained for range totals, ranking, and exact inspection. */
  rawValues: number[];
  /** Display values are raw daily values or their trailing moving average. */
  values: number[];
  /** Always the raw range total, regardless of the selected view. */
  total: number;
}

export interface UsageChartData {
  dates: string[];
  series: UsageChartSeries[];
  view: UsageView;
  averageWindowDays: number;
  rawDailyTotals: number[];
  dailyTotals: number[];
  rawMaxDailyTotal: number;
  maxDailyTotal: number;
  /** Always the raw range total, regardless of the selected view. */
  total: number;
}

export interface UsageTooltipRow {
  series: UsageChartSeries;
  value: number;
  rawValue: number;
}

interface MutableModelUsage {
  provider: UsageProviderId;
  model: string;
  kind: Exclude<UsageSeriesKind, "series-remainder">;
  tokens: number;
  cost: number;
}

interface SeriesCandidateSummary {
  id: UsageSeriesId;
  provider: UsageProviderId;
  model: string;
  kind: Exclude<UsageSeriesKind, "series-remainder">;
  total: number;
  shadeCost: number;
}

interface SeriesCandidate extends SeriesCandidateSummary {
  rawValues: number[];
}

interface UsageCandidateMatrix {
  candidates: SeriesCandidateSummary[];
  valuesByDay: Map<UsageSeriesId, number>[];
}

function finiteUsage(value: number | null | undefined): number {
  return Number.isFinite(value) ? Math.max(0, value ?? 0) : 0;
}

/**
 * Token totals are inclusive: cache reads, cache writes, and reasoning are
 * usage alongside input and output.
 */
export function sumTokenBreakdown(tokens: TokenBreakdown): number {
  return (
    finiteUsage(tokens.input) +
    finiteUsage(tokens.output) +
    finiteUsage(tokens.cacheRead) +
    finiteUsage(tokens.cacheWrite) +
    finiteUsage(tokens.reasoning)
  );
}

function inclusiveModelTokens(model: ModelBreakdownData): number {
  const componentTotal =
    finiteUsage(model.input) +
    finiteUsage(model.output) +
    finiteUsage(model.cacheRead) +
    finiteUsage(model.cacheWrite) +
    finiteUsage(model.reasoning);

  // Old database rows can have incomplete component fields, while rows from
  // newer clients can have an out-of-date scalar. Retain whichever inclusive
  // representation contains more usage rather than silently dropping it.
  return Math.max(finiteUsage(model.tokens), componentTotal);
}

function normalizeModelId(model: string | null | undefined): string {
  return model?.trim() || BLANK_USAGE_MODEL;
}

function kindForModel(model: string): AggregatedModelUsage["kind"] {
  if (model === BLANK_USAGE_MODEL) return "blank-model";
  if (model === "<synthetic>") return "synthetic";
  if (model === PROVIDER_REMAINDER_USAGE_MODEL) return "provider-remainder";
  if (model === DAILY_REMAINDER_USAGE_MODEL) return "daily-remainder";
  return "model";
}

function usageSeriesId(provider: UsageProviderId, model: string): string {
  return `${encodeURIComponent(provider)}::${encodeURIComponent(model)}`;
}

function addModelUsage(
  models: Map<string, MutableModelUsage>,
  provider: UsageProviderId,
  rawModel: string,
  tokens: number,
  cost: number,
): void {
  const model = normalizeModelId(rawModel);
  const id = usageSeriesId(provider, model);
  const current = models.get(id) ?? {
    provider,
    model,
    kind: kindForModel(model),
    tokens: 0,
    cost: 0,
  };
  current.tokens += finiteUsage(tokens);
  current.cost += finiteUsage(cost);
  models.set(id, current);
}

type UsageMeasure = "tokens" | "cost";

/**
 * Make nested attribution agree with its authoritative parent without ever
 * emitting a negative model. Over-attribution is scaled proportionally;
 * meaningful under-attribution is returned for an explicit remainder row.
 */
function reconcileAttribution(
  usages: MutableModelUsage[],
  measure: UsageMeasure,
  rawTarget: number,
): number {
  const target = finiteUsage(rawTarget);
  for (const usage of usages) usage[measure] = finiteUsage(usage[measure]);

  const attributed = usages.reduce((sum, usage) => sum + usage[measure], 0);
  if (attributed === 0) return target;

  if (attributed > target) {
    let finalPositiveIndex = usages.length - 1;
    while (
      finalPositiveIndex > 0 &&
      usages[finalPositiveIndex][measure] === 0
    ) {
      finalPositiveIndex -= 1;
    }
    let assigned = 0;
    usages.forEach((usage, index) => {
      if (index === finalPositiveIndex) return;
      usage[measure] = (usage[measure] / attributed) * target;
      assigned += usage[measure];
    });
    usages[finalPositiveIndex][measure] = Math.max(0, target - assigned);
    return 0;
  }

  const difference = target - attributed;
  const tolerance = Math.max(1e-9, target * 1e-12);
  if (difference <= tolerance) {
    // Absorb floating-point dust into an existing model instead of creating a
    // tiny visible remainder series.
    usages[usages.length - 1][measure] += difference;
    return 0;
  }
  return difference;
}

function authoritativeUsage(rawTotal: number, fallback: number): number {
  return Number.isFinite(rawTotal)
    ? finiteUsage(rawTotal)
    : finiteUsage(fallback);
}

/**
 * Merge duplicate dates and duplicate provider/model rows without mutating the
 * API payload. Nested database models and flat CLI model rows share one
 * provider+model key, while gaps at each aggregation boundary remain visible
 * as explicit remainder series.
 */
export function aggregateDailyUsage(
  contributions: readonly DailyContribution[],
  rangeStart?: string,
  rangeEnd?: string,
): AggregatedUsageDay[] {
  const byDate = new Map<
    string,
    {
      models: Map<string, MutableModelUsage>;
      totals: { tokens: number; cost: number };
    }
  >();

  for (const day of contributions) {
    const aggregate = byDate.get(day.date) ?? {
      models: new Map<string, MutableModelUsage>(),
      totals: { tokens: 0, cost: 0 },
    };
    const contributedTokens = day.clients.reduce(
      (sum, client) => sum + sumTokenBreakdown(client.tokens),
      0,
    );
    const contributedCost = day.clients.reduce(
      (sum, client) => sum + finiteUsage(client.cost),
      0,
    );
    aggregate.totals.tokens += authoritativeUsage(
      day.totals.tokens,
      contributedTokens,
    );
    aggregate.totals.cost += authoritativeUsage(
      day.totals.cost,
      contributedCost,
    );

    for (const client of day.clients) {
      const provider = client.client;
      const clientTokens = sumTokenBreakdown(client.tokens);
      const clientCost = finiteUsage(client.cost);
      const nestedModels = Object.entries(client.models ?? {});

      if (nestedModels.length === 0) {
        addModelUsage(
          aggregate.models,
          provider,
          client.modelId,
          clientTokens,
          clientCost,
        );
        continue;
      }

      const clientModels = nestedModels.map(([rawModel, modelUsage]) => {
        const model = normalizeModelId(rawModel);
        return {
          provider,
          model,
          kind: kindForModel(model),
          tokens: inclusiveModelTokens(modelUsage),
          cost: finiteUsage(modelUsage.cost),
        } satisfies MutableModelUsage;
      });
      const tokenRemainder = reconcileAttribution(
        clientModels,
        "tokens",
        clientTokens,
      );
      const costRemainder = reconcileAttribution(
        clientModels,
        "cost",
        clientCost,
      );
      if (tokenRemainder > 0 || costRemainder > 0) {
        clientModels.push({
          provider,
          model: PROVIDER_REMAINDER_USAGE_MODEL,
          kind: "provider-remainder",
          tokens: tokenRemainder,
          cost: costRemainder,
        });
      }
      for (const modelUsage of clientModels) {
        addModelUsage(
          aggregate.models,
          provider,
          modelUsage.model,
          modelUsage.tokens,
          modelUsage.cost,
        );
      }
    }

    byDate.set(day.date, aggregate);
  }

  const observedDays = [...byDate.entries()]
    .sort(([left], [right]) => left.localeCompare(right))
    .map(([date, aggregate]) => {
      const models = [...aggregate.models.values()];
      const tokenRemainder = reconcileAttribution(
        models,
        "tokens",
        aggregate.totals.tokens,
      );
      const costRemainder = reconcileAttribution(
        models,
        "cost",
        aggregate.totals.cost,
      );
      if (tokenRemainder > 0 || costRemainder > 0) {
        addModelUsage(
          aggregate.models,
          UNATTRIBUTED_USAGE_PROVIDER,
          DAILY_REMAINDER_USAGE_MODEL,
          tokenRemainder,
          costRemainder,
        );
      }

      const grouped = new Map<UsageProviderId, MutableModelUsage[]>();
      for (const usage of aggregate.models.values()) {
        const models = grouped.get(usage.provider) ?? [];
        models.push(usage);
        grouped.set(usage.provider, models);
      }

      return {
        date,
        providers: [...grouped.entries()]
          .sort(([left], [right]) => left.localeCompare(right))
          .map(([provider, models]) => {
            const sortedModels = models
              .sort((left, right) => left.model.localeCompare(right.model))
              .map(({ model, kind, tokens, cost }) => ({
                model,
                kind,
                tokens,
                cost,
              }));
            return {
              provider,
              tokens: sortedModels.reduce(
                (sum, model) => sum + model.tokens,
                0,
              ),
              cost: sortedModels.reduce((sum, model) => sum + model.cost, 0),
              models: sortedModels,
            };
          }),
      };
    });

  return fillMissingUsageDays(observedDays, rangeStart, rangeEnd);
}

const DAY_MS = 24 * 60 * 60 * 1000;
const MAX_CHART_DAYS = 370;

/** Keep the time axis honest by representing missing UTC calendar days as zero. */
export function fillMissingUsageDays(
  observedDays: readonly AggregatedUsageDay[],
  rangeStart?: string,
  rangeEnd?: string,
): AggregatedUsageDay[] {
  const explicitStart = rangeStart == null ? null : parseUtcDay(rangeStart);
  const explicitEnd = rangeEnd == null ? null : parseUtcDay(rangeEnd);
  if (
    (rangeStart != null && explicitStart == null) ||
    (rangeEnd != null && explicitEnd == null)
  ) {
    return [...observedDays];
  }

  const validObservedDays = observedDays
    .filter((day) => parseUtcDay(day.date) != null)
    .sort((left, right) => left.date.localeCompare(right.date));
  if (
    validObservedDays.length !== observedDays.length &&
    rangeStart == null &&
    rangeEnd == null
  ) {
    return [...observedDays];
  }

  const start =
    explicitStart ??
    (validObservedDays.length > 0
      ? parseUtcDay(validObservedDays[0].date)
      : null);
  const end =
    explicitEnd ??
    (validObservedDays.length > 0
      ? parseUtcDay(validObservedDays[validObservedDays.length - 1].date)
      : null);
  if (start == null || end == null || end < start) return [...observedDays];

  const dayCount = Math.floor((end - start) / DAY_MS) + 1;
  if (dayCount > MAX_CHART_DAYS) return [...observedDays];

  const byObservedDate = new Map(
    validObservedDays
      .filter((day) => {
        const timestamp = parseUtcDay(day.date);
        return timestamp != null && timestamp >= start && timestamp <= end;
      })
      .map((day) => [day.date, day]),
  );
  return Array.from({ length: dayCount }, (_, index) => {
    const date = new Date(start + index * DAY_MS).toISOString().slice(0, 10);
    return byObservedDate.get(date) ?? { date, providers: [] };
  });
}

function parseUtcDay(value: string): number | null {
  if (!/^\d{4}-\d{2}-\d{2}$/.test(value)) return null;
  const [year, month, day] = value.split("-").map(Number);
  const timestamp = Date.UTC(year, month - 1, day);
  return new Date(timestamp).toISOString().slice(0, 10) === value
    ? timestamp
    : null;
}

/** Returns providers in descending raw usage order for picker presentation. */
export function getUsageProviderTotals(
  days: readonly AggregatedUsageDay[],
  metric: UsageMetric,
): UsageProviderTotal[] {
  const totals = new Map<UsageProviderId, { tokens: number; cost: number }>();

  for (const day of days) {
    for (const provider of day.providers) {
      const current = totals.get(provider.provider) ?? { tokens: 0, cost: 0 };
      current.tokens += finiteUsage(provider.tokens);
      current.cost += finiteUsage(provider.cost);
      totals.set(provider.provider, current);
    }
  }

  return [...totals.entries()]
    .map(([provider, usage]) => ({ provider, ...usage }))
    .sort(
      (left, right) =>
        right[metric] - left[metric] ||
        left.provider.localeCompare(right.provider),
    );
}

export function toTrailingAverage(
  values: readonly number[],
  requestedWindow = DEFAULT_USAGE_AVERAGE_WINDOW,
): number[] {
  const window = Math.max(1, Math.floor(finiteUsage(requestedWindow)) || 1);
  let rollingTotal = 0;

  return values.map((rawValue, index) => {
    rollingTotal += finiteUsage(rawValue);
    if (index >= window) rollingTotal -= finiteUsage(values[index - window]);

    // Leading points use the number of calendar days observed so far. This is
    // deliberately a partial-window divisor, not an implicit zero-filled 30d.
    return rollingTotal / Math.min(window, index + 1);
  });
}

function providerLabel(provider: UsageProviderId): string {
  if (provider === UNATTRIBUTED_USAGE_PROVIDER) return "Unattributed";
  return SOURCE_DISPLAY_NAMES[provider] ?? provider;
}

function modelLabel(
  provider: UsageProviderId,
  model: string,
  kind: UsageSeriesKind,
): string {
  if (kind === "blank-model") return "Unknown model";
  if (kind === "synthetic") return "Synthetic";
  if (kind === "provider-remainder") {
    return `${providerLabel(provider)} remainder`;
  }
  if (kind === "daily-remainder") return "Unattributed usage";
  return model;
}

function mixHexColor(base: string, target: string, amount: number): string {
  const parse = (value: string) => {
    const match = /^#([\da-f]{2})([\da-f]{2})([\da-f]{2})$/i.exec(value);
    return match?.slice(1).map((part) => Number.parseInt(part, 16)) ?? null;
  };
  const from = parse(base);
  const to = parse(target);
  if (!from || !to) return base;
  const clamped = Math.max(0, Math.min(1, amount));
  return `#${from
    .map((channel, index) =>
      Math.round(channel + (to[index] - channel) * clamped)
        .toString(16)
        .padStart(2, "0"),
    )
    .join("")}`;
}

function relativeLuminance(color: string): number | null {
  const match = /^#([\da-f]{2})([\da-f]{2})([\da-f]{2})$/i.exec(color);
  if (!match) return null;
  const channels = match.slice(1).map((part) => {
    const channel = Number.parseInt(part, 16) / 255;
    return channel <= 0.04045
      ? channel / 12.92
      : ((channel + 0.055) / 1.055) ** 2.4;
  });
  return channels[0] * 0.2126 + channels[1] * 0.7152 + channels[2] * 0.0722;
}

export function providerColor(provider: UsageProviderId): string {
  const base =
    provider === UNATTRIBUTED_USAGE_PROVIDER
      ? "#737373"
      : (SOURCE_COLORS[provider] ?? "#3b82f6");
  const canvasLuminance = relativeLuminance("#090d14") ?? 0;
  const baseLuminance = relativeLuminance(base);
  if (baseLuminance == null) return base;

  const contrast =
    (Math.max(baseLuminance, canvasLuminance) + 0.05) /
    (Math.min(baseLuminance, canvasLuminance) + 0.05);
  if (contrast >= 3) return base;

  // Several provider brand colors are nearly black. Lift only those colors
  // toward a cool neutral until their area/line remains legible on the dark
  // service canvas; brighter brand colors remain exact.
  for (let amount = 0.25; amount <= 1; amount += 0.05) {
    const candidate = mixHexColor(base, "#94a3b8", amount);
    const candidateLuminance = relativeLuminance(candidate) ?? 0;
    const candidateContrast =
      (Math.max(candidateLuminance, canvasLuminance) + 0.05) /
      (Math.min(candidateLuminance, canvasLuminance) + 0.05);
    if (candidateContrast >= 3) return candidate;
  }
  return "#94a3b8";
}

// Keep the model shade progression aligned with the TUI's configurable
// provider palettes. Each source retains its profile-chart color while model
// rank determines how far that color is mixed toward white.
const MODEL_SHADE_FACTORS = [0, 0.11, 0.22, 0.33, 0.44, 0.56, 0.67] as const;

function modelFamilyTier(model: string): number {
  const lower = model.toLowerCase();
  const tokens = lower.split(/[^a-z0-9]+/).filter(Boolean);
  if (tokens.includes("fable")) return 0;
  if (tokens.includes("opus")) return 1;
  if (tokens.includes("sonnet")) return 2;
  if (tokens.includes("haiku")) return 3;
  return 4;
}

function leadingNumber(token: string): number | null {
  const match = /^\d+/.exec(token);
  if (!match) return null;
  const value = Number.parseInt(match[0], 10);
  return Number.isFinite(value) ? value : null;
}

function modelVersion(model: string): readonly [number, number] {
  const tokens = model.split(/[^a-z0-9]+/i).filter(Boolean);

  for (let index = 0; index < tokens.length; index += 1) {
    const major = leadingNumber(tokens[index]);
    if (major == null) continue;
    // Four-digit values are dates, not model versions. As in the TUI, stop
    // instead of scanning later date fragments as a plausible version.
    if (major >= 1_000) return [0, 0];
    const nextToken = tokens[index + 1];
    const parsedMinor =
      nextToken != null && /^\d+$/.test(nextToken)
        ? Number.parseInt(nextToken, 10)
        : 0;
    const minor = parsedMinor < 1_000 ? parsedMinor : 0;
    return [major, minor];
  }

  return [0, 0];
}

function compareModelShadeRank(
  left: SeriesCandidateSummary,
  right: SeriesCandidateSummary,
): number {
  const leftVersion = modelVersion(left.model);
  const rightVersion = modelVersion(right.model);
  return (
    modelFamilyTier(left.model) - modelFamilyTier(right.model) ||
    rightVersion[0] - leftVersion[0] ||
    rightVersion[1] - leftVersion[1] ||
    right.shadeCost - left.shadeCost ||
    left.model.localeCompare(right.model) ||
    left.id.localeCompare(right.id)
  );
}

function modelShade(base: string, rank: number): string {
  const factor =
    MODEL_SHADE_FACTORS[Math.min(rank, MODEL_SHADE_FACTORS.length - 1)];
  return mixHexColor(base, "#ffffff", factor);
}

function candidatesForDays(
  days: readonly AggregatedUsageDay[],
  metric: UsageMetric,
  providerFilter: UsageProviderFilter,
): UsageCandidateMatrix {
  const bySeries = new Map<UsageSeriesId, SeriesCandidateSummary>();
  const valuesByDay = days.map(() => new Map<UsageSeriesId, number>());

  days.forEach((day, dayIndex) => {
    for (const provider of day.providers) {
      if (
        providerFilter !== ALL_USAGE_PROVIDERS &&
        provider.provider !== providerFilter
      ) {
        continue;
      }
      for (const model of provider.models) {
        const id = usageSeriesId(provider.provider, model.model);
        const candidate = bySeries.get(id) ?? {
          id,
          provider: provider.provider,
          model: model.model,
          kind: model.kind,
          total: 0,
          shadeCost: 0,
        };
        const value = finiteUsage(model[metric]);
        valuesByDay[dayIndex].set(
          id,
          (valuesByDay[dayIndex].get(id) ?? 0) + value,
        );
        candidate.total += value;
        candidate.shadeCost += finiteUsage(model.cost);
        bySeries.set(id, candidate);
      }
    }
  });

  const candidates = [...bySeries.values()].filter(({ total }) => total > 0);

  const providerTotals = new Map<UsageProviderId, number>();
  for (const candidate of candidates) {
    providerTotals.set(
      candidate.provider,
      (providerTotals.get(candidate.provider) ?? 0) + candidate.total,
    );
  }

  return {
    candidates: candidates.sort(
      (left, right) =>
        (providerTotals.get(left.provider) ?? 0) -
          (providerTotals.get(right.provider) ?? 0) ||
        left.provider.localeCompare(right.provider) ||
        left.total - right.total ||
        left.model.localeCompare(right.model) ||
        left.id.localeCompare(right.id),
    ),
    valuesByDay,
  };
}

function capCandidates(
  candidates: readonly SeriesCandidateSummary[],
  requestedLimit: number,
): {
  visible: SeriesCandidateSummary[];
  remainder: SeriesCandidateSummary[];
} {
  const limit = Math.max(1, Math.floor(finiteUsage(requestedLimit)) || 1);
  if (candidates.length <= limit) {
    return { visible: [...candidates], remainder: [] };
  }

  const keepIds = new Set(
    [...candidates]
      .sort(
        (left, right) =>
          right.total - left.total || left.id.localeCompare(right.id),
      )
      .slice(0, Math.max(0, limit - 1))
      .map(({ id }) => id),
  );
  return {
    visible: candidates.filter(({ id }) => keepIds.has(id)),
    remainder: candidates.filter(({ id }) => !keepIds.has(id)),
  };
}

function colorCandidates(
  candidates: readonly SeriesCandidateSummary[],
): Map<string, string> {
  const byProvider = new Map<UsageProviderId, SeriesCandidateSummary[]>();
  for (const candidate of candidates) {
    const values = byProvider.get(candidate.provider) ?? [];
    values.push(candidate);
    byProvider.set(candidate.provider, values);
  }

  const colors = new Map<string, string>();
  for (const [provider, providerCandidates] of byProvider) {
    const actualModels = providerCandidates.filter(
      ({ kind }) => kind === "model" || kind === "synthetic",
    );
    const rankedModels = [
      ...(actualModels.length > 0 ? actualModels : providerCandidates),
    ].sort(compareModelShadeRank);
    const rankedModelIds = new Set(rankedModels.map(({ id }) => id));
    const rankedRemainders = providerCandidates
      .filter(({ id }) => !rankedModelIds.has(id))
      .sort(compareModelShadeRank);
    const base = providerColor(provider);
    [...rankedModels, ...rankedRemainders].forEach((candidate, rank) => {
      colors.set(candidate.id, modelShade(base, rank));
    });
  }
  return colors;
}

function displayedValues(
  rawValues: readonly number[],
  view: UsageView,
  averageWindowDays: number,
): number[] {
  return view === "average"
    ? toTrailingAverage(rawValues, averageWindowDays)
    : [...rawValues];
}

/**
 * Build one lossless stack per provider+model. Stack order uses ascending raw
 * provider total, then ascending raw model total, so large series remain on
 * top and neither moving-average view nor tooltip sorting can reshuffle it.
 */
export function buildUsageChartData(
  days: readonly AggregatedUsageDay[],
  metric: UsageMetric,
  provider: UsageProviderFilter = ALL_USAGE_PROVIDERS,
  view: UsageView = "average",
  averageWindowDays = DEFAULT_USAGE_AVERAGE_WINDOW,
  maxSeries = MAX_USAGE_CHART_SERIES,
): UsageChartData {
  const sortedDays = [...days].sort((left, right) =>
    left.date.localeCompare(right.date),
  );
  const completeDays = fillMissingUsageDays(sortedDays);
  const normalizedWindow = Math.max(
    1,
    Math.floor(finiteUsage(averageWindowDays)) || 1,
  );
  const { candidates, valuesByDay } = candidatesForDays(
    completeDays,
    metric,
    provider,
  );
  const { visible, remainder } = capCandidates(candidates, maxSeries);
  const colors = colorCandidates(visible);
  const visibleCandidates: SeriesCandidate[] = visible.map((candidate) => ({
    ...candidate,
    rawValues: valuesByDay.map((dayValues) => dayValues.get(candidate.id) ?? 0),
  }));

  const series: UsageChartSeries[] = visibleCandidates.map((candidate) => ({
    id: candidate.id,
    label: modelLabel(candidate.provider, candidate.model, candidate.kind),
    providerLabel: providerLabel(candidate.provider),
    provider: candidate.provider,
    model: candidate.model,
    kind: candidate.kind,
    color: colors.get(candidate.id) ?? providerColor(candidate.provider),
    providers: [candidate.provider],
    rawValues: candidate.rawValues,
    values: displayedValues(candidate.rawValues, view, normalizedWindow),
    total: candidate.total,
  }));

  if (remainder.length > 0) {
    const remainderIds = new Set(remainder.map(({ id }) => id));
    const rawValues = valuesByDay.map((dayValues) => {
      let total = 0;
      for (const [id, value] of dayValues) {
        if (remainderIds.has(id)) total += value;
      }
      return total;
    });
    series.push({
      id: OTHER_USAGE_PROVIDERS,
      label: `Other (${remainder.length})`,
      providerLabel: "Other",
      provider: null,
      model: SERIES_REMAINDER_USAGE_MODEL,
      kind: "series-remainder",
      color: "#737373",
      providers: [...new Set(remainder.map(({ provider: id }) => id))].sort(),
      rawValues,
      values: displayedValues(rawValues, view, normalizedWindow),
      total: rawValues.reduce((sum, value) => sum + value, 0),
    });
  }

  const rawDailyTotals = completeDays.map((_, dayIndex) =>
    series.reduce((sum, item) => sum + item.rawValues[dayIndex], 0),
  );
  const dailyTotals = completeDays.map((_, dayIndex) =>
    series.reduce((sum, item) => sum + item.values[dayIndex], 0),
  );

  return {
    dates: completeDays.map(({ date }) => date),
    series,
    view,
    averageWindowDays: normalizedWindow,
    rawDailyTotals,
    dailyTotals,
    rawMaxDailyTotal: Math.max(0, ...rawDailyTotals),
    maxDailyTotal: Math.max(0, ...dailyTotals),
    total: rawDailyTotals.reduce((sum, value) => sum + value, 0),
  };
}

/** Positive active-day rows in visual descending order with stable ties. */
export function getActiveTooltipRows(
  series: readonly UsageChartSeries[],
  dayIndex: number,
): UsageTooltipRow[] {
  return series
    .map((item, index) => ({
      series: item,
      value: finiteUsage(item.values[dayIndex]),
      rawValue: finiteUsage(item.rawValues[dayIndex]),
      index,
    }))
    .filter(({ value }) => value > 0)
    .sort((left, right) => right.value - left.value || left.index - right.index)
    .map(({ series: item, value, rawValue }) => ({
      series: item,
      value,
      rawValue,
    }));
}
