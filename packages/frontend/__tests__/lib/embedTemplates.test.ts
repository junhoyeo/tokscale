import { describe, expect, it } from "vitest";
import type { UserEmbedStats, EmbedContributionDay } from "../../src/lib/embed/getUserEmbedStats";
import {
  THEMES,
  applyEmbedColor,
  parseEmbedTemplate,
  parseEmbedColor,
  parseNumberFormat,
  parseRankFormat,
  formatRank,
} from "../../src/lib/embed/embedShared";
import { renderMinimalEmbedSvg } from "../../src/lib/embed/renderMinimalEmbedSvg";
import { renderTerminalEmbedSvg } from "../../src/lib/embed/renderTerminalEmbedSvg";
import { renderGraphEmbedSvg } from "../../src/lib/embed/renderGraphEmbedSvg";
import { renderOrbitEmbedSvg } from "../../src/lib/embed/renderOrbitEmbedSvg";
import { renderVitalsEmbedSvg } from "../../src/lib/embed/renderVitalsEmbedSvg";
import { renderBlueprintEmbedSvg } from "../../src/lib/embed/renderBlueprintEmbedSvg";
import { renderReceiptEmbedSvg } from "../../src/lib/embed/renderReceiptEmbedSvg";

const mockStats: UserEmbedStats = {
  user: { id: "user-id", username: "octocat", displayName: "The Octocat", avatarUrl: null },
  stats: {
    totalTokens: 1234567,
    totalCost: 42.42,
    submissionCount: 7,
    rank: 3,
    rankTotal: 80,
    updatedAt: "2026-02-24T00:00:00.000Z",
  },
};

const mockContributions: EmbedContributionDay[] = [
  { date: "2026-01-15", totalTokens: 0, totalCost: 0, intensity: 0 },
  { date: "2026-02-10", totalTokens: 5000, totalCost: 2, intensity: 2 },
  { date: "2026-02-20", totalTokens: 99999, totalCost: 40, intensity: 4 },
];

describe("parseEmbedTemplate", () => {
  it("accepts known templates", () => {
    expect(parseEmbedTemplate("minimal")).toBe("minimal");
    expect(parseEmbedTemplate("terminal")).toBe("terminal");
    expect(parseEmbedTemplate("graph")).toBe("graph");
    expect(parseEmbedTemplate("classic")).toBe("classic");
  });

  it("falls back to classic for unknown or missing values", () => {
    expect(parseEmbedTemplate("fancy")).toBe("classic");
    expect(parseEmbedTemplate(null)).toBe("classic");
  });
});

describe("parseEmbedColor", () => {
  it("accepts known palette names and rejects others", () => {
    expect(parseEmbedColor("purple")).toBe("purple");
    expect(parseEmbedColor("blue")).toBe("blue");
    expect(parseEmbedColor("not-a-color")).toBeNull();
    expect(parseEmbedColor(null)).toBeNull();
  });
});

describe("parseNumberFormat", () => {
  it("parses full and compact, undefined otherwise", () => {
    expect(parseNumberFormat("full")).toBe("full");
    expect(parseNumberFormat("compact")).toBe("compact");
    expect(parseNumberFormat("huge")).toBeUndefined();
    expect(parseNumberFormat(null)).toBeUndefined();
  });
});

describe("parseRankFormat", () => {
  it("parses plain, percent, and total, undefined otherwise", () => {
    expect(parseRankFormat("plain")).toBe("plain");
    expect(parseRankFormat("percent")).toBe("percent");
    expect(parseRankFormat("total")).toBe("total");
    expect(parseRankFormat("nope")).toBeUndefined();
    expect(parseRankFormat(null)).toBeUndefined();
  });
});

describe("formatRank", () => {
  it("formats plain as #rank", () => {
    expect(formatRank(134, 1174, "plain")).toBe("#134");
    expect(formatRank(134, 1174)).toBe("#134");
  });

  it("formats percent as a ceil-ed top N%", () => {
    expect(formatRank(134, 1174, "percent")).toBe("top 12%");
    expect(formatRank(1, 1000, "percent")).toBe("top 1%");
  });

  it("formats total as #rank / total with grouping", () => {
    expect(formatRank(134, 1174, "total")).toBe("#134 / 1,174");
  });

  it("falls back to #rank when total is missing or zero", () => {
    expect(formatRank(134, null, "percent")).toBe("#134");
    expect(formatRank(134, 0, "total")).toBe("#134");
  });
});

describe("applyEmbedColor", () => {
  it("overrides the graph grades with the named palette", () => {
    const purple = applyEmbedColor(THEMES.dark, "purple");
    expect(purple.graphGrade4).toBe("#6e40c9");
    expect(purple.graphGrade1).toBe("#cdb4ff");
    expect(purple.graphGrade0).toBe(THEMES.dark.graphGrade0);
  });

  it("returns the palette unchanged when no color is given", () => {
    expect(applyEmbedColor(THEMES.dark, null)).toBe(THEMES.dark);
  });
});

