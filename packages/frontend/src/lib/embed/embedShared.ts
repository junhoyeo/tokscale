/**
 * Shared primitives for the profile embed SVG renderers.
 *
 * The embed endpoint can render several card templates (classic, minimal,
 * terminal, graph). This module holds what they all need: the dark/light
 * theme palettes, an optional color override that maps a named graph palette
 * onto the embed accent colors, number-format parsing, font stacks, the
 * contribution-grid layout, and small SVG building blocks.
 */
import { escapeXml } from "../format";
import { colorPalettes, getPaletteNames, type ColorPaletteName } from "../themes";

export { escapeXml };

export type EmbedTheme = "dark" | "light";
export type EmbedTemplate =
  | "classic"
  | "minimal"
  | "terminal"
  | "graph"
  | "orbit"
  | "vitals"
  | "blueprint"
  | "receipt";
export type EmbedNumberFormat = "compact" | "full";
export type EmbedRankFormat = "plain" | "percent" | "total";
export type EmbedColorName = ColorPaletteName;

export interface ThemePalette {
  bgStart: string;
  bgEnd: string;
  border: string;
  glowColor: string;
  glowOpacity: number;
  metricBg: string;
  metricBorder: string;
  title: string;
  text: string;
  muted: string;
  brand: string;
  tokenStart: string;
  tokenEnd: string;
  cost: string;
  rankGold: string;
  rankSilver: string;
  rankBronze: string;
  rankDefault: string;
  badgeBg: string;
  badgeBorder: string;
  badgeText: string;
  divider: string;
  accentTokens: string;
  accentCost: string;
  accentRank: string;
  graphGrade0: string;
  graphGrade1: string;
  graphGrade2: string;
  graphGrade3: string;
  graphGrade4: string;
}

export const THEMES: Record<EmbedTheme, ThemePalette> = {
  dark: {
    bgStart: "#0D1117",
    bgEnd: "#010409",
    border: "#30363D",
    glowColor: "#388BFD",
    glowOpacity: 0.07,
    metricBg: "rgba(22,27,34,0.6)",
    metricBorder: "rgba(48,54,61,0.6)",
    title: "#F0F6FC",
    text: "#E6EDF3",
    muted: "#8B949E",
    brand: "#58A6FF",
    tokenStart: "#58A6FF",
    tokenEnd: "#A5D6FF",
    cost: "#3FB950",
    rankGold: "#E3B341",
    rankSilver: "#8B949E",
    rankBronze: "#DA7E1A",
    rankDefault: "#58A6FF",
    badgeBg: "rgba(56,139,253,0.08)",
    badgeBorder: "rgba(56,139,253,0.25)",
    badgeText: "#58A6FF",
    divider: "#30363D",
    accentTokens: "#58A6FF",
    accentCost: "#3FB950",
    accentRank: "#D29922",
    graphGrade0: "#161B22",
    graphGrade1: "#0E4429",
    graphGrade2: "#006D32",
    graphGrade3: "#26A641",
    graphGrade4: "#39D353",
  },
  light: {
    bgStart: "#FFFFFF",
    bgEnd: "#F6F8FA",
    border: "#D0D7DE",
    glowColor: "#0969DA",
    glowOpacity: 0.04,
    metricBg: "rgba(246,248,250,0.7)",
    metricBorder: "rgba(208,215,222,0.55)",
    title: "#1F2328",
    text: "#1F2328",
    muted: "#656D76",
    brand: "#0969DA",
    tokenStart: "#0969DA",
    tokenEnd: "#54AEFF",
    cost: "#1A7F37",
    rankGold: "#9A6700",
    rankSilver: "#656D76",
    rankBronze: "#BC4C00",
    rankDefault: "#0969DA",
    badgeBg: "rgba(9,105,218,0.06)",
    badgeBorder: "rgba(9,105,218,0.2)",
    badgeText: "#0969DA",
    divider: "#D0D7DE",
    accentTokens: "#0969DA",
    accentCost: "#1A7F37",
    accentRank: "#9A6700",
    graphGrade0: "#EBEDF0",
    graphGrade1: "#9BE9A8",
    graphGrade2: "#40C463",
    graphGrade3: "#30A14E",
    graphGrade4: "#216E39",
  },
};

export const FIGTREE_FONT_STACK = "Figtree, -apple-system, BlinkMacSystemFont, Segoe UI, sans-serif";
export const FIGTREE_FONT_IMPORT = "https://fonts.googleapis.com/css2?family=Figtree:wght@400;600;700;800&amp;display=swap";
export const MONO_FONT_STACK = "ui-monospace, SFMono-Regular, Menlo, Consolas, Liberation Mono, monospace";

