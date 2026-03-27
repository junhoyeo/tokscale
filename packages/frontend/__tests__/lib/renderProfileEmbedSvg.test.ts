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
    expect(svg).toContain("README EMBED");
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
    expect(svg).toContain("README EMBED");
    expect(svg).toContain("@octocat");
    expect(svg).toContain('stop-color="#F6FAFF"');
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

  it("uses gradient tokens, green cost, and medal rank colors", () => {
    const svg = renderProfileEmbedSvg(mockStats);

    expect(svg).toContain('id="tokens-gradient"');
    expect(svg).toContain('fill="url(#tokens-gradient)"');
    expect(svg).toContain('fill="#53D18C"');
    expect(svg).toContain('fill="#D97706"');
  });

  it("uses gold color for rank #1", () => {
    const svg = renderProfileEmbedSvg({
      ...mockStats,
      stats: { ...mockStats.stats, rank: 1 },
    });
    expect(svg).toContain('fill="#EAB308"');
  });

  it("renders branded gradient surfaces for the refreshed card", () => {
    const svg = renderProfileEmbedSvg(mockStats);

    expect(svg).toContain('id="card-bg"');
    expect(svg).toContain('id="shell-bg"');
    expect(svg).toContain('id="header-bg"');
    expect(svg).toContain('id="metric-sheen"');
    expect(svg).toContain('filter="url(#soft-glow)"');
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
    expect(x).toBeGreaterThanOrEqual(20 + 18 + 8 * 9 + 8);
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
});

describe("renderProfileEmbedErrorSvg", () => {
  it("renders safe fallback SVG", () => {
    const svg = renderProfileEmbedErrorSvg("User <unknown>", { theme: "light" });

    expect(svg).toContain("Tokscale Stats");
    expect(svg).toContain("README EMBED");
    expect(svg).toContain("User &lt;unknown&gt;");
    expect(svg).not.toContain("User <unknown>");
    expect(svg).toContain("family=Figtree");
    expect(svg).toContain('id="error-bg"');
  });
});
