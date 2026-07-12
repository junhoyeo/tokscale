import { afterAll, beforeAll, describe, expect, it, vi } from "vitest";
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
  fittedText,
  formatRank,
} from "../../src/lib/embed/embedShared";
import { renderMinimalEmbedSvg } from "../../src/lib/embed/renderMinimalEmbedSvg";
import { renderTerminalEmbedSvg } from "../../src/lib/embed/renderTerminalEmbedSvg";
import { renderGraphEmbedSvg } from "../../src/lib/embed/renderGraphEmbedSvg";
import { renderOrbitEmbedSvg } from "../../src/lib/embed/renderOrbitEmbedSvg";
import { renderVitalsEmbedSvg } from "../../src/lib/embed/renderVitalsEmbedSvg";
import { renderBlueprintEmbedSvg } from "../../src/lib/embed/renderBlueprintEmbedSvg";
import { renderReceiptEmbedSvg } from "../../src/lib/embed/renderReceiptEmbedSvg";
import {
  renderProfileEmbedErrorSvg,
  renderProfileEmbedSvg,
} from "../../src/lib/embed/renderProfileEmbedSvg";
import { renderIsometric3DEmbedSvg } from "../../src/lib/embed/renderIsometric3DSvg";
import { formatCurrency, formatNumber } from "../../src/lib/format";
import { colorPalettes, getDarkGradeColors } from "../../src/lib/themes";

const FROZEN_NOW = new Date("2026-02-24T12:00:00.000Z");

beforeAll(() => {
  vi.useFakeTimers();
  vi.setSystemTime(FROZEN_NOW);
});

afterAll(() => {
  vi.useRealTimers();
});

function attributes(source: string): Record<string, string> {
  return Object.fromEntries(
    [...source.matchAll(/([\w:-]+)="([^"]*)"/g)].map((match) => [
      match[1],
      match[2],
    ]),
  );
}