export const MONTH_NAMES = ["Jan", "Feb", "Mar", "Apr", "May", "Jun", "Jul", "Aug", "Sep", "Oct", "Nov", "Dec"];

export const EMBED_TEMPLATES: EmbedTemplate[] = [
  "classic",
  "minimal",
  "terminal",
  "graph",
  "orbit",
  "vitals",
  "blueprint",
  "receipt",
];

/** Parse the `template` query param, falling back to the classic card. */
export function parseEmbedTemplate(value: string | null): EmbedTemplate {
  return EMBED_TEMPLATES.includes(value as EmbedTemplate) ? (value as EmbedTemplate) : "classic";
}

/** Parse the `color` query param against the named graph palettes. */
export function parseEmbedColor(value: string | null): EmbedColorName | null {
  if (!value) return null;
  return getPaletteNames().includes(value as ColorPaletteName) ? (value as ColorPaletteName) : null;
}

/** Parse a `compact` | `full` number-format query param; `undefined` if unset. */
export function parseNumberFormat(value: string | null): EmbedNumberFormat | undefined {
  if (value === "full") return "full";
  if (value === "compact") return "compact";
  return undefined;
}

/** Parse the `rank` query param; undefined falls back to the renderer default. */
export function parseRankFormat(value: string | null): EmbedRankFormat | undefined {
  if (value === "plain" || value === "percent" || value === "total") return value;
  return undefined;
}

/** Format a rank for display: `#134`, `top 12%`, or `#134 / 1,174`. */
export function formatRank(
  rank: number,
  total: number | null,
  format: EmbedRankFormat = "plain",
): string {
  if (format === "percent" && total && total > 0) {
    return `top ${Math.max(1, Math.ceil((rank / total) * 100))}%`;
  }
  if (format === "total" && total && total > 0) {
    return `#${rank} / ${total.toLocaleString("en-US")}`;
  }
  return `#${rank}`;
}


function hexToRgb(hex: string): [number, number, number] | null {
  const match = /^#?([0-9a-f]{6})$/i.exec(hex.trim());
  if (!match) return null;
  const int = parseInt(match[1], 16);
  return [(int >> 16) & 255, (int >> 8) & 255, int & 255];
}

/** Convert a `#rrggbb` hex string to an `rgba(...)` string; returns the input on failure. */
export function hexToRgba(hex: string, alpha: number): string {
  const rgb = hexToRgb(hex);
  return rgb ? `rgba(${rgb[0]},${rgb[1]},${rgb[2]},${alpha})` : hex;
}

/**
 * Return a palette with the accent family (brand, token gradient, graph
 * grades, badge, glow, non-medal rank) replaced by the named color palette.
 * Cost stays green and the medal rank colors stay semantic so those metrics
 * remain readable independent of the chosen accent.
 */
export function applyEmbedColor(palette: ThemePalette, color: EmbedColorName | null): ThemePalette {
  if (!color) return palette;
  const p = colorPalettes[color];
  if (!p) return palette;
  return {
    ...palette,
    glowColor: p.grade2,
    brand: p.grade2,
    tokenStart: p.grade2,
    tokenEnd: p.grade1,
    rankDefault: p.grade3,
    badgeBg: hexToRgba(p.grade2, 0.1),
    badgeBorder: hexToRgba(p.grade2, 0.3),
    badgeText: p.grade3,
    accentTokens: p.grade2,
    graphGrade1: p.grade1,
    graphGrade2: p.grade2,
    graphGrade3: p.grade3,
    graphGrade4: p.grade4,
  };
}

/** Resolve the base theme palette and apply an optional color override. */
export function resolvePalette(theme: EmbedTheme, color: EmbedColorName | null): ThemePalette {
  return applyEmbedColor(THEMES[theme], color);
}

export function getRankColor(rank: number | null, palette: ThemePalette): string {
  if (rank === 1) return palette.rankGold;
  if (rank === 2) return palette.rankSilver;
  if (rank === 3) return palette.rankBronze;
  return palette.rankDefault;
}

/** Human-readable "Updated <date> (UTC)" footer label. */
export function formatDateLabel(value: string | null): string {
  if (!value) return "No submissions yet";
  const date = new Date(value);
  if (Number.isNaN(date.getTime())) return "Updated recently";
  return `Updated ${new Intl.DateTimeFormat("en-US", {
    month: "short",
    day: "numeric",
    year: "numeric",
    timeZone: "UTC",
  }).format(date)} (UTC)`;
}

