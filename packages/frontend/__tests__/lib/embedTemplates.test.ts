import { describe, expect, it } from "vitest";
import type {
  UserEmbedStats,
  EmbedContributionDay,
} from "../../src/lib/embed/getUserEmbedStats";
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
    for (const template of [
      "classic",
      "minimal",
      "terminal",
      "graph",
      "orbit",
      "vitals",
      "blueprint",
      "receipt",
    ] as const) {
      expect(parseEmbedTemplate(template)).toBe(template);
    }
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
    const svg = renderMinimalEmbedSvg(mockStats, {
      contributions: mockContributions,
    });
    expect(svg).toContain("<svg");
    expect(svg).toContain("@octocat");
    expect(svg).toContain("TOTAL TOKENS");
  });

  it("honors the token number format", () => {
    expect(
      renderMinimalEmbedSvg(mockStats, { tokensFormat: "full" }),
    ).toContain("1,234,567");
    expect(
      renderMinimalEmbedSvg(mockStats, { tokensFormat: "compact" }),
    ).toContain("1.2M");
  });
});

describe("renderTerminalEmbedSvg", () => {
  it("uses a literal command and monospace result grammar", () => {
    const svg = renderTerminalEmbedSvg(mockStats, {
      contributions: mockContributions,
    });
    expect(svg).toContain("<svg");
    expect(svg).toContain("@octocat");
    expect(svg).toContain(
      'data-terminal-command="tokscale profile @octocat --sort tokens"',
    );
    expect(svg).toContain(">tokens<");
    expect(svg).toContain("ui-monospace");
    expect(svg).not.toContain("#FF5F56");
  });
});

describe("renderGraphEmbedSvg", () => {
  it("renders the contribution graph as the hero with labels and legend", () => {
    const svg = renderGraphEmbedSvg(mockStats, {
      contributions: mockContributions,
    });
    expect(svg).toContain("<svg");
    expect(svg).toContain("@octocat");
    expect(svg).toContain(">Mon<");
    expect(svg).toContain(">Fri<");
    expect(svg).toContain("Less");
    expect(svg).toContain("More");
    expect(svg).toContain("active days");
  });

  it("recolors the graph when a color is selected", () => {
    const svg = renderGraphEmbedSvg(mockStats, {
      contributions: mockContributions,
      color: "purple",
    });
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

const allTemplates = {
  minimal: renderMinimalEmbedSvg,
  terminal: renderTerminalEmbedSvg,
  graph: renderGraphEmbedSvg,
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
      const svg = render(
        { ...mockStats, user: { ...mockStats.user, username: "a<b&c" } },
        {},
      );
      expect(svg).toContain("<svg");
      expect(svg).not.toContain("a<b&c");
    });

    it(`${name} accepts a color override`, () => {
      expect(
        render(mockStats, {
          contributions: mockContributions,
          color: "purple",
        }),
      ).toContain("<svg");
    });
  }
});

