import { describe, expect, it } from "vitest";
import { modelRef } from "@/lib/explore/modelMeta";
import type { Coverage, ModelUsageRow } from "@/lib/explore";
import { formatTotal, ledger } from "@/lib/capital/ledger";
import { buildHomeCapital } from "@/lib/capital/ledger/home";
import type { UsageSnapshot } from "@/lib/capital/ledger/usage";

const coverage: Coverage = {
  period: "30d",
  start: "2026-09-20",
  end: "2026-10-19",
  accounts: 250,
  modelIdentifiedShare: 1,
  lastDataAt: "2026-10-19T09:24:00.000Z",
};

function row(key: string, activeUsers: number, tokens: number): ModelUsageRow {
  return {
    ...modelRef(key),
    rank: 0,
    activeUsers,
    previousActiveUsers: null,
    adoption: activeUsers / coverage.accounts,
    tokens,
    estimatedCost: 0,
    isNew: false,
    topAgents: [],
  };
}

function snapshot(rows: ModelUsageRow[]): UsageSnapshot {
  return { rows, coverage, totalTokens: rows.reduce((sum, r) => sum + r.tokens, 0) };
}

const SNAPSHOT = snapshot([
  row("claude-opus-5-5", 165, 700),
  row("claude-sonnet-5-5", 68, 100),
  row("gpt-6.1-sol", 141, 150),
  row("gemini-3-pro", 20, 40),
  row("some-unknown-model", 5, 10),
]);

describe("home capital", () => {
  it("shows the four money types with the same numbers as /capital, never summed", () => {
    const home = buildHomeCapital({ snapshot: null });
    const year = ledger.reviewedAt().slice(0, 4);
    expect(home.stats.map((s) => s.label)).toEqual([
      `Equity raised · ${year} so far`,
      `Compute deals · ${year}`,
      expect.stringMatching(/^Cash capex · /),
      "Largest valuation",
    ]);
    const usd = ledger.equityYtd(year).find((t) => t.currency === "USD");
    expect(home.stats[0].value).toBe(usd ? formatTotal(usd) : "—");
    const valuation = ledger.largestValuation();
    expect(home.stats[3].value).toBe(valuation ? valuation.amount.short : "—");
    const capex = ledger.capexCalendarQuarter();
    expect(home.stats[2].value).toBe(capex ? formatTotal(capex.total) : "—");
  });

  it("ranks labs by Tokscale tokens and links each to its profile and top model", () => {
    const home = buildHomeCapital({ snapshot: SNAPSHOT });
    expect(home.labs.map((l) => l.slug)).toEqual(["anthropic", "openai", "alphabet"]);
    const anthropic = home.labs[0];
    expect(anthropic.share).toBeCloseTo(800 / 1000, 5);
    expect(anthropic.topModel?.key).toBe("claude-opus-5-5");
    expect(anthropic.href).toBe("/capital/companies/anthropic");
    expect(anthropic.round?.label).toBe("Series H");
    expect(anthropic.round?.valuation).toBe("$965B");
    expect(home.labsWindow).toEqual({ start: "2026-09-20", end: "2026-10-19", accounts: 250 });
  });

  it("omits the lab table without usage data", () => {
    const home = buildHomeCapital({ snapshot: null });
    expect(home.labs).toEqual([]);
    expect(home.labsWindow).toBeNull();
  });

  it("lists the four newest events with a source each", () => {
    const home = buildHomeCapital({ snapshot: null });
    const newest = ledger.getEvents().slice(0, 4);
    expect(home.events.map((e) => e.id)).toEqual(newest.map((e) => e.id));
    for (const e of home.events) {
      expect(e.href).toMatch(/^\/capital\/(rounds|deals)\//);
      expect(e.source?.url).toMatch(/^https?:\/\//);
    }
  });
});
