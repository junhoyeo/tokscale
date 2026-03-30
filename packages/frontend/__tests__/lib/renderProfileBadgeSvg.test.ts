import { describe, expect, it } from "vitest";
import {
  renderProfileBadgeSvg,
  renderBadgeErrorSvg,
} from "../../src/lib/embed/renderProfileBadgeSvg";
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

describe("renderProfileBadgeSvg", () => {
  it("renders a valid SVG with default metric (tokens)", () => {
    const svg = renderProfileBadgeSvg(mockStats);

    expect(svg).toContain("<svg");
    expect(svg).toContain("</svg>");
    expect(svg).toContain("Tokscale Tokens");
    expect(svg).toContain("1,234,567");
    expect(svg).toContain('height="20"');
  });

  it("renders cost metric", () => {
    const svg = renderProfileBadgeSvg(mockStats, { metric: "cost" });

    expect(svg).toContain("Tokscale Cost");
    expect(svg).toContain("$42.42");
    expect(svg).toContain("#16804B");
  });

  it("renders rank metric", () => {
    const svg = renderProfileBadgeSvg(mockStats, { metric: "rank" });

    expect(svg).toContain("Tokscale Rank");
    expect(svg).toContain("#3");
    expect(svg).toContain("#D97706");
  });

  it("renders rank N/A when rank is null", () => {
    const svg = renderProfileBadgeSvg(
      { ...mockStats, stats: { ...mockStats.stats, rank: null } },
      { metric: "rank" },
    );

    expect(svg).toContain("N/A");
  });

  it("renders flat style by default with gradient and rounded clipPath", () => {
    const svg = renderProfileBadgeSvg(mockStats);

    expect(svg).toContain('id="s"');
    expect(svg).toContain('rx="3"');
    expect(svg).toContain("clip-path");
    expect(svg).toContain("linearGradient");
  });

  it("renders flat-square style without gradient or rounded corners", () => {
    const svg = renderProfileBadgeSvg(mockStats, { style: "flat-square" });

    expect(svg).not.toContain("linearGradient");
    expect(svg).not.toContain("clip-path");
    expect(svg).toContain('shape-rendering="crispEdges"');
  });

  it("flat style includes text shadow layer", () => {
    const svg = renderProfileBadgeSvg(mockStats);

    expect(svg).toContain('fill="#010101"');
    expect(svg).toContain('fill-opacity=".3"');
  });

  it("flat-square style has no text shadow layer", () => {
    const svg = renderProfileBadgeSvg(mockStats, { style: "flat-square" });

    expect(svg).not.toContain('fill="#010101"');
    expect(svg).not.toContain('fill-opacity=".3"');
  });

  it("accepts custom label", () => {
    const svg = renderProfileBadgeSvg(mockStats, { label: "my tokens" });

    expect(svg).toContain("my tokens");
    expect(svg).not.toContain("Tokscale Tokens");
  });

  it("accepts custom color (hex without #)", () => {
    const svg = renderProfileBadgeSvg(mockStats, { color: "ff5733" });

    expect(svg).toContain("#ff5733");
  });

  it("accepts custom color (hex with #)", () => {
    const svg = renderProfileBadgeSvg(mockStats, { color: "#AABBCC" });

    expect(svg).toContain("#AABBCC");
  });

  it("falls back to default color on invalid hex", () => {
    const svg = renderProfileBadgeSvg(mockStats, { color: "not-a-color" });

    expect(svg).toContain("#0073FF");
  });

  it("rejects invalid hex lengths (5/7 digits)", () => {
    expect(renderProfileBadgeSvg(mockStats, { color: "12345" })).toContain("#0073FF");
    expect(renderProfileBadgeSvg(mockStats, { color: "1234567" })).toContain("#0073FF");
  });

  it("truncates excessively long labels", () => {
    const longLabel = "A".repeat(80);
    const svg = renderProfileBadgeSvg(mockStats, { label: longLabel });

    expect(svg).not.toContain(longLabel);
    expect(svg).toContain("A".repeat(40));
  });

  it("renders 0 for NaN/Infinity token values", () => {
    const nanSvg = renderProfileBadgeSvg(
      { ...mockStats, stats: { ...mockStats.stats, totalTokens: NaN } },
    );
    const infSvg = renderProfileBadgeSvg(
      { ...mockStats, stats: { ...mockStats.stats, totalTokens: Infinity } },
    );

    expect(nanSvg).toContain(">0<");
    expect(infSvg).toContain(">0<");
  });

  it("escapes XML in label and value", () => {
    const svg = renderProfileBadgeSvg(mockStats, { label: '<script>alert("xss")</script>' });

    expect(svg).toContain("&lt;script&gt;");
    expect(svg).not.toContain("<script>");
  });

  it("does not contain raw & outside XML entities", () => {
    const svg = renderProfileBadgeSvg(mockStats);

    const stripped = svg.replace(/&(amp|lt|gt|quot|apos|#\d+|#x[0-9a-fA-F]+);/g, "");
    expect(stripped).not.toContain("&");
  });

  it("includes accessible title element", () => {
    const svg = renderProfileBadgeSvg(mockStats);

    expect(svg).toContain("<title>");
    expect(svg).toContain("Tokscale Tokens: 1,234,567");
  });

  it("includes aria-label attribute", () => {
    const svg = renderProfileBadgeSvg(mockStats);

    expect(svg).toContain('aria-label="Tokscale Tokens: 1,234,567"');
  });

  it("shows full numbers by default (compact off)", () => {
    const svg = renderProfileBadgeSvg(
      { ...mockStats, stats: { ...mockStats.stats, totalTokens: 1234567 } },
      { metric: "tokens" },
    );

    expect(svg).toContain("1,234,567");
    expect(svg).not.toContain("1.2M");
  });

  it("shows full currency by default (compact off)", () => {
    const svg = renderProfileBadgeSvg(
      { ...mockStats, stats: { ...mockStats.stats, totalCost: 1500 } },
      { metric: "cost" },
    );

    expect(svg).toContain("$1,500.00");
  });

  it("formats tokens in compact notation when compact=true", () => {
    const cases: [number, string][] = [
      [500, "500"],
      [1500, "1.5K"],
      [1234567, "1.2M"],
      [2_500_000_000, "2.5B"],
    ];

    for (const [tokens, expected] of cases) {
      const svg = renderProfileBadgeSvg(
        { ...mockStats, stats: { ...mockStats.stats, totalTokens: tokens } },
        { metric: "tokens", compact: true },
      );
      expect(svg).toContain(expected);
    }
  });

  it("formats cost in compact notation when compact=true", () => {
    const cases: [number, string][] = [
      [42.42, "$42.42"],
      [1500, "$1.5K"],
      [2_500_000, "$2.5M"],
    ];

    for (const [cost, expected] of cases) {
      const svg = renderProfileBadgeSvg(
        { ...mockStats, stats: { ...mockStats.stats, totalCost: cost } },
        { metric: "cost", compact: true },
      );
      expect(svg).toContain(expected);
    }
  });

  it("defaults to tokens when metric is invalid", () => {
    const svg = renderProfileBadgeSvg(mockStats, { metric: "invalid" as never });

    expect(svg).toContain("Tokscale Tokens");
    expect(svg).toContain("1,234,567");
  });

  it("uses Verdana font family", () => {
    const svg = renderProfileBadgeSvg(mockStats);

    expect(svg).toContain("Verdana");
  });
});

describe("renderBadgeErrorSvg", () => {
  it("renders error badge with red value section", () => {
    const svg = renderBadgeErrorSvg("not found");

    expect(svg).toContain("<svg");
    expect(svg).toContain("Tokscale");
    expect(svg).toContain("not found");
    expect(svg).toContain("#e05d44");
  });

  it("renders flat-square error badge", () => {
    const svg = renderBadgeErrorSvg("error", { style: "flat-square" });

    expect(svg).toContain('shape-rendering="crispEdges"');
    expect(svg).not.toContain("linearGradient");
  });

  it("supports custom label for error badge", () => {
    const svg = renderBadgeErrorSvg("timeout", { label: "my-badge" });

    expect(svg).toContain("my-badge");
    expect(svg).not.toContain("Tokscale");
  });

  it("escapes XML in error message", () => {
    const svg = renderBadgeErrorSvg("user <unknown>");

    expect(svg).toContain("user &lt;unknown&gt;");
    expect(svg).not.toContain("user <unknown>");
  });

  it("truncates excessively long labels in error badges", () => {
    const longLabel = "B".repeat(80);
    const svg = renderBadgeErrorSvg("error", { label: longLabel });

    expect(svg).not.toContain(longLabel);
    expect(svg).toContain("B".repeat(40));
  });
});
