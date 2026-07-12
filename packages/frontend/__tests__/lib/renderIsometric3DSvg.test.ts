import { describe, expect, it, vi } from "vitest";
import {
  renderIsometric3DEmbedSvg,
  renderIsometric3DErrorSvg,
} from "../../src/lib/embed/renderIsometric3DSvg";
import type {
  UserEmbedStats,
  EmbedContributionDay,
} from "../../src/lib/embed/getUserEmbedStats";

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
  { date: "2026-01-15", intensity: 0, totalTokens: 0, totalCost: 0 },
  { date: "2026-02-10", intensity: 2, totalTokens: 50_000, totalCost: 1.5 },
  { date: "2026-02-20", intensity: 4, totalTokens: 500_000, totalCost: 12.0 },
  { date: "2026-03-01", intensity: 1, totalTokens: 10_000, totalCost: 0.4 },
  { date: "2026-03-10", intensity: 3, totalTokens: 150_000, totalCost: 5.0 },
];

function attributes(source: string): Record<string, string> {
  return Object.fromEntries(
    [...source.matchAll(/([\w:-]+)="([^"]*)"/g)].map((match) => [
      match[1],
      match[2],
    ]),
  );
}

function textNodes(svg: string): Array<{
  attrs: Record<string, string>;
  text: string;
}> {
  return [...svg.matchAll(/<text\b([^>]*)>([\s\S]*?)<\/text>/g)].map(
    (match) => ({
      attrs: attributes(match[1]),
      text: match[2]
        .replace(/<[^>]+>/g, "")
        .replace(/&amp;/g, "&")
        .replace(/&lt;/g, "<")
        .replace(/&gt;/g, ">")
        .replace(/&quot;/g, '"')
        .replace(/&apos;/g, "'"),
    }),
  );
}

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

  it("identifies the template and preserves root accessibility", () => {
    const svg = renderIsometric3DEmbedSvg(mockStats, mockContributions);

    expect(svg).toContain('<svg data-template="3d"');
    expect(svg).toContain('role="img"');
    expect(svg).toContain(
      'aria-label="Tokscale 3D contribution graph for @octocat"',
    );
    expect(svg).toContain(
      "<title>@octocat Tokscale 3D contribution graph</title>",
    );
  });

  it("renders one flat data rail with the canonical usage metrics", () => {
    const svg = renderIsometric3DEmbedSvg(mockStats, mockContributions);

    expect(svg).toContain("Tokens");
    expect(svg).toContain("Cost");
    expect(svg).toContain("Rank");
    expect(svg).toContain("Active days");
    expect(svg).toContain("1,234,567");
    expect(svg).toContain("$42.42");
  });

  it("computes active days from contributions with intensity > 0", () => {
    const svg = renderIsometric3DEmbedSvg(mockStats, mockContributions);

    expect(svg).toMatch(/>Active days<\/text><text[^>]*>4<\/text>/);
  });

  it("uses a local system font stack without external font loading", () => {
    const svg = renderIsometric3DEmbedSvg(mockStats, mockContributions);

    expect(svg).toContain(
      'font-family="-apple-system, BlinkMacSystemFont, Segoe UI, sans-serif"',
    );
    expect(svg).not.toContain("@import");
    expect(svg).not.toContain("fonts.googleapis.com");
  });

  it("omits decorative effects, animation, and fabricated stat chrome", () => {
    const svg = renderIsometric3DEmbedSvg(mockStats, mockContributions);

    expect(svg).not.toContain("<linearGradient");
    expect(svg).not.toContain("<radialGradient");
    expect(svg).not.toContain("<animate");
    expect(svg).not.toContain("filter=");
    expect(svg).not.toContain('fill="url(');
    expect(svg).not.toContain("Tokscale Stats");
    expect(svg).not.toContain("Token Usage");
    expect(svg).not.toContain("Streaks");
  });

  it("renders with dark theme by default", () => {
    const svg = renderIsometric3DEmbedSvg(mockStats, mockContributions);

    expect(svg).toContain('fill="#131822"');
    expect(svg).toContain('fill="#F4F7FB"');
  });

  it("renders with light theme when specified", () => {
    const svg = renderIsometric3DEmbedSvg(mockStats, mockContributions, {
      theme: "light",
    });

    expect(svg).toContain('fill="#FFFFFF"');
    expect(svg).toContain('fill="#1F2328"');
  });

  it("uses distinct colors for each cube intensity", () => {
    const svg = renderIsometric3DEmbedSvg(mockStats, mockContributions);
    const topFaceFills = [...svg.matchAll(/\.d\d-t\{fill:([^}]+)\}/g)].map(
      ([, fill]) => fill,
    );

    expect(new Set(topFaceFills).size).toBe(5);
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
    expect(svg).toContain("No activity yet");
    expect(svg).toMatch(/>Active days<\/text><text[^>]*>0<\/text>/);
  });

  it("preserves the public 680 by 482 SVG dimensions", () => {
    const svg = renderIsometric3DEmbedSvg(mockStats, mockContributions);

    expect(svg).toContain('width="680"');
    expect(svg).toContain('height="482"');
    expect(svg).toContain('viewBox="0 0 680 482"');
  });

  it("stays exactly 680 by 482 on a date that spans 54 calendar columns", () => {
    vi.useFakeTimers();
    vi.setSystemTime(new Date("2024-03-03T12:00:00.000Z"));

    try {
      const svg = renderIsometric3DEmbedSvg(mockStats, mockContributions);

      expect(svg).toContain('width="680"');
      expect(svg).toContain('height="482"');
      expect(svg).toContain('viewBox="0 0 680 482"');
      expect(svg).not.toContain('height="487"');
    } finally {
      vi.useRealTimers();
    }
  });

  it("shows rank as dash when null", () => {
    const svg = renderIsometric3DEmbedSvg(
      { ...mockStats, stats: { ...mockStats.stats, rank: null } },
      mockContributions,
    );

    expect(svg).toContain("Rank");
    expect(svg).toContain(">\u2014</text>");
  });

  it("preserves compact number formatting", () => {
    const stats = {
      ...mockStats,
      stats: { ...mockStats.stats, totalCost: 12_345.67 },
    };
    const regularSvg = renderIsometric3DEmbedSvg(stats, mockContributions);
    const compactSvg = renderIsometric3DEmbedSvg(stats, mockContributions, {
      compact: true,
    });

    expect(regularSvg).toContain("1,234,567");
    expect(regularSvg).toContain("$12,345.67");
    expect(compactSvg).toContain("1.2M");
    expect(compactSvg).toContain("$12.3K");
  });

  it("shows date range from first to last active contribution", () => {
    const svg = renderIsometric3DEmbedSvg(mockStats, mockContributions);

    expect(svg).toContain("Feb 10");
    expect(svg).toContain("Mar 10, 2026");
    expect(svg).toContain("\u2192");
  });

  it("ignores buffered contributions outside the visible year", () => {
    const baseline = renderIsometric3DEmbedSvg(mockStats, mockContributions);
    const withBuffer = renderIsometric3DEmbedSvg(mockStats, [
      {
        date: "2000-01-01",
        intensity: 4,
        totalTokens: 999_999_999,
        totalCost: 9_999,
      },
      ...mockContributions,
    ]);

    expect(withBuffer).toBe(baseline);
  });

  it("uses one solid theme-appropriate surface", () => {
    const darkSvg = renderIsometric3DEmbedSvg(mockStats, mockContributions);
    const lightSvg = renderIsometric3DEmbedSvg(mockStats, mockContributions, {
      theme: "light",
    });

    expect(darkSvg).toContain(
      '<rect width="680" height="482" rx="12" fill="#131822"/>',
    );
    expect(lightSvg).toContain(
      '<rect width="680" height="482" rx="12" fill="#FFFFFF"/>',
    );
  });

  it("uses the profile service empty color and monotonic blue grades", () => {
    const svg = renderIsometric3DEmbedSvg(mockStats, mockContributions);

    expect(svg).toContain(".d0-t{fill:#191F2B}");
    expect(svg).toContain(".d1-t{fill:#4069b2}");
    expect(svg).toContain(".d2-t{fill:#1f6feb}");
    expect(svg).toContain(".d3-t{fill:#388bfd}");
    expect(svg).toContain(".d4-t{fill:#79b8ff}");
  });

  it("scales cube heights by usage within the same intensity bucket", () => {
    const svg = renderIsometric3DEmbedSvg(mockStats, [
      { date: "2026-02-10", intensity: 4, totalTokens: 1, totalCost: 1 },
      { date: "2026-02-11", intensity: 4, totalTokens: 1000, totalCost: 10 },
    ]);

    expect(svg).toContain('height="7"');
    expect(svg).toContain('height="30.4"');
  });

  it("keeps the complete SVG unchanged when only contribution cost changes", () => {
    const today = new Date();
    const currentUtcDate = new Date(
      Date.UTC(today.getUTCFullYear(), today.getUTCMonth(), today.getUTCDate()),
    );
    const previousUtcDate = new Date(currentUtcDate);
    previousUtcDate.setUTCDate(previousUtcDate.getUTCDate() - 1);
    const baseline: EmbedContributionDay[] = [
      {
        date: previousUtcDate.toISOString().split("T")[0],
        intensity: 4,
        totalTokens: 100,
        totalCost: 0,
      },
      {
        date: currentUtcDate.toISOString().split("T")[0],
        intensity: 1,
        totalTokens: 0,
        totalCost: 0,
      },
    ];
    const costOnlyMutation = baseline.map((day, index) =>
      index === 1 ? { ...day, totalCost: 1_000_000 } : day,
    );

    expect(renderIsometric3DEmbedSvg(mockStats, costOnlyMutation)).toBe(
      renderIsometric3DEmbedSvg(mockStats, baseline),
    );
  });
});

