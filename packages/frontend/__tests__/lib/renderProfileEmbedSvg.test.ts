import { describe, expect, it } from "vitest";
import {
  renderProfileEmbedErrorSvg,
  renderProfileEmbedSvg,
} from "../../src/lib/embed/renderProfileEmbedSvg";
import type { UserEmbedStats } from "../../src/lib/embed/getUserEmbedStats";

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

describe("renderProfileEmbedSvg", () => {
  it("renders a complete SVG with metrics", () => {
    const svg = renderProfileEmbedSvg(mockStats);

    expect(svg).toContain("<svg");
    expect(svg).toContain("Tokscale Stats");
    expect(svg).toContain("@octocat");
    expect(svg).toContain("1,234,567");
    expect(svg).toContain("$42.42");
    expect(svg).toContain("#3");
    expect(svg).not.toContain("Submissions");
  });

  it("uses Figtree font in SVG", () => {
    const svg = renderProfileEmbedSvg(mockStats);

    expect(svg).toContain("family=Figtree");
    expect(svg).toContain('font-family="Figtree');
  });

  it("renders compact variant", () => {
    const svg = renderProfileEmbedSvg(mockStats, { compact: true, theme: "light" });

    expect(svg).toContain('width="460"');
    expect(svg).toContain('height="162"');
    expect(svg).toContain("Tokscale Stats");
    expect(svg).toContain("@octocat");
    expect(svg).toContain('stop-color="#FFFFFF"');
    expect(svg).not.toContain("Submissions");
  });

  it("supports compact number notation when enabled", () => {
    const svg = renderProfileEmbedSvg(mockStats, { compactNumbers: true });

    expect(svg).toContain("1.2M");
  });

  it("renders rank label based on selected sorting", () => {
    const tokensSvg = renderProfileEmbedSvg(mockStats, { sortBy: "tokens" });
    const costSvg = renderProfileEmbedSvg(mockStats, { sortBy: "cost" });

    expect(tokensSvg).toContain("Rank (Tokens)");
    expect(costSvg).toContain("Rank (Cost)");
    expect(tokensSvg).toContain("RANK · TOKENS");
    expect(costSvg).toContain("RANK · COST");
  });

  it("uses gradient tokens, green cost, and rank-specific colors", () => {
    const svg = renderProfileEmbedSvg(mockStats);

    expect(svg).toContain('id="token-grad"');
    expect(svg).toContain('fill="url(#token-grad)"');
    expect(svg).toContain('fill="#3FB950"');
    expect(svg).toContain('fill="#DA7E1A"');
  });

  it("uses gold color for rank #1", () => {
    const svg = renderProfileEmbedSvg({
      ...mockStats,
      stats: { ...mockStats.stats, rank: 1 },
    });
    expect(svg).toContain('fill="#E3B341"');
  });

  it("renders redesigned card structure with brand icon and accent bars", () => {
    const svg = renderProfileEmbedSvg(mockStats);

    expect(svg).toContain('id="bg"');
    expect(svg).toContain('id="glow"');
    expect(svg).toContain('id="divider-grad"');
    expect(svg).toContain('id="acc-tokens"');
    expect(svg).toContain('id="acc-cost"');
    expect(svg).toContain('id="acc-rank"');
    expect(svg).toContain('clip-path="url(#card-clip)"');
  });

  it("escapes XML in user-provided text", () => {
    const svg = renderProfileEmbedSvg({
      ...mockStats,
      user: {
        ...mockStats.user,
        displayName: "<script>alert('xss')</script>",
      },
    });

    expect(svg).toContain("&lt;script&gt;alert(&apos;xss&apos;)&lt;/script&gt;");
    expect(svg).not.toContain("<script>alert('xss')</script>");
  });

  it("does not contain raw & outside XML entities (well-formed XML)", () => {
    const svg = renderProfileEmbedSvg(mockStats);

    const stripped = svg.replace(/&(amp|lt|gt|quot|apos|#\d+|#x[0-9a-fA-F]+);/g, "");
    expect(stripped).not.toContain("&");
  });

  it("positions display name dynamically after username", () => {
    const svg = renderProfileEmbedSvg(mockStats);

    const displayNameTag = svg.match(/<text x="(\d+(?:\.\d+)?)"[^>]*>The Octocat<\/text>/);
    expect(displayNameTag).toBeTruthy();
    const x = Number(displayNameTag![1]);
    expect(x).toBeGreaterThanOrEqual(24 + 8 * 17 * 0.6 + 8);
  });

  it("hides display name when username is too long to leave room", () => {
    const longUsername = "a".repeat(50);
    const svg = renderProfileEmbedSvg(
      {
        ...mockStats,
        user: {
          ...mockStats.user,
          username: longUsername,
          displayName: "Should Be Hidden",
        },
      },
      { compact: true }
    );
    expect(svg).not.toContain("Should Be Hidden");
  });

  it("computes display name collision width from raw text, not XML-escaped", () => {
    // In compact mode this name fits when measured as raw text (29 chars),
    // but would be hidden if measured after XML escaping (33 chars).
    const displayName = `${"A".repeat(14)} & ${"B".repeat(12)}`;
    const expectedDisplayName = `${"A".repeat(14)} &amp; ${"B".repeat(12)}`;

    const compactSvg = renderProfileEmbedSvg(
      {
        ...mockStats,
        user: {
          ...mockStats.user,
          username: "short",
          displayName,
        },
      },
      { compact: true }
    );
    const defaultSvg = renderProfileEmbedSvg({
      ...mockStats,
      user: {
        ...mockStats.user,
        username: "short",
        displayName,
      },
    });

    expect(compactSvg).toContain(expectedDisplayName);
    expect(defaultSvg).toContain(expectedDisplayName);
  });

  it("auto-scales font size for very long token values", () => {
    const svg = renderProfileEmbedSvg({
      ...mockStats,
      stats: { ...mockStats.stats, totalTokens: 15726314363 },
    });

    expect(svg).toContain("15,726,314,363");
    const valueTag = svg.match(/font-size="(\d+)"[^>]*font-weight="800"[^>]*>15,726,314,363/);
    expect(valueTag).toBeTruthy();
    const fontSize = Number(valueTag![1]);
    expect(fontSize).toBeLessThan(28);
    expect(fontSize).toBeGreaterThanOrEqual(14);
  });
});

describe("renderProfileEmbedSvg with contributions graph", () => {
  const mockContributions = [
    { date: "2026-01-15", intensity: 0 as const },
    { date: "2026-02-10", intensity: 2 as const },
    { date: "2026-02-20", intensity: 4 as const },
  ];

  it("extends card height when contributions provided", () => {
    const withoutGraph = renderProfileEmbedSvg(mockStats);
    const withGraph = renderProfileEmbedSvg(mockStats, { contributions: mockContributions });

    expect(withoutGraph).toContain('height="186"');
    const heightMatch = withGraph.match(/height="(\d+)"/);
    expect(heightMatch).toBeTruthy();
    expect(Number(heightMatch![1])).toBeGreaterThan(186);
  });

  it("renders GitHub-style contribution grid cells", () => {
    const svg = renderProfileEmbedSvg(mockStats, { contributions: mockContributions });

    expect(svg).toContain('rx="2"');
    expect(svg).toContain('fill="#161B22"');
    expect(svg).toContain("Less");
    expect(svg).toContain("More");
  });

  it("renders day labels (Mon, Wed, Fri)", () => {
    const svg = renderProfileEmbedSvg(mockStats, { contributions: mockContributions });

    expect(svg).toContain(">Mon<");
    expect(svg).toContain(">Wed<");
    expect(svg).toContain(">Fri<");
  });

  it("renders month labels", () => {
    const svg = renderProfileEmbedSvg(mockStats, { contributions: mockContributions });

    expect(svg).toContain(">Jan<");
  });

  it("ignores contributions in compact mode", () => {
    const svg = renderProfileEmbedSvg(mockStats, { compact: true, contributions: mockContributions });

    expect(svg).toContain('height="162"');
    expect(svg).not.toContain("Less");
    expect(svg).not.toContain("More");
  });

  it("uses light theme graph colors", () => {
    const svg = renderProfileEmbedSvg(mockStats, { theme: "light", contributions: mockContributions });

    expect(svg).toContain('fill="#EBEDF0"');
  });

  it("does not render graph when contributions is null", () => {
    const svg = renderProfileEmbedSvg(mockStats, { contributions: null });

    expect(svg).toContain('height="186"');
    expect(svg).not.toContain("Less");
  });
});

describe("renderProfileEmbedErrorSvg", () => {
  it("renders safe fallback SVG", () => {
    const svg = renderProfileEmbedErrorSvg("User <unknown>", { theme: "light" });

    expect(svg).toContain("Tokscale Stats");
    expect(svg).toContain("User &lt;unknown&gt;");
    expect(svg).not.toContain("User <unknown>");
    expect(svg).toContain("family=Figtree");
    expect(svg).toContain('id="err-bg"');
  });
});
