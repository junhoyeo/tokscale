/** @vitest-environment jsdom */

import { cleanup, fireEvent, render } from "@testing-library/react";
import { afterEach, describe, expect, it, vi } from "vitest";
import { ProfileContributionGraph } from "../../src/components/profile/ProfileContributionGraph";
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

describe("ProfileContributionGraph keyboard selection", () => {
  it("selects a day with Enter and moves roving focus with the arrow keys", () => {
    const onSelectedDateChange = vi.fn();
    render(
      <ProfileContributionGraph
        contributions={[
          contribution("2026-01-01", 10),
          contribution("2026-01-02", 20),
          contribution("2026-01-03", 30),
        ]}
        rangeStart="2026-01-01"
        rangeEnd="2026-01-03"
        onSelectedDateChange={onSelectedDateChange}
      />,
    );

    const days = [
      ...document.querySelectorAll<HTMLButtonElement>(
        "[data-contribution-date]",
      ),
    ].filter((node) => !node.disabled);
    expect(days.length).toBe(3);
    const first = days.find(
      (node) => node.getAttribute("data-contribution-date") === "2026-01-01",
    );
    expect(first).toBeTruthy();

    first?.focus();
    fireEvent.keyDown(first as HTMLElement, { key: "ArrowRight" });
    expect(document.activeElement?.getAttribute("data-contribution-date")).toBe(
      "2026-01-02",
    );

    fireEvent.keyDown(document.activeElement as HTMLElement, { key: "Enter" });
    expect(onSelectedDateChange).toHaveBeenCalledWith("2026-01-02");
  });
});
