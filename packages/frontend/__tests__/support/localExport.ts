import type { ClientContribution, ClientType, DailyContribution, TokenBreakdown, TokenContributionData } from "../../src/lib/types";

/** One client and model line of a `tokscale graph` day. */
export function clientLine(
  client: ClientType,
  modelId: string,
  cost: number,
  tokens: Partial<TokenBreakdown>,
  messages = 1,
): ClientContribution {
  return {
    client,
    modelId,
    cost,
    messages,
    tokens: { input: 0, output: 0, cacheRead: 0, cacheWrite: 0, reasoning: 0, ...tokens },
  };
}

/** A day with totals summed from its client lines, as the CLI writes them. */
export function exportDay(date: string, clients: ClientContribution[], intensity: DailyContribution["intensity"] = 2): DailyContribution {
  const tokenBreakdown: TokenBreakdown = { input: 0, output: 0, cacheRead: 0, cacheWrite: 0, reasoning: 0 };
  for (const line of clients) {
    for (const key of Object.keys(tokenBreakdown) as (keyof TokenBreakdown)[]) {
      tokenBreakdown[key] += line.tokens[key];
    }
  }
  return {
    date,
    intensity,
    tokenBreakdown,
    clients,
    totals: {
      tokens: Object.values(tokenBreakdown).reduce((sum, value) => sum + value, 0),
      cost: clients.reduce((sum, line) => sum + line.cost, 0),
      messages: clients.reduce((sum, line) => sum + line.messages, 0),
    },
  };
}

/** A small two-year export in the shape `tokscale graph` prints. */
export function localExport(): TokenContributionData {
  const contributions = [
    exportDay("2025-12-30", [clientLine("claude", "claude-opus-4-5", 12.5, { input: 1_000, output: 4_000, cacheRead: 200_000 }, 40)], 1),
    exportDay(
      "2026-03-02",
      [
        clientLine("claude", "claude-sonnet-4-5", 30.25, { input: 2_000, output: 9_000, cacheRead: 1_500_000, cacheWrite: 80_000 }, 120),
        clientLine("codex", "gpt-5.1-codex", 8.75, { input: 50_000, output: 6_000, cacheRead: 400_000, reasoning: 3_000 }, 64),
      ],
      4,
    ),
    exportDay("2026-03-03", [clientLine("codex", "gpt-5.1-codex", 4.5, { input: 20_000, output: 2_000, cacheRead: 100_000 }, 21)], 2),
  ];
  const totalCost = contributions.reduce((sum, day) => sum + day.totals.cost, 0);
  const totalTokens = contributions.reduce((sum, day) => sum + day.totals.tokens, 0);
  return {
    meta: { generatedAt: "2026-03-04T00:00:00Z", version: "4.20.0", dateRange: { start: "2025-12-30", end: "2026-03-03" } },
    summary: {
      totalTokens,
      totalCost,
      totalDays: 3,
      activeDays: 3,
      averagePerDay: totalCost / 3,
      maxCostInSingleDay: 39,
      clients: ["claude", "codex"],
      models: ["claude-opus-4-5", "claude-sonnet-4-5", "gpt-5.1-codex"],
    },
    years: [
      { year: "2025", totalTokens: contributions[0].totals.tokens, totalCost: 12.5, range: { start: "2025-12-30", end: "2025-12-31" } },
      { year: "2026", totalTokens: totalTokens - contributions[0].totals.tokens, totalCost: totalCost - 12.5, range: { start: "2026-01-01", end: "2026-03-03" } },
    ],
    contributions,
  };
}
