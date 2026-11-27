/** @vitest-environment jsdom */

import type { ReactNode } from "react";
import { renderToStaticMarkup } from "react-dom/server";
import { cleanup, fireEvent, render, screen, within } from "@testing-library/react";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import {
  MODEL_RANK_BY_STORAGE_KEY,
  ProfileModels,
  getFavoriteModel,
  isModelRankBy,
  rankModelUsage,
  type ModelUsage,
} from "../../src/components/profile";
import ProfilePageClient, {
  type ProfileData,
} from "../../src/app/u/[username]/ProfilePageClient";

vi.mock("next/link", () => ({
  default: ({
    href,
    children,
    ...rest
  }: {
    href: string;
    children: ReactNode;
    prefetch?: boolean;
  }) => {
    const anchorProps = { ...rest };
    delete anchorProps.prefetch;
    return (
      <a href={href} {...anchorProps}>
        {children}
      </a>
    );
  },
}));

vi.mock("@/components/layout/Navigation", () => ({
  Navigation: () => <nav aria-label="Primary navigation" />,
}));

vi.mock("@/components/layout/PublicFooter", () => ({
  PublicFooter: () => <footer />,
}));

vi.mock("@/components/profile/ProfileOverview", () => ({
  ProfileOverview: () => <section aria-label="Profile overview" />,
}));

vi.mock("@/components/profile/ProfileContributionGraph", async (importOriginal) => {
  const actual = await importOriginal<
    typeof import("@/components/profile/ProfileContributionGraph")
  >();
  return {
    ...actual,
    ProfileContributionGraph: () => <section aria-label="Contributions" />,
    ProfileContributionBreakdown: () => <section aria-label="Day breakdown" />,
  };
});

vi.mock("@/components/profile/ProfileUsageChart", () => ({
  ProfileUsageChart: () => <section aria-label="Usage over time" />,
}));

vi.mock("@/components/profile/TokenBreakdown", () => ({
  TokenBreakdown: () => <section aria-label="Token mix" />,
}));

vi.mock("@/components/profile/ProfileDevices", () => ({
  ProfileDevices: () => <section aria-label="Devices" />,
}));

const SAMPLE_USAGE: ModelUsage[] = [
  {
    model: "claude-opus-5-5",
    tokens: 2_400_000_000,
    cost: 1_700,
    percentage: 68,
  },
  {
    model: "glm-5-3",
    tokens: 9_000_000_000,
    cost: 800,
    percentage: 32,
  },
  {
    model: "swe-2-high",
    tokens: 7_200_000_000,
    cost: 0,
    percentage: 0,
  },
  {
    model: "<synthetic>",
    tokens: 99_000_000_000,
    cost: 99_999,
    percentage: 0,
  },
];

function createProfileData(overrides: Partial<ProfileData> = {}): ProfileData {
  return {
    user: {
      id: "user-1",
      username: "Jackson57279",
      displayName: "Jackson",
      avatarUrl: null,
      createdAt: "2026-05-15T16:49:26.163Z",
      rank: 275,
      rankDateRange: { start: "2026-10-01", end: "2026-10-31" },
    },
    stats: {
      totalTokens: 18_600_000_000,
      totalCost: 2_500,
      inputTokens: 800_000_000,
      outputTokens: 150_000_000,
      cacheReadTokens: 17_250_000_000,
      cacheWriteTokens: 400_000_000,
      reasoningTokens: 0,
      submissionCount: 12,
      activeDays: 31,
      sessionCount: 0,
    },
    dateRange: { start: "2026-10-01", end: "2026-10-31" },
    updatedAt: "2026-11-27T15:12:34.201Z",
    clients: ["claude", "droid"],
    models: ["claude-opus-5-5", "glm-5-3", "swe-2-high"],
    mcpServers: ["web"],
    modelUsage: SAMPLE_USAGE,
    contributions: [
      {
        date: "2026-10-31",
        totals: { tokens: 1_000_000, cost: 10, messages: 1 },
        intensity: 1,
        tokenBreakdown: {
          input: 500_000,
          output: 500_000,
          cacheRead: 0,
          cacheWrite: 0,
          reasoning: 0,
        },
        clients: [],
      },
    ],
    period: "last-month",
    ...overrides,
  };
}

