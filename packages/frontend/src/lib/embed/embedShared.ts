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
import {
  colorPalettes,
  DEFAULT_PALETTE,
  getDarkGradeColors,
  getPaletteNames,
  type ColorPaletteName,
} from "../themes";

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
  scheme: EmbedTheme;
  surface: string;
  border: string;
  text: string;
  muted: string;
  brand: string;
  cost: string;
  rankGold: string;
  rankSilver: string;
  rankBronze: string;
  rankDefault: string;
  divider: string;
  graphGrade0: string;
  graphGrade1: string;
  graphGrade2: string;
  graphGrade3: string;
  graphGrade4: string;
}

const defaultDarkGrades = getDarkGradeColors(colorPalettes[DEFAULT_PALETTE]);
const defaultLightPalette = colorPalettes[DEFAULT_PALETTE];

export const THEMES: Record<EmbedTheme, ThemePalette> = {
  dark: {
    scheme: "dark",
    surface: "#131822",
    border: "rgba(255,255,255,0.16)",
    text: "#F4F7FB",
    muted: "#A8B3C5",
    brand: "#2F8FFF",
    cost: "#3FB950",
    rankGold: "#E3B341",
    rankSilver: "#A8B3C5",
    rankBronze: "#DA7E1A",
    rankDefault: "#F4F7FB",
    divider: "rgba(255,255,255,0.09)",
    graphGrade0: "#191F2B",
    graphGrade1: defaultDarkGrades[0],
    graphGrade2: defaultDarkGrades[1],
    graphGrade3: defaultDarkGrades[2],
    graphGrade4: defaultDarkGrades[3],
  },
  light: {
    scheme: "light",
    surface: "#FFFFFF",
    border: "rgba(31,35,40,0.18)",
    text: "#1F2328",
    muted: "#57606A",
    brand: "#0969DA",
    cost: "#1A7F37",
    rankGold: "#9A6700",
    rankSilver: "#656D76",
    rankBronze: "#BC4C00",
    rankDefault: "#1F2328",
    divider: "rgba(31,35,40,0.12)",
    graphGrade0: "#EFF2F5",
    graphGrade1: defaultLightPalette.grade1,
    graphGrade2: defaultLightPalette.grade2,
    graphGrade3: defaultLightPalette.grade3,
    graphGrade4: defaultLightPalette.grade4,
  },
};

export const FIGTREE_FONT_STACK =
  "Figtree, -apple-system, BlinkMacSystemFont, Segoe UI, sans-serif";
export const MONO_FONT_STACK =
  "ui-monospace, SFMono-Regular, Menlo, Consolas, Liberation Mono, monospace";

export const MONTH_NAMES = [
  "Jan",
  "Feb",
  "Mar",
  "Apr",
  "May",
  "Jun",
  "Jul",
  "Aug",
  "Sep",
  "Oct",
  "Nov",
  "Dec",
];

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
  return EMBED_TEMPLATES.includes(value as EmbedTemplate)
    ? (value as EmbedTemplate)
    : "classic";
}

/** Parse the `color` query param against the named graph palettes. */
export function parseEmbedColor(value: string | null): EmbedColorName | null {
  if (!value) return null;
  return getPaletteNames().includes(value as ColorPaletteName)
    ? (value as ColorPaletteName)
    : null;
}

/** Parse a `compact` | `full` number-format query param; `undefined` if unset. */
export function parseNumberFormat(
  value: string | null,
): EmbedNumberFormat | undefined {
  if (value === "full") return "full";
  if (value === "compact") return "compact";
  return undefined;
}