describe("restrained embed design language", () => {
  for (const [name, render] of Object.entries(allTemplates)) {
    it(`${name} uses one solid card surface without decorative effects`, () => {
      const svg = render(mockStats, {
        contributions: mockContributions,
        graph: true,
      });

      expect(svg).toContain(`data-template="${name}"`);
      expect(svg).not.toContain("<linearGradient");
      expect(svg).not.toContain("<radialGradient");
      expect(svg).not.toContain("<pattern");
      expect(svg).not.toContain("filter=");
    });
  }

  it("removes the former orbit, blueprint, and receipt cosplay motifs", () => {
    const orbit = renderOrbitEmbedSvg(mockStats, {
      contributions: mockContributions,
    });
    const blueprint = renderBlueprintEmbedSvg(mockStats, {
      contributions: mockContributions,
    });
    const receipt = renderReceiptEmbedSvg(mockStats, {
      contributions: mockContributions,
    });

    expect(orbit).not.toContain("<path");
    expect(blueprint).not.toContain('id="grid"');
    expect(blueprint).not.toContain("registration");
    expect(receipt).not.toContain("barcode");
    expect(receipt).not.toContain("THANK YOU FOR VIBE CODING");
  });

  it("uses product identity instead of template-taxonomy overlines", () => {
    const rendered = Object.values(allTemplates).map((render) =>
      render(mockStats, { contributions: mockContributions }),
    );

    for (const svg of rendered) {
      expect(svg).not.toContain("TOKEN FOCUS");
      expect(svg).not.toContain("DETAILED STATS");
      expect(svg).not.toContain("COMPACT LIST");
      expect(svg).not.toContain("ACTIVITY SUMMARY");
      expect(svg).not.toContain("RANK FOCUS");
      expect(svg).not.toContain("usage.readout");
    }
  });

  it("makes the detailed template expose facts beyond the overview", () => {
    const svg = renderBlueprintEmbedSvg(mockStats, {
      contributions: mockContributions,
    });

    expect(svg).toContain('data-detail="submissions"');
    expect(svg).toContain('data-detail="active-days"');
    expect(svg).toContain('data-detail="peak-day"');
    expect(svg).toContain('data-detail="year-tokens"');
  });

  it("uses one semantic rank color across detailed and ledger layouts", () => {
    expect(renderBlueprintEmbedSvg(mockStats)).toContain(
      `fill="${THEMES.dark.rankBronze}"`,
    );
    expect(renderReceiptEmbedSvg(mockStats)).toContain(
      `fill="${THEMES.dark.rankBronze}"`,
    );
  });

  it("keeps the established template widths", () => {
    expect(renderMinimalEmbedSvg(mockStats)).toContain('width="600"');
    expect(renderTerminalEmbedSvg(mockStats)).toContain('width="600"');
    expect(renderGraphEmbedSvg(mockStats)).toContain('width="680"');
    expect(renderOrbitEmbedSvg(mockStats)).toContain('width="560"');
    expect(renderVitalsEmbedSvg(mockStats)).toContain('width="520"');
    expect(renderBlueprintEmbedSvg(mockStats)).toContain('width="640"');
    expect(renderReceiptEmbedSvg(mockStats)).toContain('width="400"');
  });

  it("keeps calendar-first graph labels legible at README width", () => {
    const svg = renderGraphEmbedSvg(mockStats, {
      contributions: mockContributions,
    });

    expect(svg).not.toContain('font-size="9"');
  });
});

describe("graph toggle", () => {
  it("omits the contribution graph by default for every template", () => {
    expect(
      renderMinimalEmbedSvg(mockStats, { contributions: mockContributions }),
    ).not.toContain("Contribution activity");
    expect(
      renderTerminalEmbedSvg(mockStats, { contributions: mockContributions }),
    ).not.toContain("Contribution activity");
    expect(
      renderBlueprintEmbedSvg(mockStats, { contributions: mockContributions }),
    ).not.toContain("Contribution activity");
    expect(
      renderOrbitEmbedSvg(mockStats, { contributions: mockContributions }),
    ).not.toContain("Contribution activity");
    expect(
      renderReceiptEmbedSvg(mockStats, { contributions: mockContributions }),
    ).not.toContain("Contribution activity");
  });

  it("appends the contribution graph when graph is true", () => {
    expect(
      renderMinimalEmbedSvg(mockStats, {
        contributions: mockContributions,
        graph: true,
      }),
    ).toContain("Contribution activity");
    expect(
      renderTerminalEmbedSvg(mockStats, {
        contributions: mockContributions,
        graph: true,
      }),
    ).toContain("Contribution activity");
    expect(
      renderBlueprintEmbedSvg(mockStats, {
        contributions: mockContributions,
        graph: true,
      }),
    ).toContain("Contribution activity");
    expect(
      renderOrbitEmbedSvg(mockStats, {
        contributions: mockContributions,
        graph: true,
      }),
    ).toContain("Contribution activity");
    expect(
      renderReceiptEmbedSvg(mockStats, {
        contributions: mockContributions,
        graph: true,
      }),
    ).toContain("Contribution activity");
  });

  it("reserves footer space below every optional contribution panel", () => {
    const svgs = [
      renderMinimalEmbedSvg(mockStats, {
        contributions: mockContributions,
        graph: true,
      }),
      renderTerminalEmbedSvg(mockStats, {
        contributions: mockContributions,
        graph: true,
      }),
      renderBlueprintEmbedSvg(mockStats, {
        contributions: mockContributions,
        graph: true,
      }),
      renderOrbitEmbedSvg(mockStats, {
        contributions: mockContributions,
        graph: true,
      }),
      renderReceiptEmbedSvg(mockStats, {
        contributions: mockContributions,
        graph: true,
      }),
      renderGraphEmbedSvg(mockStats, { contributions: mockContributions }),
    ];

    for (const svg of svgs) {
      const height = Number(svg.match(/<svg[^>]*height="([\d.]+)"/)?.[1]);
      const panelBottom = Number(svg.match(/data-bottom="([\d.]+)"/)?.[1]);
      const footerY = Number(svg.match(/data-card-footer-y="([\d.]+)"/)?.[1]);

      expect(footerY).toBeGreaterThanOrEqual(panelBottom + 12);
      expect(height).toBeGreaterThan(footerY);
    }
  });
});