describe("renderIsometric3DErrorSvg", () => {
  it("renders a valid error SVG", () => {
    const svg = renderIsometric3DErrorSvg("Something went wrong");

    expect(svg).toContain('<svg data-template="3d-error"');
    expect(svg).toContain('role="img"');
    expect(svg).toContain("Something went wrong");
    expect(svg).toContain(">Tokscale</text>");
  });

  it("escapes XML in error message", () => {
    const svg = renderIsometric3DErrorSvg("User <unknown> not found");

    expect(svg).toContain("User &lt;unknown&gt; not found");
    expect(svg).not.toContain("User <unknown> not found");
  });

  it("fits long error copy within the error-card content width", () => {
    const message =
      "프로필을 찾을 수 없습니다 · 使用量データを確認してください";
    const node = textNodes(renderIsometric3DErrorSvg(message)).find(
      (candidate) => candidate.text === message,
    );

    expect(node).toBeDefined();
    expect(Number(node?.attrs["data-fit-max-width"])).toBeGreaterThan(0);
  });

  it("supports light theme", () => {
    const svg = renderIsometric3DErrorSvg("Error", { theme: "light" });

    expect(svg).toContain(
      '<rect width="540" height="120" rx="12" fill="#FFFFFF"/>',
    );
  });

  it("uses the same restrained local design language", () => {
    const svg = renderIsometric3DErrorSvg("Error");

    expect(svg).toContain(
      'font-family="-apple-system, BlinkMacSystemFont, Segoe UI, sans-serif"',
    );
    expect(svg).not.toContain("@import");
    expect(svg).not.toContain("<linearGradient");
    expect(svg).not.toContain("<radialGradient");
    expect(svg).not.toContain("Tokscale Stats");
  });
});