/** Parse the `rank` query param; undefined falls back to the renderer default. */
export function parseRankFormat(
  value: string | null,
): EmbedRankFormat | undefined {
  if (value === "plain" || value === "percent" || value === "total")
    return value;
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

/**
 * Apply a named graph palette without changing semantic cost or medal colors.
 * Dark cards use the same contrast-corrected ramp as the public profile.
 */
export function applyEmbedColor(
  palette: ThemePalette,
  color: EmbedColorName | null,
): ThemePalette {
  if (!color) return palette;
  const p = colorPalettes[color];
  if (!p) return palette;
  const grades =
    palette.scheme === "dark"
      ? getDarkGradeColors(p)
      : ([p.grade1, p.grade2, p.grade3, p.grade4] as const);
  return {
    ...palette,
    brand: palette.scheme === "dark" ? grades[2] : p.grade2,
    graphGrade1: grades[0],
    graphGrade2: grades[1],
    graphGrade3: grades[2],
    graphGrade4: grades[3],
  };
}

/** Resolve the base theme palette and apply an optional color override. */
export function resolvePalette(
  theme: EmbedTheme,
  color: EmbedColorName | null,
): ThemePalette {
  return applyEmbedColor(THEMES[theme], color);
}

export function getRankColor(
  rank: number | null,
  palette: ThemePalette,
): string {
  if (rank === 1) return palette.rankGold;
  if (rank === 2) return palette.rankSilver;
  if (rank === 3) return palette.rankBronze;
  return palette.rankDefault;
}

/** Human-readable "Updated <date> (UTC)" footer label. */
export function formatDateLabel(value: string | null): string {
  if (!value) return "No submissions yet";
  const date = new Date(value);
  if (Number.isNaN(date.getTime())) return "Update unavailable";
  return `Updated ${new Intl.DateTimeFormat("en-US", {
    month: "short",
    day: "numeric",
    year: "numeric",
    timeZone: "UTC",
  }).format(date)} (UTC)`;
}

export interface ContributionDay {
  date: string;
  intensity: 0 | 1 | 2 | 3 | 4;
  totalTokens?: number;
}

export interface ContributionCell {
  week: number;
  day: number;
  date: string;
  inRange: boolean;
  intensity: 0 | 1 | 2 | 3 | 4;
}

export interface ContributionLayout {
  numWeeks: number;
  cells: ContributionCell[];
  months: { week: number; label: string }[];
  activeDays: number;
  rangeDays: number;
}

export function getContributionIntensity(
  tokens: number,
  maxTokens: number,
): 0 | 1 | 2 | 3 | 4 {
  if (tokens <= 0 || maxTokens <= 0) return 0;
  const ratio = tokens / maxTokens;
  if (ratio >= 0.75) return 4;
  if (ratio >= 0.5) return 3;
  if (ratio >= 0.25) return 2;
  return 1;
}

const DAY_MS = 86_400_000;

export interface ContributionWindow<T extends ContributionDay> {
  calendarStart: string;
  rangeStart: string;
  rangeEnd: string;
  rangeDays: number;
  numWeeks: number;
  days: T[];
}

/**
 * Scope contribution facts to the same exact trailing year as the profile.
 * The grid may begin on the preceding Sunday, but aligned padding never
 * affects intensity, active-day counts, or one-year totals.
 */
export function getContributionWindow<T extends ContributionDay>(
  contributions: readonly T[],
  referenceDate: Date = new Date(),
): ContributionWindow<T> {
  const rangeEnd = new Date(
    Date.UTC(
      referenceDate.getUTCFullYear(),
      referenceDate.getUTCMonth(),
      referenceDate.getUTCDate(),
    ),
  );
  const rangeStart = new Date(rangeEnd);
  rangeStart.setUTCFullYear(rangeStart.getUTCFullYear() - 1);
  const calendarStart = new Date(rangeStart);
  calendarStart.setUTCDate(
    calendarStart.getUTCDate() - calendarStart.getUTCDay(),
  );

  const rangeStartKey = rangeStart.toISOString().slice(0, 10);
  const rangeEndKey = rangeEnd.toISOString().slice(0, 10);
  const calendarStartKey = calendarStart.toISOString().slice(0, 10);
  const rangeDays =
    Math.floor((rangeEnd.getTime() - rangeStart.getTime()) / DAY_MS) + 1;
  const calendarDays =
    Math.floor((rangeEnd.getTime() - calendarStart.getTime()) / DAY_MS) + 1;

  return {
    calendarStart: calendarStartKey,
    rangeStart: rangeStartKey,
    rangeEnd: rangeEndKey,
    rangeDays,
    numWeeks: Math.ceil(calendarDays / 7),
    days: contributions.filter(
      ({ date }) => date >= rangeStartKey && date <= rangeEndKey,
    ),
  };
}

/**
 * Lay out the trailing ~1 year of contributions into a GitHub-style grid of
 * weeks (columns) by weekdays (rows), aligned so the first column starts on a
 * Sunday. Future days are omitted. `activeDays` counts days with any usage.
 */
export function layoutContributions(
  contributions: ContributionDay[],
): ContributionLayout {
  const window = getContributionWindow(contributions);
  const hasTokenTotals = window.days.some(
    ({ totalTokens }) => typeof totalTokens === "number",
  );
  const maxTokens = hasTokenTotals
    ? Math.max(
        0,
        ...window.days.map(({ totalTokens }) => Math.max(0, totalTokens ?? 0)),
      )
    : 0;
  const intensityMap = new Map<string, 0 | 1 | 2 | 3 | 4>();
  for (const contribution of window.days) {
    intensityMap.set(
      contribution.date,
      hasTokenTotals
        ? getContributionIntensity(
            Math.max(0, contribution.totalTokens ?? 0),
            maxTokens,
          )
        : contribution.intensity,
    );
  }

  const start = new Date(`${window.calendarStart}T00:00:00Z`);
  const today = new Date(`${window.rangeEnd}T00:00:00Z`);
  const numWeeks = window.numWeeks;

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
      const dateKey = date.toISOString().split("T")[0];
      const inRange = dateKey >= window.rangeStart;
      const intensity = inRange ? (intensityMap.get(dateKey) ?? 0) : 0;
      if (intensity > 0) activeDays += 1;
      cells.push({ week: w, day: d, date: dateKey, inRange, intensity });
    }
  }

  return {
    numWeeks,
    cells,
    months,
    activeDays,
    rangeDays: window.rangeDays,
  };
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

