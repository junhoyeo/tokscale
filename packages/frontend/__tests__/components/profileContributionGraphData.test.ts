import { describe, expect, it } from "vitest";
import {
  createContributionClientDetails,
  createContributionCalendar,
  getContributionDayMessageCount,
  getContributionColor,
  getContributionFocusDate,
  mergeDailyContributions,
} from "../../src/components/profile/ProfileContributionGraph";
import type { DailyContribution } from "../../src/lib/types";
import { colorPalettes } from "../../src/lib/themes";

function contribution(
  date: string,
  tokens: number,
  cost: number,
  intensity: 0 | 1 | 2 | 3 | 4 = 0,
): DailyContribution {
  return {
    date,
    totals: { tokens, cost, messages: 0 },
    intensity,
    tokenBreakdown: {
      input: tokens,
      output: 0,
      cacheRead: 0,
      cacheWrite: 0,
      reasoning: 0,
    },
    clients: [],
  };
}

function relativeLuminance(color: string): number {
  const channels = [1, 3, 5].map((offset) =>
    Number.parseInt(color.slice(offset, offset + 2), 16),
  );
  const linear = channels.map((channel) => {
    const normalized = channel / 255;
    return normalized <= 0.04045
      ? normalized / 12.92
      : ((normalized + 0.055) / 1.055) ** 2.4;
  });
  return linear[0] * 0.2126 + linear[1] * 0.7152 + linear[2] * 0.0722;
}

function contrastRatio(left: string, right: string): number {
  const luminances = [relativeLuminance(left), relativeLuminance(right)].sort(
    (a, b) => b - a,
  );
  return (luminances[0] + 0.05) / (luminances[1] + 0.05);
}