beforeEach(() => {
  window.localStorage.clear();
});

afterEach(() => {
  cleanup();
  window.localStorage.clear();
});

describe("modelRanking helpers", () => {
  it("validates ModelRankBy values", () => {
    expect(isModelRankBy("cost")).toBe(true);
    expect(isModelRankBy("tokens")).toBe(true);
    expect(isModelRankBy("other")).toBe(false);
    expect(isModelRankBy(null)).toBe(false);
  });

  it("ranks models by cost or tokens and ignores <synthetic>", () => {
    const byCost = rankModelUsage(SAMPLE_USAGE, "cost");
    expect(byCost.map((item) => item.model)).toEqual([
      "claude-opus-5-5",
      "glm-5-3",
      "swe-2-high",
    ]);
    expect(byCost[0].share).toBeCloseTo(68, 4);
    expect(byCost[1].share).toBeCloseTo(32, 4);
    expect(byCost[2].share).toBe(0);

    const byTokens = rankModelUsage(SAMPLE_USAGE, "tokens");
    expect(byTokens.map((item) => item.model)).toEqual([
      "glm-5-3",
      "swe-2-high",
      "claude-opus-5-5",
    ]);
    expect(byTokens[0].share).toBeCloseTo((9 / 18.6) * 100, 4);
    expect(byTokens[1].share).toBeCloseTo((7.2 / 18.6) * 100, 4);
    expect(byTokens[2].share).toBeCloseTo((2.4 / 18.6) * 100, 4);
  });

  it("returns null share for zero-cost usage when ranked by cost while preserving token tie-breaks", () => {
    const zeroCostUsage: ModelUsage[] = [
      { model: "free-small", tokens: 500, cost: 0, percentage: 0 },
      { model: "free-large", tokens: 5_000, cost: 0, percentage: 0 },
    ];

    const favoriteByCost = getFavoriteModel(zeroCostUsage, "cost");
    expect(favoriteByCost?.model).toBe("free-large");
    expect(favoriteByCost?.share).toBeNull();

    const favoriteByTokens = getFavoriteModel(zeroCostUsage, "tokens");
    expect(favoriteByTokens?.model).toBe("free-large");
    expect(favoriteByTokens?.share).toBeCloseTo((5_000 / 5_500) * 100, 4);
  });

  it("returns null favorite when no non-synthetic usage exists", () => {
    expect(
      getFavoriteModel(
        [{ model: "<synthetic>", tokens: 100, cost: 1, percentage: 100 }],
        "tokens",
      ),
    ).toBeNull();
    expect(getFavoriteModel([], "cost")).toBeNull();
  });
});

describe("ProfileModels ranking toggle", () => {
  it("switches ordering, active aria-sort, and share percentages between cost and tokens", () => {
    render(
      <ProfileModels
        models={["claude-opus-5-5", "glm-5-3", "swe-2-high"]}
        modelUsage={SAMPLE_USAGE}
      />,
    );

    const table = screen.getByRole("table");
    const getRowModels = () =>
      within(table)
        .getAllByRole("row")
        .slice(1)
        .map((row) => within(row).getByRole("rowheader").textContent);

    expect(getRowModels()).toEqual([
      "claude-opus-5-5",
      "glm-5-3",
      "swe-2-high",
    ]);
    expect(
      screen.getByRole("columnheader", { name: "Cost" }).getAttribute("aria-sort"),
    ).toBe("descending");
    expect(
      screen.getByRole("columnheader", { name: "Tokens" }).getAttribute("aria-sort"),
    ).toBe("none");

    const toggleGroup = screen.getByRole("group", { name: "Rank models by" });
    fireEvent.click(within(toggleGroup).getByRole("button", { name: "Tokens" }));

    expect(getRowModels()).toEqual([
      "glm-5-3",
      "swe-2-high",
      "claude-opus-5-5",
    ]);
    expect(
      screen.getByRole("columnheader", { name: "Tokens" }).getAttribute("aria-sort"),
    ).toBe("descending");
    expect(
      screen.getByRole("columnheader", { name: "Cost" }).getAttribute("aria-sort"),
    ).toBe("none");
    expect(within(table).getByText("48.4%")).toBeTruthy();
    expect(within(table).getByText("38.7%")).toBeTruthy();
  });
});