export function cardTextStyle(): string {
  return `<defs><style>text{font-variant-numeric:tabular-nums}</style></defs>`;
}

export interface FittedTextOptions {
  text: string;
  x: number;
  y: number;
  maxWidth: number;
  fill: string;
  fontSize: number;
  minFontSize?: number;
  fontFamily?: string;
  fontWeight?: number | string;
  textAnchor?: "start" | "middle" | "end";
  attributes?: string;
}

function estimateTextUnits(value: string): number {
  let units = 0;
  for (const character of Array.from(value)) {
    const codePoint = character.codePointAt(0) ?? 0;
    const isCombiningMark =
      (codePoint >= 0x0300 && codePoint <= 0x036f) ||
      (codePoint >= 0x1ab0 && codePoint <= 0x1aff) ||
      (codePoint >= 0x1dc0 && codePoint <= 0x1dff) ||
      (codePoint >= 0x20d0 && codePoint <= 0x20ff) ||
      (codePoint >= 0xfe20 && codePoint <= 0xfe2f);
    if (isCombiningMark) continue;
    units += codePoint > 0xff ? 1 : 0.6;
  }
  return Math.max(1, units);
}

/** Render one text node with an explicit width budget for stress-safe embeds. */
export function fittedText(options: FittedTextOptions): string {
  const {
    text,
    x,
    y,
    maxWidth,
    fill,
    fontSize,
    minFontSize = 8,
    fontFamily = FIGTREE_FONT_STACK,
    fontWeight,
    textAnchor = "start",
    attributes = "",
  } = options;
  const characterUnits = estimateTextUnits(text);
  const idealSize = maxWidth / characterUnits;
  const fittedSize = Math.min(
    fontSize,
    Math.max(minFontSize, Math.floor(idealSize * 10) / 10),
  );
  const needsLengthAdjustment = characterUnits * fittedSize > maxWidth;
  const size = Number.isInteger(fittedSize)
    ? String(fittedSize)
    : fittedSize.toFixed(1);

  return `<text x="${x}" y="${y}" fill="${fill}" font-size="${size}"${fontWeight ? ` font-weight="${fontWeight}"` : ""}${textAnchor === "start" ? "" : ` text-anchor="${textAnchor}"`} font-family="${fontFamily}" data-fit-max-width="${maxWidth}"${needsLengthAdjustment ? ` textLength="${maxWidth}" lengthAdjust="spacingAndGlyphs"` : ""}${attributes ? ` ${attributes}` : ""}>${escapeXml(text)}</text>`;
}

/** Shared quiet card surface used by every 2D embed template. */
export function cardSurface(
  width: number,
  height: number,
  palette: ThemePalette,
  radius = 12,
): string {
  return [
    `<rect width="${width}" height="${height}" rx="${radius}" fill="${palette.surface}"/>`,
    `<rect x="0.5" y="0.5" width="${width - 1}" height="${height - 1}" rx="${radius - 0.5}" fill="none" stroke="${palette.border}"/>`,
  ].join("\n  ");
}

export interface CardHeaderOptions {
  username: string;
  displayName?: string | null;
  palette: ThemePalette;
  x: number;
  y: number;
  right: number;
}

/** Compact identity header shared by every 2D widget. */
export function cardHeader(options: CardHeaderOptions): string {
  const { username, displayName, palette, x, y, right } = options;
  const identityWidth = Math.max(80, right - x - 92);
  const headline = displayName?.trim() || `@${username}`;

  return [
    fittedText({
      text: headline,
      x,
      y,
      maxWidth: identityWidth,
      fill: palette.text,
      fontSize: 16,
      minFontSize: 10,
      fontWeight: 600,
    }),
    displayName
      ? fittedText({
          text: `@${username}`,
          x,
          y: y + 17,
          maxWidth: identityWidth,
          fill: palette.muted,
          fontSize: 11,
          minFontSize: 8,
        })
      : "",
    `<text x="${right}" y="${y}" fill="${palette.muted}" font-size="11" font-weight="600" text-anchor="end" font-family="${FIGTREE_FONT_STACK}">Tokscale</text>`,
  ]
    .filter(Boolean)
    .join("\n  ");
}

