import { describe, expect, it } from "vitest";
import {
  renderIsometric3DEmbedSvg,
  renderIsometric3DErrorSvg,
} from "../../src/lib/embed/renderIsometric3DSvg";
import type { UserEmbedStats, EmbedContributionDay } from "../../src/lib/embed/getUserEmbedStats";

const mockStats: UserEmbedStats = {
  user: {
    id: "user-id",
    username: "octocat",
    displayName: "The Octocat",
    avatarUrl: null,
  },
  stats: {
    totalTokens: 1234567,
    totalCost: 42.42,
    submissionCount: 7,
    rank: 3,
    updatedAt: "2026-02-24T00:00:00.000Z",
  },
};

const mockContributions: EmbedContributionDay[] = [
  { date: "2026-01-15", intensity: 0 },
  { date: "2026-02-10", intensity: 2 },
  { date: "2026-02-20", intensity: 4 },
  { date: "2026-03-01", intensity: 1 },
  { date: "2026-03-10", intensity: 3 },
];

describe("renderIsometric3DEmbedSvg", () => {
  it("renders a valid SVG with rect-based isometric cubes", () => {
    const svg = renderIsometric3DEmbedSvg(mockStats, mockContributions);

    expect(svg).toContain("<svg");
    expect(svg).toContain("</svg>");
    expect(svg).toContain("<rect");
    expect(svg).toContain("skewY");
  });

  it("renders three rect faces per cube (top, left, right via CSS classes)", () => {
    const svg = renderIsometric3DEmbedSvg(mockStats, mockContributions);

    expect(svg).toContain('class="d0-t"');
    expect(svg).toContain('class="d0-l"');
    expect(svg).toContain('class="d0-r"');
  });

  it("contains the username", () => {
    const svg = renderIsometric3DEmbedSvg(mockStats, mockContributions);

    expect(svg).toContain("@octocat");
  });

  it("renders Token Usage stats box with cost and tokens", () => {
    const svg = renderIsometric3DEmbedSvg(mockStats, mockContributions);

    expect(svg).toContain("Token Usage");
    expect(svg).toContain("Total");
    expect(svg).toContain("Tokens");
    expect(svg).toContain("active days");
  });

  it("renders Streaks stats box", () => {
    const svg = renderIsometric3DEmbedSvg(mockStats, mockContributions);

    expect(svg).toContain("Streaks");
    expect(svg).toContain("Longest");
    expect(svg).toContain("Current");
    expect(svg).toContain("days");
  });

  it("computes active days from contributions with intensity > 0", () => {
    const svg = renderIsometric3DEmbedSvg(mockStats, mockContributions);

    expect(svg).toContain("4 active days");
  });

  it("uses Figtree font", () => {
    const svg = renderIsometric3DEmbedSvg(mockStats, mockContributions);

    expect(svg).toContain("family=Figtree");
    expect(svg).toContain('font-family="Figtree');
  });

  it("renders with dark theme by default", () => {
    const svg = renderIsometric3DEmbedSvg(mockStats, mockContributions);

    expect(svg).toContain('stop-color="#0D1117"');
  });

  it("renders with light theme when specified", () => {
    const svg = renderIsometric3DEmbedSvg(mockStats, mockContributions, { theme: "light" });

    expect(svg).toContain('stop-color="#FFFFFF"');
  });

  it("uses different polygon fill colors for cube faces", () => {
    const svg = renderIsometric3DEmbedSvg(mockStats, mockContributions);
    const fills = svg.match(/fill="(#[0-9a-fA-F]{6})"/g) || [];
    const uniqueFills = new Set(fills);

    expect(uniqueFills.size).toBeGreaterThan(3);
  });

  it("includes tokscale.ai profile link", () => {
    const svg = renderIsometric3DEmbedSvg(mockStats, mockContributions);

    expect(svg).toContain("tokscale.ai/u/octocat");
  });

  it("escapes XML in user-provided text", () => {
    const svg = renderIsometric3DEmbedSvg(
      {
        ...mockStats,
        user: { ...mockStats.user, username: "test<user" },
      },
      mockContributions,
    );

    expect(svg).toContain("@test&lt;user");
    expect(svg).not.toContain("@test<user");
  });

  it("handles empty contributions array gracefully", () => {
    const svg = renderIsometric3DEmbedSvg(mockStats, []);

    expect(svg).toContain("<svg");
    expect(svg).toContain("skewY");
    expect(svg).toContain("0 active days");
    expect(svg).toContain("0 days");
  });

  it("renders fixed-width SVG of 680px", () => {
    const svg = renderIsometric3DEmbedSvg(mockStats, mockContributions);

    expect(svg).toContain('width="680"');
  });

  it("shows rank as dash when null", () => {
    const svg = renderIsometric3DEmbedSvg(
      { ...mockStats, stats: { ...mockStats.stats, rank: null } },
      mockContributions,
    );

    expect(svg).toContain("Rank");
  });

  it("shows date range from first to last active contribution", () => {
    const svg = renderIsometric3DEmbedSvg(mockStats, mockContributions);

    expect(svg).toContain("02/10");
    expect(svg).toContain("03/10");
    expect(svg).toContain("\u2192");
  });

  it("renders stats box backgrounds with theme-appropriate colors", () => {
    const darkSvg = renderIsometric3DEmbedSvg(mockStats, mockContributions);
    const lightSvg = renderIsometric3DEmbedSvg(mockStats, mockContributions, { theme: "light" });

    expect(darkSvg).toContain('fill="#1A212A"');
    expect(lightSvg).toContain('fill="#F6F8FA"');
  });

  it("uses blue palette for graph cubes (matching frontend default)", () => {
    const svg = renderIsometric3DEmbedSvg(mockStats, mockContributions);

    expect(svg).toContain("fill:#79b8ff");
    expect(svg).toContain("fill:#1A212A");
  });

  it("scales cube heights by usage within the same intensity bucket", () => {
    const svg = renderIsometric3DEmbedSvg(mockStats, [
      { date: "2026-02-10", intensity: 4, totalTokens: 1, totalCost: 1 },
      { date: "2026-02-11", intensity: 4, totalTokens: 1000, totalCost: 10 },
    ]);

    expect(svg).toContain('height="7"');
    expect(svg).toContain('height="30.4"');
  });

});

describe("renderIsometric3DErrorSvg", () => {
  it("renders a valid error SVG", () => {
    const svg = renderIsometric3DErrorSvg("Something went wrong");

    expect(svg).toContain("<svg");
    expect(svg).toContain("Something went wrong");
    expect(svg).toContain("Tokscale Stats");
  });

  it("escapes XML in error message", () => {
    const svg = renderIsometric3DErrorSvg("User <unknown> not found");

    expect(svg).toContain("User &lt;unknown&gt; not found");
    expect(svg).not.toContain("User <unknown> not found");
  });

  it("supports light theme", () => {
    const svg = renderIsometric3DErrorSvg("Error", { theme: "light" });

    expect(svg).toContain('stop-color="#FFFFFF"');
  });

  it("uses Figtree font", () => {
    const svg = renderIsometric3DErrorSvg("Error");

    expect(svg).toContain("family=Figtree");
  });
});