/** Three ascending bars used as the Tokscale brand mark. */
export function brandIcon(x: number, baselineY: number, color: string): string {
  const top = baselineY - 12;
  return [
    `<rect x="${x}" y="${top + 8}" width="3" height="6" rx="1" fill="${color}" opacity="0.45"/>`,
    `<rect x="${x + 5}" y="${top}" width="3" height="14" rx="1" fill="${color}"/>`,
    `<rect x="${x + 10}" y="${top + 4}" width="3" height="10" rx="1" fill="${color}" opacity="0.7"/>`,
  ].join("");
}

export interface ContributionDay {
  date: string;
  intensity: 0 | 1 | 2 | 3 | 4;
}

export interface ContributionCell {
  week: number;
  day: number;
  intensity: 0 | 1 | 2 | 3 | 4;
}

export interface ContributionLayout {
  numWeeks: number;
  cells: ContributionCell[];
  months: { week: number; label: string }[];
  activeDays: number;
}

/**
 * Lay out the trailing ~1 year of contributions into a GitHub-style grid of
 * weeks (columns) by weekdays (rows), aligned so the first column starts on a
 * Sunday. Future days are omitted. `activeDays` counts days with any usage.
 */
export function layoutContributions(contributions: ContributionDay[]): ContributionLayout {
  const intensityMap = new Map<string, 0 | 1 | 2 | 3 | 4>();
  for (const c of contributions) intensityMap.set(c.date, c.intensity);

  const now = new Date();
  const today = new Date(Date.UTC(now.getUTCFullYear(), now.getUTCMonth(), now.getUTCDate()));
  const start = new Date(today);
  start.setUTCFullYear(start.getUTCFullYear() - 1);
  start.setUTCDate(start.getUTCDate() + 1);
  start.setUTCDate(start.getUTCDate() - start.getUTCDay());

  const diffDays = Math.ceil((today.getTime() - start.getTime()) / 86_400_000);
  const numWeeks = Math.ceil((diffDays + 1) / 7);

  const cells: ContributionCell[] = [];
  const months: { week: number; label: string }[] = [];
  let lastMonth = -1;
  let activeDays = 0;

  for (let w = 0; w < numWeeks; w++) {
    const weekStart = new Date(start);
    weekStart.setUTCDate(weekStart.getUTCDate() + w * 7);
    if (weekStart.getUTCMonth() !== lastMonth) {
      lastMonth = weekStart.getUTCMonth();
      months.push({ week: w, label: MONTH_NAMES[lastMonth] });
    }
    for (let d = 0; d < 7; d++) {
      const date = new Date(start);
      date.setUTCDate(date.getUTCDate() + w * 7 + d);
      if (date > today) continue;
      const intensity = intensityMap.get(date.toISOString().split("T")[0]) ?? 0;
      if (intensity > 0) activeDays += 1;
      cells.push({ week: w, day: d, intensity });
    }
  }

  return { numWeeks, cells, months, activeDays };
}

/** Ordered grade colors [empty, ...four intensity levels] for a palette. */
export function gradeColors(palette: ThemePalette): string[] {
  return [
    palette.graphGrade0,
    palette.graphGrade1,
    palette.graphGrade2,
    palette.graphGrade3,
    palette.graphGrade4,
  ];
}

/** Point on a circle. 0deg = 12 o'clock, angle increases clockwise. */
export function polarPoint(cx: number, cy: number, r: number, deg: number): [number, number] {
  const a = ((deg - 90) * Math.PI) / 180;
  return [cx + r * Math.cos(a), cy + r * Math.sin(a)];
}

/** SVG arc path from startDeg to endDeg, drawn clockwise. 0deg = 12 o'clock. */
export function arcPath(cx: number, cy: number, r: number, startDeg: number, endDeg: number): string {
  const [x1, y1] = polarPoint(cx, cy, r, startDeg);
  const [x2, y2] = polarPoint(cx, cy, r, endDeg);
  const large = Math.abs(endDeg - startDeg) > 180 ? 1 : 0;
  return `M ${x1.toFixed(2)} ${y1.toFixed(2)} A ${r} ${r} 0 ${large} 1 ${x2.toFixed(2)} ${y2.toFixed(2)}`;
}

/** Count of contribution days at intensity >= the given threshold. */
export function activeDayCount(contributions: ContributionDay[], minIntensity = 1): number {
  return contributions.reduce((n, c) => (c.intensity >= minIntensity ? n + 1 : n), 0);
}