export function cardFooter(options: {
  updatedAt: string | null;
  palette: ThemePalette;
  x: number;
  right: number;
  y: number;
}): string {
  const available = options.right - options.x;
  return [
    `<g data-card-footer-y="${options.y}">`,
    fittedText({
      text: formatDateLabel(options.updatedAt),
      x: options.x,
      y: options.y,
      maxWidth: Math.max(80, available - 96),
      fill: options.palette.muted,
      fontSize: 10,
    }),
    fittedText({
      text: "tokscale.ai",
      x: options.right,
      y: options.y,
      maxWidth: 78,
      fill: options.palette.muted,
      fontSize: 10,
      textAnchor: "end",
    }),
    `</g>`,
  ].join("\n  ");
}

export function divider(
  x: number,
  right: number,
  y: number,
  palette: ThemePalette,
): string {
  return `<line x1="${x}" y1="${y}" x2="${right}" y2="${y}" stroke="${palette.divider}"/>`;
}

export interface ContributionPanelOptions {
  x: number;
  y: number;
  width: number;
  palette: ThemePalette;
  contributions: ContributionDay[];
  showDayLabels?: boolean;
  showLegend?: boolean;
  mono?: boolean;
  heading?: string;
}

export interface ContributionPanelResult {
  svg: string;
  height: number;
  activeDays: number;
}

/**
 * Shared contribution panel. It retains each date in a native SVG title so
 * standalone SVG embeds remain inspectable without adding visual noise.
 */
export function contributionPanel(
  options: ContributionPanelOptions,
): ContributionPanelResult {
  const {
    x,
    y,
    width,
    palette,
    contributions,
    showDayLabels = false,
    showLegend = false,
    mono = false,
    heading = "Contribution activity",
  } = options;
  const layout = layoutContributions(contributions);
  const colors = gradeColors(palette);
  const font = mono ? MONO_FONT_STACK : FIGTREE_FONT_STACK;
  const dayLabelWidth = showDayLabels ? 30 : 0;
  const gap = width >= 700 ? 3 : 2;
  const graphWidth = width - dayLabelWidth;
  const stride = (graphWidth + gap) / layout.numWeeks;
  const cell = Math.max(2, stride - gap);
  const graphX = x + dayLabelWidth;
  const monthY = y + 28;
  const gridY = y + 36;
  const gridHeight = 7 * stride - gap;
  const legendY = gridY + gridHeight + 16;
  const height = 36 + gridHeight + (showLegend ? 36 : 14);
  const parts: string[] = [];

  parts.push(
    `<text x="${x}" y="${y + 11}" fill="${palette.text}" font-size="12" font-weight="600" font-family="${font}">${escapeXml(heading)}</text>`,
  );
  parts.push(
    `<text x="${x + width}" y="${y + 11}" fill="${palette.muted}" font-size="10" text-anchor="end" font-family="${font}">${layout.activeDays} active days</text>`,
  );

  for (const month of layout.months) {
    parts.push(
      `<text x="${(graphX + month.week * stride).toFixed(1)}" y="${monthY}" fill="${palette.muted}" font-size="10" font-family="${font}">${month.label}</text>`,
    );
  }

  if (showDayLabels) {
    for (const [day, label] of [
      [1, "Mon"],
      [3, "Wed"],
      [5, "Fri"],
    ] as const) {
      parts.push(
        `<text x="${x}" y="${(gridY + day * stride + cell - 1).toFixed(1)}" fill="${palette.muted}" font-size="10" font-family="${font}">${label}</text>`,
      );
    }
  }

  for (const item of layout.cells) {
    parts.push(
      `<rect x="${(graphX + item.week * stride).toFixed(2)}" y="${(gridY + item.day * stride).toFixed(2)}" width="${cell.toFixed(2)}" height="${cell.toFixed(2)}" rx="${Math.min(2, cell / 3).toFixed(1)}" fill="${colors[item.intensity]}"><title>${item.date} · level ${item.intensity}</title></rect>`,
    );
  }

  if (showLegend) {
    const legendCell = 9;
    const legendStride = 13;
    let legendX = x + width - (24 + legendStride * 5 + 28);
    parts.push(
      `<text x="${legendX}" y="${legendY + 8}" fill="${palette.muted}" font-size="10" font-family="${font}">Less</text>`,
    );
    legendX += 24;
    colors.forEach((color) => {
      parts.push(
        `<rect x="${legendX}" y="${legendY}" width="${legendCell}" height="${legendCell}" rx="2" fill="${color}"/>`,
      );
      legendX += legendStride;
    });
    parts.push(
      `<text x="${legendX + 2}" y="${legendY + 8}" fill="${palette.muted}" font-size="10" font-family="${font}">More</text>`,
    );
  }

  return {
    svg: `<g data-contribution-panel="true" data-bottom="${(y + height).toFixed(1)}">\n  ${parts.join("\n  ")}\n  </g>`,
    height,
    activeDays: layout.activeDays,
  };
}