describe("profile contribution calendar", () => {
  it("derives intensity from tokens so free usage remains visible", () => {
    const calendar = createContributionCalendar([
      contribution("2026-07-05", 100, 0, 0),
      contribution("2026-07-06", 400, 10, 4),
    ]);

    const freeDay = calendar.cells.find(({ date }) => date === "2026-07-05");
    expect(freeDay).toMatchObject({ intensity: 2, tokens: 100 });
    expect(calendar.activeDays).toBe(2);
    expect(calendar.freeTokenDays).toBe(1);
    expect(calendar.highestDay?.date).toBe("2026-07-06");
  });

  it("renders explicit outer range days as zero-valued cells", () => {
    const calendar = createContributionCalendar(
      [contribution("2026-07-06", 200, 1)],
      "2026-07-05",
      "2026-07-11",
    );
    const scopedCells = calendar.cells.filter(({ inRange }) => inRange);

    expect(scopedCells).toHaveLength(7);
    expect(scopedCells[0]).toMatchObject({
      date: "2026-07-05",
      intensity: 0,
      tokens: 0,
    });
    expect(scopedCells.at(-1)).toMatchObject({
      date: "2026-07-11",
      intensity: 0,
      tokens: 0,
    });
    expect(calendar.startDate).toBe("2026-07-05");
    expect(calendar.endDate).toBe("2026-07-11");
    expect(calendar.activeDays).toBe(1);
  });

  it("merges duplicate dates before token intensity is calculated", () => {
    const calendar = createContributionCalendar([
      contribution("2026-07-05", 50, 0),
      contribution("2026-07-05", 50, 1),
      contribution("2026-07-06", 200, 2),
    ]);

    expect(
      calendar.cells.find(({ date }) => date === "2026-07-05"),
    ).toMatchObject({
      intensity: 3,
      tokens: 100,
    });
    expect(calendar.activeDays).toBe(2);
  });

  it("merges duplicate day detail without dropping token or client data", () => {
    const first = contribution("2026-07-05", 50, 1);
    first.totals.messages = 2;
    first.clients = [
      {
        client: "codex",
        cost: 1,
        messages: 2,
        modelId: "gpt-5.4",
        providerId: "openai",
        tokens: first.tokenBreakdown,
      },
    ];
    const second = contribution("2026-07-05", 75, 2);
    second.totals.messages = 3;
    second.clients = [
      {
        client: "claude",
        cost: 2,
        messages: 3,
        modelId: "claude-opus-4-7",
        providerId: "anthropic",
        tokens: second.tokenBreakdown,
      },
    ];

    const merged = mergeDailyContributions([first, second]).get("2026-07-05");

    expect(merged?.totals).toEqual({ cost: 3, messages: 5, tokens: 125 });
    expect(merged?.tokenBreakdown.input).toBe(125);
    expect(merged?.clients.map(({ client }) => client)).toEqual([
      "codex",
      "claude",
    ]);
  });

  it("builds sorted client and model detail for flat and nested API formats", () => {
    const day = contribution("2026-07-05", 300, 12);
    day.clients = [
      {
        client: "codex",
        cost: 2,
        messages: 2,
        modelId: "gpt-5.4",
        providerId: "openai",
        tokens: {
          cacheRead: 0,
          cacheWrite: 0,
          input: 40,
          output: 10,
          reasoning: 0,
        },
      },
      {
        client: "claude",
        cost: 10,
        messages: 4,
        modelId: "",
        providerId: "anthropic",
        tokens: {
          cacheRead: 140,
          cacheWrite: 0,
          input: 80,
          output: 30,
          reasoning: 0,
        },
        models: {
          "claude-opus-4-7": {
            cacheRead: 140,
            cacheWrite: 0,
            cost: 10,
            input: 80,
            messages: 4,
            output: 30,
            reasoning: 0,
            tokens: 250,
          },
        },
      },
    ];

    const details = createContributionClientDetails(day);

    expect(details.map(({ client }) => client)).toEqual(["claude", "codex"]);
    expect(details[0]).toMatchObject({
      cost: 10,
      messages: 4,
      totalTokens: 250,
    });
    expect(details[0].models[0]).toMatchObject({
      modelId: "claude-opus-4-7",
      providerId: "anthropic",
      totalTokens: 250,
    });
    expect(details[1].models[0]).toMatchObject({
      modelId: "gpt-5.4",
      providerId: "openai",
      totalTokens: 50,
    });
  });

  it("falls back to nested model messages when the daily summary omits them", () => {
    const day = contribution("2026-07-05", 300, 12);
    day.clients = [
      {
        client: "claude",
        cost: 12,
        messages: 0,
        modelId: "",
        providerId: "anthropic",
        tokens: day.tokenBreakdown,
        models: {
          "claude-fable-5": {
            cacheRead: 200,
            cacheWrite: 0,
            cost: 8,
            input: 60,
            messages: 703,
            output: 20,
            reasoning: 0,
            tokens: 280,
          },
          "claude-opus-4-8": {
            cacheRead: 10,
            cacheWrite: 0,
            cost: 4,
            input: 8,
            messages: 46,
            output: 2,
            reasoning: 0,
            tokens: 20,
          },
        },
      },
    ];

    const details = createContributionClientDetails(day);

    expect(details[0].messages).toBe(749);
    expect(getContributionDayMessageCount(day, details)).toBe(749);

    day.totals.messages = 11;
    expect(getContributionDayMessageCount(day, details)).toBe(11);
  });

  it("moves one roving contribution focus by day, week, and boundary", () => {
    const calendar = createContributionCalendar(
      [contribution("2026-07-06", 200, 1)],
      "2026-07-05",
      "2026-07-18",
    );

    expect(
      getContributionFocusDate(calendar.cells, "2026-07-11", "ArrowRight"),
    ).toBe("2026-07-12");
    expect(
      getContributionFocusDate(calendar.cells, "2026-07-11", "ArrowDown"),
    ).toBe("2026-07-18");
    expect(
      getContributionFocusDate(calendar.cells, "2026-07-11", "ArrowUp"),
    ).toBe("2026-07-05");
    expect(getContributionFocusDate(calendar.cells, "2026-07-11", "Home")).toBe(
      "2026-07-05",
    );
    expect(getContributionFocusDate(calendar.cells, "2026-07-11", "End")).toBe(
      "2026-07-18",
    );
    expect(
      getContributionFocusDate(calendar.cells, "2026-07-05", "ArrowLeft"),
    ).toBe("2026-07-05");
  });

  it("keeps every active palette level distinct from the dark empty cell", () => {
    for (const palette of Object.values(colorPalettes)) {
      for (const level of [1, 2, 3, 4] as const) {
        expect(
          contrastRatio(getContributionColor(palette, level), "#191f2b"),
        ).toBeGreaterThanOrEqual(3);
      }
    }
  });
});
