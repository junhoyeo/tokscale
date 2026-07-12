import { describe, expect, it } from "vitest";
import type {
  ClientContribution,
  ClientType,
  DailyContribution,
  ModelBreakdownData,
  TokenBreakdown,
} from "../../src/lib/types";
import {
  BLANK_USAGE_MODEL,
  DAILY_REMAINDER_USAGE_MODEL,
  OTHER_USAGE_PROVIDERS,
  PROVIDER_REMAINDER_USAGE_MODEL,
  UNATTRIBUTED_USAGE_PROVIDER,
  aggregateDailyUsage,
  buildUsageChartData,
  fillMissingUsageDays,
  getActiveTooltipRows,
  sumTokenBreakdown,
  toTrailingAverage,
} from "../../src/components/profile/usageChartData";

const EMPTY_TOKENS: TokenBreakdown = {
  input: 0,
  output: 0,
  cacheRead: 0,
  cacheWrite: 0,
  reasoning: 0,
};

function client(
  provider: ClientType,
  tokens: Partial<TokenBreakdown>,
  cost = 0,
  modelId = "model",
): ClientContribution {
  return {
    client: provider,
    modelId,
    tokens: { ...EMPTY_TOKENS, ...tokens },
    cost,
    messages: 1,
  };
}

function model(
  tokens: Partial<TokenBreakdown>,
  cost = 0,
  reportedTokens?: number,
): ModelBreakdownData {
  const breakdown = { ...EMPTY_TOKENS, ...tokens };
  return {
    tokens: reportedTokens ?? sumTokenBreakdown(breakdown),
    cost,
    ...breakdown,
    messages: 1,
  };
}

function nestedClient(
  provider: ClientType,
  models: Record<string, ModelBreakdownData>,
  tokens: Partial<TokenBreakdown>,
  cost: number,
): ClientContribution {
  return {
    ...client(provider, tokens, cost, ""),
    models,
  };
}

function day(date: string, clients: ClientContribution[]): DailyContribution {
  const tokenBreakdown = clients.reduce<TokenBreakdown>(
    (total, item) => ({
      input: total.input + item.tokens.input,
      output: total.output + item.tokens.output,
      cacheRead: total.cacheRead + item.tokens.cacheRead,
      cacheWrite: total.cacheWrite + item.tokens.cacheWrite,
      reasoning: total.reasoning + item.tokens.reasoning,
    }),
    { ...EMPTY_TOKENS },
  );

  return {
    date,
    totals: {
      tokens: sumTokenBreakdown(tokenBreakdown),
      cost: clients.reduce((total, item) => total + item.cost, 0),
      messages: clients.length,
    },
    intensity: clients.length > 0 ? 1 : 0,
    tokenBreakdown,
    clients,
  };
}