describe("ProfilePageClient favorite model ranking", () => {
  it("renders cost ranking on SSR regardless of stored preference", () => {
    window.localStorage.setItem(MODEL_RANK_BY_STORAGE_KEY, "tokens");

    const markup = renderToStaticMarkup(
      <ProfilePageClient
        initialData={createProfileData()}
        username="Jackson57279"
      />,
    );

    expect(markup).toContain("Favorite model (Last month)");
    expect(markup).toContain("claude-opus-5-5");
    expect(markup).toContain("$1.70K · 68.0%");
    expect(markup).not.toContain("9.0B tokens");
  });

  it("toggles favorite model between cost and tokens, persists preference, and syncs the Models tab", () => {
    render(
      <ProfilePageClient
        initialData={createProfileData()}
        username="Jackson57279"
      />,
    );

    const insights = screen.getByRole("region", { name: "Usage details" });
    expect(within(insights).getByText("claude-opus-5-5")).toBeTruthy();
    expect(within(insights).getByText("$1.70K · 68.0%")).toBeTruthy();

    const rankGroup = within(insights).getByRole("group", {
      name: "Rank favorite model by",
    });
    const costButton = within(rankGroup).getByRole("button", { name: "Cost" });
    const tokensButton = within(rankGroup).getByRole("button", {
      name: "Tokens",
    });
    expect(costButton.getAttribute("aria-pressed")).toBe("true");
    expect(tokensButton.getAttribute("aria-pressed")).toBe("false");

    fireEvent.click(tokensButton);

    expect(tokensButton.getAttribute("aria-pressed")).toBe("true");
    expect(within(insights).getByText("glm-5-3")).toBeTruthy();
    expect(within(insights).getByText("9.0B tokens · 48.4%")).toBeTruthy();
    expect(window.localStorage.getItem(MODEL_RANK_BY_STORAGE_KEY)).toBe(
      "tokens",
    );

    // Switching to Models tab reflects the same token ranking
    fireEvent.click(screen.getByRole("tab", { name: "Models" }));
    const table = screen.getByRole("table");
    const firstRowModel = within(
      within(table).getAllByRole("row")[1],
    ).getByRole("rowheader").textContent;
    expect(firstRowModel).toBe("glm-5-3");

    // Switching back to Cost in the Models tab syncs back to Usage details
    const modelsRankGroup = screen.getByRole("group", {
      name: "Rank models by",
    });
    fireEvent.click(
      within(modelsRankGroup).getByRole("button", { name: "Cost" }),
    );
    fireEvent.click(screen.getByRole("tab", { name: "Usage" }));

    const refreshedInsights = screen.getByRole("region", {
      name: "Usage details",
    });
    expect(within(refreshedInsights).getByText("claude-opus-5-5")).toBeTruthy();
    expect(window.localStorage.getItem(MODEL_RANK_BY_STORAGE_KEY)).toBe("cost");
  });

  it("restores a saved tokens preference on client mount", () => {
    window.localStorage.setItem(MODEL_RANK_BY_STORAGE_KEY, "tokens");

    render(
      <ProfilePageClient
        initialData={createProfileData()}
        username="Jackson57279"
      />,
    );

    const insights = screen.getByRole("region", { name: "Usage details" });
    expect(within(insights).getByText("glm-5-3")).toBeTruthy();
    expect(within(insights).getByText("9.0B tokens · 48.4%")).toBeTruthy();
  });

  it("hides the rank toggle when no non-synthetic model usage exists", () => {
    render(
      <ProfilePageClient
        initialData={createProfileData({ modelUsage: [] })}
        username="Jackson57279"
      />,
    );

    const insights = screen.getByRole("region", { name: "Usage details" });
    expect(within(insights).getByText("Not available")).toBeTruthy();
    expect(
      within(insights).queryByRole("group", { name: "Rank favorite model by" }),
    ).toBeNull();
  });
});
