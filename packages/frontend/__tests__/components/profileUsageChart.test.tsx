/** @vitest-environment jsdom */

import { renderToStaticMarkup } from "react-dom/server";
import { cleanup, fireEvent, render, screen } from "@testing-library/react";
import { afterEach, describe, expect, it } from "vitest";
import { ProfileUsageChart } from "../../src/components/profile/ProfileUsageChart";
import type { DailyContribution } from "../../src/lib/types";

const EMPTY_TOKENS = {
  input: 0,
  output: 0,
  cacheRead: 0,
  cacheWrite: 0,
  reasoning: 0,
};

function contribution(date: string, input: number): DailyContribution {
  return {
    date,
    totals: { tokens: input, cost: 0, messages: 1 },
    intensity: 1,
    tokenBreakdown: { ...EMPTY_TOKENS, input },
    clients: [
      {
        client: "claude",
        modelId: "claude-test",
        tokens: { ...EMPTY_TOKENS, input },
        cost: 0,
        messages: 1,
      },
    ],
  };
}

afterEach(() => {
  cleanup();
});

describe("ProfileUsageChart", () => {
  it("renders an unset preference in chronological order", () => {
    const markup = renderToStaticMarkup(
      <ProfileUsageChart
        contributions={[
          contribution("2026-01-03", 30),
          contribution("2026-01-01", 10),
          contribution("2026-01-02", 20),
        ]}
      />,
    );

    const range = markup.slice(markup.indexOf('aria-label="Chart date range"'));
    expect(range).toContain("Jan 1, 2026");
    expect(range).toContain("Jan 3, 2026");
    expect(range.indexOf("Jan 1, 2026")).toBeLessThan(
      range.indexOf("Jan 3, 2026"),
    );
  });

  it("renders display and provider select controls", () => {
    const markup = renderToStaticMarkup(
      <ProfileUsageChart
        contributions={[
          contribution("2026-01-01", 10),
        ]}
      />,
    );

    expect(markup).toMatch(/<button[^>]*role="combobox"[^>]*aria-label="Usage display"/);
    expect(markup).toMatch(/<button[^>]*role="combobox"[^>]*aria-label="Usage provider"/);
    expect(markup).toMatch(/<input type="hidden" name="profile-usage-provider" value="all"\/>/);
  });

  it("lists every provider in the provider select and switches the filter", () => {
    render(
      <ProfileUsageChart
        contributions={[
          contribution("2026-01-01", 10),
        ]}
      />,
    );

    const display = screen.getByRole("combobox", { name: "Usage display" });
    display.focus();
    fireEvent.keyDown(display, { key: "ArrowDown" });
    expect(screen.getAllByRole("option").map((option) => option.textContent)).toEqual(["30d average", "Daily"]);
    fireEvent.keyDown(display, { key: "Escape" });

    const provider = screen.getByRole("combobox", { name: "Usage provider" });
    provider.focus();
    fireEvent.keyDown(provider, { key: "ArrowDown" });
    expect(screen.getAllByRole("option").map((option) => option.textContent)).toEqual(["All", "Claude Code"]);
    fireEvent.keyDown(provider, { key: "ArrowDown" });
    fireEvent.keyDown(provider, { key: "Enter" });

    const hidden = document.querySelector('input[name="profile-usage-provider"]') as HTMLInputElement;
    expect(hidden.value).toBe("claude");
    expect(provider.getAttribute("aria-expanded")).toBe("false");
  });
});


describe("ProfileUsageChart keyboard inspection", () => {
  it("steps through days with the arrow keys and describes the day", () => {
    render(
      <ProfileUsageChart
        contributions={[
          contribution("2026-01-01", 10),
          contribution("2026-01-02", 20),
        ]}
      />,
    );

    const plot = screen.getByRole("group", {
      name: /Interactive tokens chart/,
    });
    fireEvent.keyDown(plot, { key: "ArrowLeft" });

    // The newest day is inspected first; one step left lands on the older day.
    // Both the tooltip and the pinned panel carry the inspected day.
    expect(screen.getAllByText("2026-01-01").length).toBeGreaterThan(0);
    expect(screen.getByRole("status").textContent).toContain("Jan 1, 2026");
  });
});