describe("renderMinimalEmbedSvg", () => {
  it("renders an SVG with the username and token hero", () => {
    const svg = renderMinimalEmbedSvg(mockStats, { contributions: mockContributions });
    expect(svg).toContain("<svg");
    expect(svg).toContain("@octocat");
    expect(svg).toContain("TOTAL TOKENS");
  });

  it("honors the token number format", () => {
    expect(renderMinimalEmbedSvg(mockStats, { tokensFormat: "full" })).toContain("1,234,567");
    expect(renderMinimalEmbedSvg(mockStats, { tokensFormat: "compact" })).toContain("1.2M");
  });
});

describe("renderTerminalEmbedSvg", () => {
  it("renders a terminal window with a prompt and stats", () => {
    const svg = renderTerminalEmbedSvg(mockStats, { contributions: mockContributions });
    expect(svg).toContain("<svg");
    expect(svg).toContain("tokscale — @octocat");
    expect(svg).toContain("tokscale stats");
    expect(svg).toContain("ui-monospace");
  });
});

describe("renderGraphEmbedSvg", () => {
  it("renders the contribution graph as the hero with labels and legend", () => {
    const svg = renderGraphEmbedSvg(mockStats, { contributions: mockContributions });
    expect(svg).toContain("<svg");
    expect(svg).toContain("@octocat");
    expect(svg).toContain(">Mon<");
    expect(svg).toContain(">Fri<");
    expect(svg).toContain("Less");
    expect(svg).toContain("More");
    expect(svg).toContain("active days");
  });

  it("recolors the graph when a color is selected", () => {
    const svg = renderGraphEmbedSvg(mockStats, { contributions: mockContributions, color: "purple" });
    expect(svg).toContain("#6e40c9");
    expect(svg).not.toContain("#39D353");
  });
});

describe("template renderers escape user input", () => {
  it("escapes XML in the username", () => {
    const evil: UserEmbedStats = {
      ...mockStats,
      user: { ...mockStats.user, username: "a<b&c" },
    };
    for (const svg of [
      renderMinimalEmbedSvg(evil),
      renderTerminalEmbedSvg(evil),
      renderGraphEmbedSvg(evil),
    ]) {
      expect(svg).toContain("a&lt;b&amp;c");
      expect(svg).not.toContain("a<b&c");
    }
  });
});

const batchOne = {
  orbit: renderOrbitEmbedSvg,
  vitals: renderVitalsEmbedSvg,
  blueprint: renderBlueprintEmbedSvg,
  receipt: renderReceiptEmbedSvg,
};

describe("batch-1 template renderers", () => {
  for (const [name, render] of Object.entries(batchOne)) {
    it(`${name} renders a well-formed SVG`, () => {
      const svg = render(mockStats, { contributions: mockContributions });
      expect(svg).toContain("<svg");
      expect(svg.trimEnd().endsWith("</svg>")).toBe(true);
      const stripped = svg.replace(/&(amp|lt|gt|quot|apos|#\d+);/g, "");
      expect(stripped).not.toContain("&");
    });

    it(`${name} escapes the username and survives missing contributions`, () => {
      const svg = render({ ...mockStats, user: { ...mockStats.user, username: "a<b&c" } }, {});
      expect(svg).toContain("<svg");
      expect(svg).not.toContain("a<b&c");
    });

    it(`${name} accepts a color override`, () => {
      expect(render(mockStats, { contributions: mockContributions, color: "purple" })).toContain("<svg");
    });
  }
});

describe("graph toggle", () => {
  it("omits the contribution graph by default for every template", () => {
    expect(renderMinimalEmbedSvg(mockStats, { contributions: mockContributions }))
      .not.toContain("Daily token activity");
    expect(renderTerminalEmbedSvg(mockStats, { contributions: mockContributions }))
      .not.toContain("contribution graph");
    expect(renderBlueprintEmbedSvg(mockStats, { contributions: mockContributions }))
      .not.toContain("ACTIVITY PROFILE");
    expect(renderOrbitEmbedSvg(mockStats, { contributions: mockContributions }))
      .not.toContain("Daily token activity");
    expect(renderReceiptEmbedSvg(mockStats, { contributions: mockContributions }))
      .not.toContain("DAILY ACTIVITY");
  });

  it("appends the contribution graph when graph is true", () => {
    expect(renderMinimalEmbedSvg(mockStats, { contributions: mockContributions, graph: true }))
      .toContain("Daily token activity");
    expect(renderTerminalEmbedSvg(mockStats, { contributions: mockContributions, graph: true }))
      .toContain("contribution graph");
    expect(renderBlueprintEmbedSvg(mockStats, { contributions: mockContributions, graph: true }))
      .toContain("ACTIVITY PROFILE");
    expect(renderOrbitEmbedSvg(mockStats, { contributions: mockContributions, graph: true }))
      .toContain("Daily token activity");
    expect(renderReceiptEmbedSvg(mockStats, { contributions: mockContributions, graph: true }))
      .toContain("DAILY ACTIVITY");
  });
});