describe("profile usage chart aggregation", () => {
  it("includes cache and reasoning usage and rejects non-finite components", () => {
    expect(
      sumTokenBreakdown({
        input: 100,
        output: 50,
        cacheRead: 25,
        cacheWrite: 10,
        reasoning: 15,
      }),
    ).toBe(200);
    expect(
      sumTokenBreakdown({
        input: Number.POSITIVE_INFINITY,
        output: Number.NaN,
        cacheRead: -20,
        cacheWrite: 4,
        reasoning: 6,
      }),
    ).toBe(10);
  });

  it("sorts dates, merges duplicate flat rows, and preserves input order", () => {
    const contributions = [
      day("2026-03-03", [client("claude", { input: 3 }, 3, "opus")]),
      day("2026-03-01", [client("claude", { input: 1 }, 1, "opus")]),
      day("2026-03-01", [client("claude", { output: 2 }, 2, "opus")]),
    ];

    const result = aggregateDailyUsage(contributions);

    expect(result.map(({ date }) => date)).toEqual([
      "2026-03-01",
      "2026-03-02",
      "2026-03-03",
    ]);
    expect(result[0].providers[0]).toMatchObject({
      provider: "claude",
      tokens: 3,
      cost: 3,
      models: [{ model: "opus", tokens: 3, cost: 3 }],
    });
    expect(contributions.map(({ date }) => date)).toEqual([
      "2026-03-03",
      "2026-03-01",
      "2026-03-01",
    ]);
  });

  it("keeps equal model ids from different providers as unique series", () => {
    const aggregated = aggregateDailyUsage([
      day("2026-04-10", [
        client("codex", { input: 10 }, 1, "shared-model"),
        client("claude", { input: 20 }, 2, "shared-model"),
      ]),
    ]);
    const chart = buildUsageChartData(aggregated, "tokens", "all", "daily");

    expect(chart.series).toHaveLength(2);
    expect(new Set(chart.series.map(({ id }) => id)).size).toBe(2);
    expect(chart.series.map(({ model: modelId }) => modelId)).toEqual([
      "shared-model",
      "shared-model",
    ]);
    expect(chart.series.map(({ provider }) => provider)).toEqual([
      "codex",
      "claude",
    ]);
    expect(chart.total).toBe(30);
  });

  it("expands nested database models and retains blank, synthetic, and provider remainders", () => {
    const nested = nestedClient(
      "claude",
      {
        opus: model({ input: 40, cacheRead: 20 }, 6, 45),
        "": model({ input: 20 }, 2),
        "<synthetic>": model({ input: 5 }, 0.5),
      },
      { input: 85, cacheRead: 15 },
      10,
    );
    const aggregated = aggregateDailyUsage([day("2026-04-11", [nested])]);
    const provider = aggregated[0].providers[0];

    expect(provider.tokens).toBe(100);
    expect(provider.cost).toBe(10);
    expect(provider.models).toEqual(
      expect.arrayContaining([
        expect.objectContaining({
          model: "opus",
          kind: "model",
          tokens: 60,
        }),
        expect.objectContaining({
          model: BLANK_USAGE_MODEL,
          kind: "blank-model",
          tokens: 20,
        }),
        expect.objectContaining({
          model: "<synthetic>",
          kind: "synthetic",
          tokens: 5,
        }),
        expect.objectContaining({
          model: PROVIDER_REMAINDER_USAGE_MODEL,
          kind: "provider-remainder",
          tokens: 15,
          cost: 1.5,
        }),
      ]),
    );
  });

  it("preserves legacy daily totals without client attribution", () => {
    const legacyDay = day("2026-07-01", []);
    legacyDay.totals.tokens = 1_234;
    legacyDay.totals.cost = 4.5;

    const aggregated = aggregateDailyUsage([legacyDay]);
    const chart = buildUsageChartData(aggregated, "tokens", "all", "daily");

    expect(aggregated[0].providers[0]).toMatchObject({
      provider: UNATTRIBUTED_USAGE_PROVIDER,
      tokens: 1_234,
      cost: 4.5,
      models: [
        {
          model: DAILY_REMAINDER_USAGE_MODEL,
          kind: "daily-remainder",
          tokens: 1_234,
          cost: 4.5,
        },
      ],
    });
    expect(chart.series[0]).toMatchObject({
      provider: UNATTRIBUTED_USAGE_PROVIDER,
      label: "Unattributed usage",
      rawValues: [1_234],
      values: [1_234],
      total: 1_234,
    });
    expect(chart.rawDailyTotals).toEqual([1_234]);
  });

  it("scales over-attributed nested tokens and cost to exact client totals", () => {
    const contribution = day("2026-04-12", [
      nestedClient(
        "claude",
        {
          opus: model({ input: 80 }, 8),
          sonnet: model({ input: 40 }, 4),
        },
        { input: 100 },
        10,
      ),
    ]);

    const aggregated = aggregateDailyUsage([contribution]);
    const provider = aggregated[0].providers[0];
    const tokenChart = buildUsageChartData(
      aggregated,
      "tokens",
      "all",
      "daily",
    );
    const costChart = buildUsageChartData(aggregated, "cost", "all", "daily");

    expect(provider.tokens).toBeCloseTo(100, 12);
    expect(provider.cost).toBeCloseTo(10, 12);
    expect(
      provider.models.every(({ tokens, cost }) => tokens >= 0 && cost >= 0),
    ).toBe(true);
    expect(tokenChart.rawDailyTotals[0]).toBeCloseTo(100, 12);
    expect(tokenChart.total).toBeCloseTo(100, 12);
    expect(costChart.rawDailyTotals[0]).toBeCloseTo(10, 12);
    expect(costChart.total).toBeCloseTo(10, 12);
  });

  it("adds under-attribution remainders and reconciles authoritative day totals", () => {
    const underAttributed = day("2026-04-13", [
      nestedClient(
        "claude",
        {
          opus: model({ input: 40 }, 4),
          sonnet: model({ input: 20 }, 2),
        },
        { input: 100 },
        10,
      ),
    ]);
    const dayOverAttributed = day("2026-04-14", [
      client("codex", { input: 100 }, 10, "gpt"),
    ]);
    dayOverAttributed.totals.tokens = 75;
    dayOverAttributed.totals.cost = 7.5;
    const dayUnderAttributed = day("2026-04-15", [
      client("codex", { input: 100 }, 10, "gpt"),
    ]);
    dayUnderAttributed.totals.tokens = 125;
    dayUnderAttributed.totals.cost = 12.5;

    const aggregated = aggregateDailyUsage([
      underAttributed,
      dayOverAttributed,
      dayUnderAttributed,
    ]);
    const providerRemainder = aggregated[0].providers[0].models.find(
      ({ model: modelId }) => modelId === PROVIDER_REMAINDER_USAGE_MODEL,
    );
    const dailyRemainder = aggregated[2].providers
      .find(({ provider }) => provider === UNATTRIBUTED_USAGE_PROVIDER)
      ?.models.find(
        ({ model: modelId }) => modelId === DAILY_REMAINDER_USAGE_MODEL,
      );
    const tokenChart = buildUsageChartData(
      aggregated,
      "tokens",
      "all",
      "daily",
    );
    const costChart = buildUsageChartData(aggregated, "cost", "all", "daily");

    expect(providerRemainder).toMatchObject({ tokens: 40, cost: 4 });
    expect(dailyRemainder).toMatchObject({ tokens: 25, cost: 2.5 });
    expect(tokenChart.rawDailyTotals).toEqual([100, 75, 125]);
    expect(costChart.rawDailyTotals).toEqual([10, 7.5, 12.5]);
    expect(tokenChart.total).toBe(300);
    expect(costChart.total).toBe(30);
    expect(
      aggregated.flatMap(({ providers }) =>
        providers.flatMap(({ models }) =>
          models.flatMap(({ tokens, cost }) => [tokens, cost]),
        ),
      ),
    ).toSatisfy((values: number[]) => values.every((value) => value >= 0));
  });

  it("uses ascending provider and model totals for a stable stack", () => {
    const aggregated = aggregateDailyUsage([
      day("2026-05-01", [
        client("claude", { input: 70 }, 7, "large"),
        client("claude", { input: 30 }, 3, "small"),
        client("gemini", { input: 50 }, 5, "only"),
      ]),
    ]);
    const chart = buildUsageChartData(aggregated, "tokens", "all", "daily");

    expect(
      chart.series.map(({ provider, model: modelId }) => [provider, modelId]),
    ).toEqual([
      ["gemini", "only"],
      ["claude", "small"],
      ["claude", "large"],
    ]);
    expect(
      chart.series.find(({ model: modelId }) => modelId === "large")?.color,
    ).toBe("#f97316");
    expect(
      chart.series.find(({ model: modelId }) => modelId === "small")?.color,
    ).not.toBe("#f97316");
  });

  it("ranks source shades by the TUI family hierarchy without changing stack or tooltip order", () => {
    const chart = buildUsageChartData(
      aggregateDailyUsage([
        day("2026-06-17", [
          client("claude", { input: 10 }, 1, "claude-fable-5"),
          client("claude", { input: 300 }, 30, "claude-opus-4-8"),
          client("claude", { input: 5 }, 50, "claude-opus-4-7"),
          client("claude", { input: 1 }, 200, "claude-sonnet-5"),
          client("claude", { input: 400 }, 400, "claude-haiku-6"),
        ]),
      ]),
      "tokens",
      "all",
      "daily",
    );
    const colorByModel = Object.fromEntries(
      chart.series.map(({ model: modelId, color }) => [modelId, color]),
    );

    // Same hierarchy and seven-step white interpolation as the TUI: family,
    // version, cost, then name. Fable stays the base Claude Code color even
    // when every lower-tier family has more usage or cost.
    expect(colorByModel).toMatchObject({
      "claude-fable-5": "#f97316",
      "claude-opus-4-8": "#fa8230",
      "claude-opus-4-7": "#fa9249",
      "claude-sonnet-5": "#fba163",
      "claude-haiku-6": "#fcb17d",
    });
    expect(chart.series.map(({ model: modelId }) => modelId)).toEqual([
      "claude-sonnet-5",
      "claude-opus-4-7",
      "claude-fable-5",
      "claude-opus-4-8",
      "claude-haiku-6",
    ]);
    expect(
      getActiveTooltipRows(chart.series, 0).map(({ series }) => series.model),
    ).toEqual([
      "claude-haiku-6",
      "claude-opus-4-8",
      "claude-fable-5",
      "claude-opus-4-7",
      "claude-sonnet-5",
    ]);
  });

  it("does not classify model-name substrings as model families", () => {
    const chart = buildUsageChartData(
      aggregateDailyUsage([
        day("2026-07-01", [
          client("claude", { input: 1 }, 1, "claude-opus-4"),
          client("claude", { input: 1 }, 1, "claude-myopus-99"),
        ]),
      ]),
      "tokens",
      "all",
      "daily",
    );

    const colors = Object.fromEntries(
      chart.series.map(({ model, color }) => [model, color]),
    );
    expect(colors["claude-opus-4"]).toBe("#f97316");
    expect(colors["claude-myopus-99"]).toBe("#fa8230");
  });

  it("assigns duplicate model ids deterministic shades inside each source", () => {
    const forward = [
      client("claude", { input: 10 }, 1, "claude-fable-5"),
      client("claude", { input: 100 }, 10, "claude-opus-4-8"),
      client("opencode", { input: 20 }, 2, "claude-fable-5"),
      client("opencode", { input: 200 }, 20, "claude-opus-4-8"),
    ];
    const buildColors = (clients: ClientContribution[]) =>
      Object.fromEntries(
        buildUsageChartData(
          aggregateDailyUsage([day("2026-06-17", clients)]),
          "tokens",
          "all",
          "daily",
        ).series.map(({ id, color }) => [id, color]),
      );

    const colors = buildColors(forward);
    expect(buildColors([...forward].reverse())).toEqual(colors);
    expect(colors["claude::claude-fable-5"]).toBe("#f97316");
    expect(colors["claude::claude-opus-4-8"]).toBe("#fa8230");
    expect(colors["opencode::claude-fable-5"]).toBe("#00a8e8");
    expect(colors["opencode::claude-opus-4-8"]).toBe("#1cb2eb");
  });

  it("lifts near-black provider colors for the dark chart canvas", () => {
    const chart = buildUsageChartData(
      aggregateDailyUsage([
        day("2026-05-02", [client("droid", { input: 10 }, 1, "droid-model")]),
      ]),
      "tokens",
      "all",
      "daily",
    );

    expect(chart.series[0].color).toMatch(/^#[\da-f]{6}$/i);
    expect(chart.series[0].color.toLowerCase()).not.toBe("#1f1d1c");
  });

  it("inserts zero UTC days before applying a trailing average with partial leading divisors", () => {
    const aggregated = aggregateDailyUsage([
      day("2026-06-01", [client("codex", { input: 30 }, 3, "gpt")]),
      day("2026-06-03", [client("codex", { input: 90 }, 9, "gpt")]),
    ]);
    const daily = buildUsageChartData(aggregated, "tokens", "all", "daily", 3);
    const average = buildUsageChartData(
      aggregated,
      "tokens",
      "all",
      "average",
      3,
    );

    expect(daily.dates).toEqual(["2026-06-01", "2026-06-02", "2026-06-03"]);
    expect(daily.dailyTotals).toEqual([30, 0, 90]);
    expect(average.dailyTotals).toEqual([30, 15, 40]);
    expect(toTrailingAverage([30, 0, 90, 60], 3)).toEqual([30, 15, 40, 50]);
    expect(average.total).toBe(120);
    expect(daily.total).toBe(120);
    expect(average.series[0].total).toBe(daily.series[0].total);
    expect(average.rawDailyTotals).toEqual(daily.rawDailyTotals);
  });

  it("fills explicit week boundaries and excludes observations outside them", () => {
    const aggregated = aggregateDailyUsage(
      [
        day("2026-06-07", [client("codex", { input: 99 }, 9.9, "old")]),
        day("2026-06-09", [client("codex", { input: 10 }, 1, "gpt")]),
        day("2026-06-12", [client("codex", { input: 40 }, 4, "gpt")]),
        day("2026-06-15", [client("codex", { input: 99 }, 9.9, "new")]),
      ],
      "2026-06-08",
      "2026-06-14",
    );
    const chart = buildUsageChartData(aggregated, "tokens", "all", "daily");

    expect(chart.dates).toEqual([
      "2026-06-08",
      "2026-06-09",
      "2026-06-10",
      "2026-06-11",
      "2026-06-12",
      "2026-06-13",
      "2026-06-14",
    ]);
    expect(chart.rawDailyTotals).toEqual([0, 10, 0, 0, 40, 0, 0]);
    expect(chart.total).toBe(50);
  });

  it("fills a seven-day range around one observed day", () => {
    const observed = aggregateDailyUsage([
      day("2026-06-18", [client("gemini", { input: 70 }, 7, "flash")]),
    ]);
    const completed = fillMissingUsageDays(
      observed,
      "2026-06-15",
      "2026-06-21",
    );

    expect(completed.map(({ date }) => date)).toEqual([
      "2026-06-15",
      "2026-06-16",
      "2026-06-17",
      "2026-06-18",
      "2026-06-19",
      "2026-06-20",
      "2026-06-21",
    ]);
    expect(completed.map(({ providers }) => providers.length)).toEqual([
      0, 0, 0, 1, 0, 0, 0,
    ]);
  });

  it("filters a provider while retaining zeroes on inactive dates", () => {
    const aggregated = aggregateDailyUsage([
      day("2026-06-01", [
        client("claude", { input: 10 }, 1, "opus"),
        client("gemini", { input: 20 }, 2, "flash"),
      ]),
      day("2026-06-02", [client("claude", { input: 30 }, 3, "opus")]),
      day("2026-06-03", [client("gemini", { input: 40 }, 4, "flash")]),
    ]);
    const chart = buildUsageChartData(aggregated, "cost", "gemini", "daily");

    expect(chart.series).toHaveLength(1);
    expect(chart.series[0]).toMatchObject({
      provider: "gemini",
      model: "flash",
      rawValues: [2, 0, 4],
      values: [2, 0, 4],
      total: 6,
    });
    expect(chart.dailyTotals).toEqual([2, 0, 4]);
    expect(chart.total).toBe(6);
  });

  it("caps pathological model counts with one lossless remainder series", () => {
    const models = Object.fromEntries(
      Array.from({ length: 140 }, (_, index) => [
        `model-${index.toString().padStart(3, "0")}`,
        model({ input: index + 1 }, (index + 1) / 10),
      ]),
    );
    const total = (140 * 141) / 2;
    const aggregated = aggregateDailyUsage([
      day("2026-07-02", [
        nestedClient("codex", models, { input: total }, total / 10),
      ]),
    ]);
    const chart = buildUsageChartData(aggregated, "tokens", "all", "daily");
    const remainder = chart.series.find(
      ({ id }) => id === OTHER_USAGE_PROVIDERS,
    );

    expect(chart.series).toHaveLength(128);
    expect(remainder).toMatchObject({
      kind: "series-remainder",
      label: "Other (13)",
    });
    expect(chart.total).toBe(total);
    expect(chart.rawDailyTotals).toEqual([total]);
    expect(chart.series.reduce((sum, series) => sum + series.total, 0)).toBe(
      total,
    );
    expect(
      chart.series.flatMap(({ values, rawValues }) => [
        ...values,
        ...rawValues,
      ]),
    ).toSatisfy((values: number[]) =>
      values.every((value) => Number.isFinite(value) && value >= 0),
    );
  });

  it("keeps exact raw totals across daily, average, provider, and capped views", () => {
    const aggregated = aggregateDailyUsage(
      [
        day("2026-07-05", [
          client("codex", { input: 30 }, 3, "gpt-a"),
          client("codex", { input: 20 }, 2, "gpt-b"),
          client("claude", { input: 10 }, 1, "opus"),
        ]),
        day("2026-07-07", [
          client("codex", { input: 5 }, 0.5, "gpt-a"),
          client("claude", { input: 15 }, 1.5, "sonnet"),
        ]),
      ],
      "2026-07-05",
      "2026-07-07",
    );
    const daily = buildUsageChartData(aggregated, "tokens", "all", "daily");
    const average = buildUsageChartData(
      aggregated,
      "tokens",
      "all",
      "average",
      2,
    );
    const filtered = buildUsageChartData(
      aggregated,
      "tokens",
      "codex",
      "daily",
    );
    const capped = buildUsageChartData(
      aggregated,
      "tokens",
      "all",
      "daily",
      30,
      2,
    );

    expect(daily.rawDailyTotals).toEqual([60, 0, 20]);
    expect(daily.total).toBe(80);
    expect(average.rawDailyTotals).toEqual([60, 0, 20]);
    expect(average.total).toBe(80);
    expect(filtered.rawDailyTotals).toEqual([50, 0, 5]);
    expect(filtered.total).toBe(55);
    expect(capped.series).toHaveLength(2);
    expect(capped.rawDailyTotals).toEqual([60, 0, 20]);
    expect(capped.total).toBe(80);
    expect(
      capped.rawDailyTotals.map((_, dayIndex) =>
        capped.series.reduce(
          (sum, series) => sum + series.rawValues[dayIndex],
          0,
        ),
      ),
    ).toEqual(capped.rawDailyTotals);
  });

  it("sorts positive tooltip rows descending while preserving stack-order ties", () => {
    const aggregated = aggregateDailyUsage([
      day("2026-07-03", [
        client("codex", { input: 20 }, 2, "b"),
        client("codex", { input: 10 }, 1, "a"),
        client("gemini", { input: 20 }, 2, "c"),
        client("claude", { input: 0 }, 0, "zero"),
      ]),
    ]);
    const chart = buildUsageChartData(aggregated, "tokens", "all", "daily");
    const stackOrder = chart.series
      .filter(({ values }) => values[0] === 20)
      .map(({ id }) => id);
    const rows = getActiveTooltipRows(chart.series, 0);

    expect(rows.map(({ value }) => value)).toEqual([20, 20, 10]);
    expect(rows.slice(0, 2).map(({ series }) => series.id)).toEqual(stackOrder);
    expect(rows.every(({ value }) => value > 0)).toBe(true);
  });

  it("returns finite chart geometry inputs for malformed numeric payloads", () => {
    const malformed = day("2026-07-04", [
      client(
        "codex",
        {
          input: Number.NaN,
          output: Number.POSITIVE_INFINITY,
          cacheRead: -100,
          reasoning: 5,
        },
        Number.NaN,
        "gpt",
      ),
    ]);
    malformed.totals.tokens = Number.POSITIVE_INFINITY;
    malformed.totals.cost = Number.NaN;
    const chart = buildUsageChartData(
      aggregateDailyUsage([malformed]),
      "tokens",
      "all",
      "average",
    );

    expect(chart.total).toBe(5);
    expect(chart.dailyTotals).toEqual([5]);
    expect(chart.maxDailyTotal).toBe(5);
    expect(
      chart.series.flatMap(({ values, rawValues }) => [
        ...values,
        ...rawValues,
      ]),
    ).toSatisfy((values: number[]) => values.every(Number.isFinite));
  });

  it("leaves invalid or excessively broad date ranges unchanged", () => {
    expect(
      fillMissingUsageDays([
        { date: "not-a-date", providers: [] },
        { date: "still-invalid", providers: [] },
      ]),
    ).toHaveLength(2);
    expect(
      fillMissingUsageDays([
        { date: "2025-01-01", providers: [] },
        { date: "2026-12-31", providers: [] },
      ]),
    ).toHaveLength(2);
    const observed = [{ date: "2026-07-08", providers: [] }];
    expect(fillMissingUsageDays(observed, "invalid", "2026-07-14")).toEqual(
      observed,
    );
    expect(fillMissingUsageDays(observed, "2026-07-14", "2026-07-08")).toEqual(
      observed,
    );
    expect(fillMissingUsageDays(observed, "2025-01-01", "2026-12-31")).toEqual(
      observed,
    );
  });
});