function rootAttributes(svg: string): Record<string, string> {
  const root = svg.match(/<svg\b([^>]*)>/);
  expect(root, "renderer must emit an SVG root").not.toBeNull();
  return attributes(root?.[1] ?? "");
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

function renderedTextWidth(node: ReturnType<typeof textNodes>[number]): number {
  if (node.attrs.textLength) return Number(node.attrs.textLength);

  const fontSize = Number(node.attrs["font-size"]);
  const glyphUnits = [...node.text].reduce(
    (width, character) =>
      width +
      (/\p{Script=Han}|\p{Script=Hangul}|\p{Script=Hiragana}|\p{Script=Katakana}/u.test(
        character,
      )
        ? 1
        : 0.6),
    0,
  );
  return glyphUnits * fontSize;
}

function isometricCubeGeometry(svg: string): string[] {
  return [
    ...svg.matchAll(/<g transform="translate\(([^"]+)\)">([\s\S]*?)<\/g>/g),
  ]
    .filter((match) => /class="[dl]\d-t"/.test(match[2]))
    .map((match) => {
      const faceHeight = match[2].match(
        /height="([^"]+)"[^>]*class="[dl]\d-l"/,
      )?.[1];
      return `${match[1]}:${faceHeight}`;
    });
}

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
    const grades = getDarkGradeColors(colorPalettes.purple);
    expect([
      purple.graphGrade1,
      purple.graphGrade2,
      purple.graphGrade3,
      purple.graphGrade4,
    ]).toEqual(grades);
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
    expect(svg).toContain("Total tokens");
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
  it("uses monospace result grammar without inventing a profile command", () => {
    const svg = renderTerminalEmbedSvg(mockStats, {
      contributions: mockContributions,
    });
    expect(svg).toContain("<svg");
    expect(svg).toContain("@octocat");
    expect(svg).not.toContain("tokscale profile");
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
    const purple = getDarkGradeColors(colorPalettes.purple);
    expect(svg).toContain(purple[0]);
    expect(svg).toContain(purple[3]);
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
  classic: renderProfileEmbedSvg,
  minimal: renderMinimalEmbedSvg,
  terminal: renderTerminalEmbedSvg,
  graph: renderGraphEmbedSvg,
  orbit: renderOrbitEmbedSvg,
  vitals: renderVitalsEmbedSvg,
  blueprint: renderBlueprintEmbedSvg,
  receipt: renderReceiptEmbedSvg,
};

const ariaLabels: Record<keyof typeof allTemplates, string> = {
  classic: "Tokscale profile stats for @octocat",
  minimal: "Tokscale stats for @octocat",
  terminal: "Tokscale stats for @octocat",
  graph: "Tokscale contribution graph for @octocat",
  orbit: "Tokscale leaderboard standing for @octocat",
  vitals: "Tokscale usage signals for @octocat",
  blueprint: "Tokscale usage data sheet for @octocat",
  receipt: "Tokscale compact ledger for @octocat",
};

const graphOptions = { contributions: mockContributions, graph: true } as const;
const rendererRootCases: Array<
  [string, keyof typeof allTemplates, number, number, () => string]
> = [
  ["classic", "classic", 680, 186, () => renderProfileEmbedSvg(mockStats)],
  [
    "classic compact",
    "classic",
    460,
    162,
    () => renderProfileEmbedSvg(mockStats, { ...graphOptions, compact: true }),
  ],
  [
    "classic with graph",
    "classic",
    680,
    336,
    () => renderProfileEmbedSvg(mockStats, graphOptions),
  ],
  ["minimal", "minimal", 600, 162, () => renderMinimalEmbedSvg(mockStats)],
  [
    "minimal with graph",
    "minimal",
    600,
    301,
    () => renderMinimalEmbedSvg(mockStats, graphOptions),
  ],
  ["terminal", "terminal", 600, 176, () => renderTerminalEmbedSvg(mockStats)],
  [
    "terminal with graph",
    "terminal",
    600,
    312,
    () => renderTerminalEmbedSvg(mockStats, graphOptions),
  ],
  [
    "graph",
    "graph",
    680,
    240,
    () => renderGraphEmbedSvg(mockStats, graphOptions),
  ],
  ["orbit", "orbit", 560, 248, () => renderOrbitEmbedSvg(mockStats)],
  [
    "orbit with graph",
    "orbit",
    560,
    384,
    () => renderOrbitEmbedSvg(mockStats, graphOptions),
  ],
  [
    "vitals",
    "vitals",
    520,
    250,
    () => renderVitalsEmbedSvg(mockStats, graphOptions),
  ],
  [
    "blueprint",
    "blueprint",
    640,
    232,
    () => renderBlueprintEmbedSvg(mockStats),
  ],
  [
    "blueprint with graph",
    "blueprint",
    640,
    367,
    () => renderBlueprintEmbedSvg(mockStats, graphOptions),
  ],
  ["receipt", "receipt", 400, 250, () => renderReceiptEmbedSvg(mockStats)],
  [
    "receipt with graph",
    "receipt",
    400,
    352,
    () => renderReceiptEmbedSvg(mockStats, graphOptions),
  ],
];

describe("embed renderer root contracts", () => {
  it.each(rendererRootCases)(
    "%s preserves its intrinsic SVG and accessibility metadata",
    (_name, template, width, height, render) => {
      const attrs = rootAttributes(render());

      expect(attrs).toMatchObject({
        "data-template": template,
        width: String(width),
        height: String(height),
        viewBox: `0 0 ${width} ${height}`,
        role: "img",
        "aria-label": ariaLabels[template],
      });
    },
  );
});

describe("embed text fitting", () => {
  const username = "u".repeat(39);
  const extremeStats: UserEmbedStats = {
    user: {
      ...mockStats.user,
      username,
      displayName: "A deliberately long production display name",
    },
    stats: {
      ...mockStats.stats,
      totalTokens: Number.MAX_SAFE_INTEGER,
      totalCost: 9_876_543_210.98,
      rank: 987_654,
      rankTotal: 9_876_543,
    },
  };
  const fullOptions = {
    tokensFormat: "full" as const,
    costFormat: "full" as const,
    rankFormat: "total" as const,
  };
  const expectedText = [
    `@${username}`,
    formatNumber(extremeStats.stats.totalTokens),
    formatCurrency(extremeStats.stats.totalCost),
    formatRank(
      extremeStats.stats.rank ?? 0,
      extremeStats.stats.rankTotal ?? null,
      "total",
    ),
  ];
  const renderers = {
    classic: () => renderProfileEmbedSvg(extremeStats, fullOptions),
    minimal: () => renderMinimalEmbedSvg(extremeStats, fullOptions),
    terminal: () => renderTerminalEmbedSvg(extremeStats, fullOptions),
    graph: () => renderGraphEmbedSvg(extremeStats, fullOptions),
    orbit: () => renderOrbitEmbedSvg(extremeStats, fullOptions),
    vitals: () => renderVitalsEmbedSvg(extremeStats, fullOptions),
    blueprint: () => renderBlueprintEmbedSvg(extremeStats, fullOptions),
    receipt: () => renderReceiptEmbedSvg(extremeStats, fullOptions),
  };

  it.each(Object.entries(renderers))(
    "%s exposes a bounded fit budget for long identity and full metric values",
    (_name, render) => {
      const nodes = textNodes(render());

      for (const expected of expectedText) {
        const candidates = nodes.filter((node) => node.text.includes(expected));
        expect(
          candidates.length,
          `missing visible text: ${expected}`,
        ).toBeGreaterThan(0);

        const fitted = candidates.find(
          (node) => node.attrs["data-fit-max-width"] !== undefined,
        );
        expect(
          fitted,
          `${expected} must be rendered through the shared text-fit helper`,
        ).toBeDefined();

        const maxWidth = Number(fitted?.attrs["data-fit-max-width"]);
        const fontSize = Number(fitted?.attrs["font-size"]);
        const renderedWidth = fitted?.attrs.textLength
          ? Number(fitted.attrs.textLength)
          : (fitted?.text.length ?? 0) * fontSize * 0.6;

        expect(maxWidth).toBeGreaterThan(0);
        expect(fontSize).toBeGreaterThan(0);
        expect(renderedWidth).toBeLessThanOrEqual(maxWidth);
      }
    },
  );

  it("keeps CJK and other wide glyphs inside the declared fit budget", () => {
    const maxWidth = 96;
    const markup = fittedText({
      text: "프로필使用量統計テスト".repeat(2),
      x: 0,
      y: 16,
      maxWidth,
      fill: "#fff",
      fontSize: 16,
      minFontSize: 8,
    });
    const [node] = textNodes(markup);

    expect(node.attrs["data-fit-max-width"]).toBe(String(maxWidth));
    expect(renderedTextWidth(node)).toBeLessThanOrEqual(maxWidth);
  });

  it("fits long profile error copy instead of allowing it to overflow", () => {
    const message =
      "프로필을 찾을 수 없습니다 · 使用量データを確認してください";
    const node = textNodes(renderProfileEmbedErrorSvg(message)).find(
      (candidate) => candidate.text === message,
    );

    expect(node).toBeDefined();
    expect(node?.attrs["data-fit-max-width"]).toBeDefined();
    expect(renderedTextWidth(node!)).toBeLessThanOrEqual(
      Number(node?.attrs["data-fit-max-width"]),
    );
  });
});

describe("embed contribution scaling", () => {
  it("uses the same inclusive 25%, 50%, and 75% token thresholds as the profile", () => {
    const tokenOnly: EmbedContributionDay[] = [
      ["2026-02-17", 1, 1],
      ["2026-02-18", 24, 1],
      ["2026-02-19", 25, 2],
      ["2026-02-20", 49, 2],
      ["2026-02-21", 50, 3],
      ["2026-02-22", 74, 3],
      ["2026-02-23", 75, 4],
      ["2026-02-24", 100, 4],
    ].map(([date, totalTokens, intensity]) => ({
      date: String(date),
      totalTokens: Number(totalTokens),
      totalCost: 0,
      intensity: Number(intensity) as 0 | 1 | 2 | 3 | 4,
    }));

    const svg = renderGraphEmbedSvg(mockStats, {
      contributions: tokenOnly,
    });

    for (const [date, , expectedLevel] of [
      ["2026-02-17", 1, 1],
      ["2026-02-18", 24, 1],
      ["2026-02-19", 25, 2],
      ["2026-02-20", 49, 2],
      ["2026-02-21", 50, 3],
      ["2026-02-22", 74, 3],
      ["2026-02-23", 75, 4],
      ["2026-02-24", 100, 4],
    ] as const) {
      expect(svg).toContain(`<title>${date} · level ${expectedLevel}</title>`);
    }
    expect(svg).toContain("8 active days");
  });

  it("excludes invisible buffer spikes from visible levels and one-year facts", () => {
    const visible: EmbedContributionDay[] = [
      {
        date: "2026-02-23",
        totalTokens: 25,
        totalCost: 1,
        intensity: 2,
      },
      {
        date: "2026-02-24",
        totalTokens: 100,
        totalCost: 4,
        intensity: 4,
      },
    ];
    const withInvisibleSpikes = [
      {
        date: "2025-02-22",
        totalTokens: 999_999_999,
        totalCost: 999_999,
        intensity: 4 as const,
      },
      ...visible,
      {
        date: "2026-02-25",
        totalTokens: 888_888_888,
        totalCost: 888_888,
        intensity: 4 as const,
      },
    ];

    expect(
      renderGraphEmbedSvg(mockStats, { contributions: withInvisibleSpikes }),
    ).toBe(renderGraphEmbedSvg(mockStats, { contributions: visible }));
    expect(
      renderBlueprintEmbedSvg(mockStats, {
        contributions: withInvisibleSpikes,
      }),
    ).toBe(renderBlueprintEmbedSvg(mockStats, { contributions: visible }));
  });

  it("keeps 3D cube geometry unchanged when only contribution cost changes", () => {
    const baseline: EmbedContributionDay[] = [
      {
        date: "2026-02-23",
        totalTokens: 100,
        totalCost: 0,
        intensity: 4,
      },
      {
        date: "2026-02-24",
        totalTokens: 0,
        totalCost: 0,
        intensity: 1,
      },
    ];
    const costOnlyMutation = baseline.map((day) =>
      day.date === "2026-02-24" ? { ...day, totalCost: 1_000_000 } : day,
    );

    expect(
      isometricCubeGeometry(
        renderIsometric3DEmbedSvg(mockStats, costOnlyMutation),
      ),
    ).toEqual(
      isometricCubeGeometry(renderIsometric3DEmbedSvg(mockStats, baseline)),
    );
  });
});

describe("orbit standing", () => {
  it("reports the last profile as ahead of 0% without counting itself", () => {
    const svg = renderOrbitEmbedSvg({
      ...mockStats,
      stats: { ...mockStats.stats, rank: 100, rankTotal: 100 },
    });

    expect(svg).toContain("Ahead of 0% of ranked profiles");
  });

  it("reports the first profile as ahead of every other ranked profile", () => {
    const svg = renderOrbitEmbedSvg({
      ...mockStats,
      stats: { ...mockStats.stats, rank: 1, rankTotal: 100 },
    });

    expect(svg).toContain("Ahead of 99% of ranked profiles");
  });
});

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
      expect(svg).not.toContain("<mask");
      expect(svg).not.toMatch(/(?:fill|stroke)="url\(/);
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

  it("keeps an explicitly requested empty blueprint graph visible", () => {
    const svg = renderBlueprintEmbedSvg(mockStats, {
      contributions: [],
      graph: true,
    });

    expect(svg).toContain("Contribution activity");
    expect(svg).toContain('height="367"');
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
