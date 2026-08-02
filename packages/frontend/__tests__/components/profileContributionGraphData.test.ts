import { describe, expect, it } from "vitest";
import {
  createContributionRangeOptions,
  createContributionIsometricGeometry,
  createContributionClientDetails,
  createContributionCalendar,
  getContributionDayForDate,
  getDefaultContributionDate,
  getContributionDayMessageCount,
  getContributionColor,
  getContributionFocusDate,
  getContributionScrollOffset,
  isContributionDateHit,
  getNearestContributionDate,
  mergeDailyContributions,
  PROFILE_CONTRIBUTION_CELL_RADIUS,
  PROFILE_CONTRIBUTION_CELL_SIZE,
  reconcileContributionSelectionRange,
  resolveContributionRange,
  resolveContributionSelectedDate,
} from "../../src/components/profile/ProfileContributionGraph";
import { BOX_BORDER_RADIUS, BOX_WIDTH } from "../../src/lib/constants";
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
  it("offers a rolling year followed by complete calendar years", () => {
    const options = createContributionRangeOptions(
      [
        contribution("2024-12-31", 10, 1),
        contribution("2025-01-01", 20, 2),
        contribution("2026-07-12", 30, 3),
        contribution("not-a-date", 40, 4),
      ],
      "2025-07-12",
      "2026-07-12",
    );

    expect(options).toEqual([
      {
        value: "recent",
        label: "Recent year",
        startDate: "2025-07-12",
        endDate: "2026-07-12",
      },
      {
        value: "2026",
        label: "2026",
        startDate: "2026-01-01",
        endDate: "2026-12-31",
      },
      {
        value: "2025",
        label: "2025",
        startDate: "2025-01-01",
        endDate: "2025-12-31",
      },
      {
        value: "2024",
        label: "2024",
        startDate: "2024-01-01",
        endDate: "2024-12-31",
      },
    ]);
  });

  it("falls back to the rolling year when a stale range value is requested", () => {
    const options = createContributionRangeOptions(
      [contribution("2026-07-12", 30, 3)],
      "2025-07-12",
      "2026-07-12",
    );

    expect(resolveContributionRange(options, "2026")).toMatchObject({
      value: "2026",
      startDate: "2026-01-01",
      endDate: "2026-12-31",
    });
    expect(resolveContributionRange(options, "2024")).toMatchObject({
      value: "recent",
      startDate: "2025-07-12",
      endDate: "2026-07-12",
    });
  });

  it("does not offer future years accepted through submission clock skew", () => {
    const options = createContributionRangeOptions(
      [
        contribution("2024-12-31", 10, 1),
        contribution("2026-01-01", 20, 2),
      ],
      "2025-01-01",
      "2025-12-31",
    );

    expect(options.map(({ value }) => value)).toEqual([
      "recent",
      "2025",
      "2024",
    ]);
  });

  it("renders a complete current year while keeping future dates inert", () => {
    const calendar = createContributionCalendar(
      [
        contribution("2026-07-12", 30, 3),
        contribution("2026-07-13", 3_000, 0),
      ],
      "2026-01-01",
      "2026-12-31",
      "2026-07-12",
    );

    expect(calendar.startDate).toBe("2026-01-01");
    expect(calendar.endDate).toBe("2026-12-31");
    expect(calendar.selectableEndDate).toBe("2026-07-12");
    expect(
      calendar.cells.find(({ date }) => date === "2026-07-12"),
    ).toMatchObject({
      inRange: true,
      intensity: 4,
      selectable: true,
      tokens: 30,
    });
    expect(
      calendar.cells.find(({ date }) => date === "2026-07-13"),
    ).toMatchObject({
      inRange: true,
      intensity: 0,
      selectable: false,
      tokens: 0,
    });
    expect(calendar.activeDays).toBe(1);
    expect(calendar.freeTokenDays).toBe(0);
    expect(getContributionFocusDate(calendar.cells, "2026-07-12", "End")).toBe(
      "2026-07-12",
    );
    expect(
      getDefaultContributionDate(
        [contribution("2026-07-12", 30, 3)],
        "2026-01-01",
        "2026-12-31",
        "2026-07-12",
      ),
    ).toBe("2026-07-12");
  });

  it("uses rounded contribution cells smaller than the main graph", () => {
    expect(PROFILE_CONTRIBUTION_CELL_SIZE).toBeLessThan(BOX_WIDTH);
    expect(PROFILE_CONTRIBUTION_CELL_RADIUS).toBeGreaterThan(0);
    expect(
      PROFILE_CONTRIBUTION_CELL_RADIUS / PROFILE_CONTRIBUTION_CELL_SIZE,
    ).toBeCloseTo(BOX_BORDER_RADIUS / BOX_WIDTH, 5);
  });

  it("reveals the controlled selected date inside a horizontally scrolling calendar", () => {
    expect(getContributionScrollOffset(0, 20, 360, 570, 578)).toBe(218);
    expect(getContributionScrollOffset(218, 20, 360, -4, 4)).toBe(194);
    expect(getContributionScrollOffset(218, 20, 360, 120, 128)).toBe(218);
  });

  it("does not turn a marked inert cell into a nearest-date selection", () => {
    const futureCell = {
      closest: (selector: string) =>
        selector === "[data-contribution-date]" ? {} : null,
    } as unknown as Element;

    expect(isContributionDateHit(futureCell)).toBe(true);
    expect(isContributionDateHit(null)).toBe(false);
  });

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

  it("treats floating-point cost residue as free usage", () => {
    const calendar = createContributionCalendar([
      contribution("2026-07-05", 100, 1e-7),
      contribution("2026-07-06", 100, 1e-5),
    ]);

    expect(calendar.freeTokenDays).toBe(1);
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

  it("reconciles stale nested model totals with their token breakdown", () => {
    const day = contribution("2026-07-05", 250, 10);
    day.clients = [
      {
        client: "claude",
        cost: 10,
        messages: 4,
        modelId: "",
        providerId: "anthropic",
        tokens: day.tokenBreakdown,
        models: {
          "claude-opus-4-7": {
            cacheRead: 140,
            cacheWrite: 0,
            cost: 10,
            input: 80,
            messages: 4,
            output: 30,
            reasoning: 0,
            tokens: 50,
          },
        },
      },
    ];

    const details = createContributionClientDetails(day);

    expect(details[0].models[0].totalTokens).toBe(250);
    expect(details[0].totalTokens).toBe(250);
  });

  it("defaults the persistent breakdown to the visible range end", () => {
    const contributions = [contribution("2026-07-10", 250, 10)];

    expect(
      getDefaultContributionDate(contributions, "2026-07-05", "2026-07-11"),
    ).toBe("2026-07-11");
    expect(
      getContributionDayForDate(contributions, "2026-07-11"),
    ).toMatchObject({
      date: "2026-07-11",
      totals: { cost: 0, messages: 0, tokens: 0 },
    });
  });

  it("does not revive a requested day after the visible range changes", () => {
    const requested = {
      date: "2026-07-10",
      rangeIdentity: "2026-07-05:2026-07-11",
    };

    expect(
      resolveContributionSelectedDate(
        requested,
        "2026-07-05:2026-07-11",
        "2026-07-11",
      ),
    ).toBe("2026-07-10");
    expect(
      resolveContributionSelectedDate(
        requested,
        "2025-07-11:2026-07-11",
        "2026-07-11",
      ),
    ).toBe("2026-07-11");

    const weekSelection = reconcileContributionSelectionRange(
      {
        date: "2026-06-17",
        rangeIdentity: "2025-07-11:2026-07-11",
      },
      "2026-07-05:2026-07-11",
    );
    const lifetimeSelection = reconcileContributionSelectionRange(
      weekSelection,
      "2025-07-11:2026-07-11",
    );
    expect(weekSelection).toEqual({
      date: null,
      rangeIdentity: "2026-07-05:2026-07-11",
    });
    expect(lifetimeSelection).toEqual({
      date: null,
      rangeIdentity: "2025-07-11:2026-07-11",
    });
  });

  it("builds finite isometric cells from the same scoped calendar", () => {
    const calendar = createContributionCalendar(
      [
        contribution("2026-07-05", 0, 0),
        contribution("2026-07-06", 400, 10),
        contribution("2026-07-07", 200, 5),
      ],
      "2026-07-05",
      "2026-07-18",
    );

    const geometry = createContributionIsometricGeometry(calendar);
    const empty = geometry.cells.find(({ cell }) => cell.date === "2026-07-05");
    const active = geometry.cells.find(
      ({ cell }) => cell.date === "2026-07-06",
    );
    const midpoint = geometry.cells.find(
      ({ cell }) => cell.date === "2026-07-07",
    );

    expect(geometry.cells).toHaveLength(14);
    expect(geometry.viewBox.width).toBeGreaterThan(0);
    expect(geometry.viewBox.height).toBeGreaterThan(0);
    expect(empty?.height).toBe(1.5);
    expect(midpoint?.height).toBe(52);
    expect(active?.height).toBe(100);
    expect(active?.height).toBeGreaterThan(empty?.height ?? 0);
    expect(
      geometry.cells.every(({ centerX, centerY, height }) =>
        [centerX, centerY, height].every(Number.isFinite),
      ),
    ).toBe(true);
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

  it("maps compact chart taps to the nearest contribution day", () => {
    const targets = [
      { bottom: 4, date: "2026-07-10", left: 0, right: 4, top: 0 },
      { bottom: 4, date: "2026-07-11", left: 5, right: 9, top: 0 },
    ];

    expect(getNearestContributionDate(targets, 4.75, 2)).toBe("2026-07-11");
    expect(getNearestContributionDate(targets, -20, 2)).toBe("2026-07-10");
    expect(getNearestContributionDate(targets, -25, 2)).toBeNull();
  });

  it("keeps every active palette level distinct from the dark empty cell", () => {
    for (const palette of Object.values(colorPalettes)) {
      const colors = ([1, 2, 3, 4] as const).map((level) =>
        getContributionColor(palette, level),
      );
      const luminances = colors.map(relativeLuminance);

      expect(new Set(colors).size).toBe(4);
      expect(luminances).toEqual([...luminances].sort((a, b) => a - b));
      for (let index = 1; index < luminances.length; index += 1) {
        expect(
          luminances[index] - luminances[index - 1],
        ).toBeGreaterThanOrEqual(0.02);
      }

      for (const color of colors) {
        expect(contrastRatio(color, "#191f2b")).toBeGreaterThanOrEqual(3);
      }
    }
  });
});
