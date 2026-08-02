import { renderToStaticMarkup } from "react-dom/server";
import { describe, expect, it } from "vitest";
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
});
