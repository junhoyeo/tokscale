import type { UserEmbedStats, EmbedContributionDay } from "./getUserEmbedStats";
import { escapeXml, formatNumber, formatCurrency } from "../format";
import { colorPalettes, getDarkGradeColors } from "../themes";
import {
  fittedText,
  formatDateLabel,
  getContributionIntensity,
  getContributionWindow,
} from "./embedShared";

export type EmbedTheme = "dark" | "light";

type ThemePalette = {
  surface: string;
  border: string;
  borderOpacity: number;
  text: string;
  muted: string;
  accent: string;
  divider: string;
  dividerOpacity: number;
  graphGrade: [string, string, string, string, string];
};

const LEFT_FACTOR = 0.8367; // 0.7^0.5  — d3 .darker(0.5)
const RIGHT_FACTOR = 0.7; //   0.7^1.0  — d3 .darker(1.0)

function darken(hex: string, factor: number): string {
  const r = parseInt(hex.slice(1, 3), 16);
  const g = parseInt(hex.slice(3, 5), 16);
  const b = parseInt(hex.slice(5, 7), 16);
  const c = (v: number) => Math.max(0, Math.min(255, Math.round(v * factor)));
  return `rgb(${c(r)},${c(g)},${c(b)})`;
}

function buildFaceCSS(theme: string, grades: readonly string[]): string {
  let css = "";
  for (let i = 0; i < grades.length; i++) {
    const hex = grades[i];
    css += `.${theme}${i}-t{fill:${hex}}`;
    css += `.${theme}${i}-l{fill:${darken(hex, LEFT_FACTOR)}}`;
    css += `.${theme}${i}-r{fill:${darken(hex, RIGHT_FACTOR)}}`;
  }
  return css;
}

const darkBlueGrades = getDarkGradeColors(colorPalettes.blue);

const THEMES: Record<EmbedTheme, ThemePalette> = {
  dark: {
    surface: "#131822",
    border: "#FFFFFF",
    borderOpacity: 0.09,
    text: "#F4F7FB",
    muted: "#A8B3C5",
    accent: "#57A5FF",
    divider: "#FFFFFF",
    dividerOpacity: 0.09,
    graphGrade: ["#191F2B", ...darkBlueGrades],
  },
  light: {
    surface: "#FFFFFF",
    border: "#1F2328",
    borderOpacity: 0.14,
    text: "#1F2328",
    muted: "#59636E",
    accent: "#0969DA",
    divider: "#1F2328",
    dividerOpacity: 0.12,
    graphGrade: ["#E8ECF1", "#79B8FF", "#388BFD", "#1F6FEB", "#0D419D"],
  },
};

const FONT_STACK = "-apple-system, BlinkMacSystemFont, Segoe UI, sans-serif";

const CELL = 10;
const W = 9;
const TAN30 = 0.5774;
const DY = +(CELL * TAN30).toFixed(2);
const RIGHT_DY = +(W * TAN30).toFixed(2);
const MAX_HEIGHT = 35;
const MIN_HEIGHT = 2;
const MIN_NON_ZERO_HEIGHT = 8;

/**
 * Isometric cube via <rect> + CSS transforms (matches github-profile-3d-contrib).
 * Top face: skewY(-30) skewX(40.89) scale(1 1.15)
 * Left face: skewY(30) scale(1 1.15)
 * Right face: translate(W dy) skewY(-30) scale(1 1.15)
 */
function renderCube(x: number, y: number, h: number, cls: string): string {
  const SCALE_Y = 1.15;
  const faceH = +(h / SCALE_Y).toFixed(1);
  const topT = `skewY(-30) skewX(40.89) scale(1 ${SCALE_Y})`;
  const leftT = `skewY(30) scale(1 ${SCALE_Y})`;
  const rightT = `translate(${W} ${RIGHT_DY}) skewY(-30) scale(1 ${SCALE_Y})`;
  const fx = x.toFixed(1);
  const fy = y.toFixed(1);

  return `<g transform="translate(${fx} ${fy})"><rect stroke="none" x="0" y="0" width="${W}" height="${W}" transform="${topT}" class="${cls}-t"/><rect stroke="none" x="0" y="0" width="${W}" height="${faceH}" transform="${leftT}" class="${cls}-l"/><rect stroke="none" x="0" y="0" width="${W}" height="${faceH}" transform="${rightT}" class="${cls}-r"/></g>`;
}

function formatShortDate(dateStr: string, includeYear: boolean): string {
  const d = new Date(dateStr + "T00:00:00Z");
  if (Number.isNaN(d.getTime())) return dateStr;
  return new Intl.DateTimeFormat("en-US", {
    month: "short",
    day: "numeric",
    ...(includeYear ? { year: "numeric" } : {}),
    timeZone: "UTC",
  }).format(d);
}

function formatActiveDateRange(activeDates: string[]): string {
  if (activeDates.length === 0) return "No activity yet";
  if (activeDates.length === 1) return formatShortDate(activeDates[0], true);

  const first = activeDates[0];
  const last = activeDates[activeDates.length - 1];
  const crossesYear = first.slice(0, 4) !== last.slice(0, 4);
  return `${formatShortDate(first, crossesYear)} \u2192 ${formatShortDate(last, true)}`;
}

export function renderIsometric3DEmbedSvg(
  data: UserEmbedStats,
  contributions: EmbedContributionDay[],
  options: { theme?: EmbedTheme; compact?: boolean } = {},
): string {
  const theme: EmbedTheme = options.theme === "light" ? "light" : "dark";
  const compactNumbers = options.compact ?? false;
  const palette = THEMES[theme];
  const cls = theme === "dark" ? "d" : "l";

  const contributionWindow = getContributionWindow(contributions);
  const visibleContributions = contributionWindow.days;
  const start = new Date(`${contributionWindow.calendarStart}T00:00:00Z`);
  const today = new Date(`${contributionWindow.rangeEnd}T00:00:00Z`);
  const numWeeks = contributionWindow.numWeeks;
  const maxContributionTokens = visibleContributions.reduce(
    (max, contribution) => Math.max(max, contribution.totalTokens),
    0,
  );
  const contributionMap = new Map<string, EmbedContributionDay>();
  for (const contribution of visibleContributions) {
    contributionMap.set(contribution.date, {
      ...contribution,
      intensity: getContributionIntensity(
        contribution.totalTokens,
        maxContributionTokens,
      ),
    });
  }

  const px = 24;
  const width = 680;
  const headerH = 70;
  const gridRows = numWeeks + 7;
  const height = 482;
  const rx = 12;

  const gridXExtent = gridRows * CELL + W + W;
  const gridOriginX = +(
    px +
    7 * CELL +
    Math.max(0, (width - 2 * px - gridXExtent) / 2)
  ).toFixed(1);
  const gridOriginY = +(headerH + MAX_HEIGHT + 4).toFixed(1);
  let cubes = "";
  for (let w = 0; w < numWeeks; w++) {
    for (let d = 0; d < 7; d++) {
      const date = new Date(start);
      date.setUTCDate(date.getUTCDate() + w * 7 + d);
      if (date > today) continue;

      const dateStr = date.toISOString().split("T")[0];
      const contribution = contributionMap.get(dateStr);
      const intensity = (contribution?.intensity ?? 0) as 0 | 1 | 2 | 3 | 4;
      const contributionTokens = contribution?.totalTokens ?? 0;
      const h =
        contributionTokens > 0 && maxContributionTokens > 0
          ? Math.max(
              MIN_NON_ZERO_HEIGHT,
              Math.round(
                (contributionTokens / maxContributionTokens) *
                  (MAX_HEIGHT - MIN_NON_ZERO_HEIGHT) +
                  MIN_NON_ZERO_HEIGHT,
              ),
            )
          : MIN_HEIGHT;

      const cubeX = gridOriginX + (w - d) * CELL;
      const cubeY = gridOriginY + (w + d) * DY - h;

      cubes += renderCube(cubeX, cubeY, h, `${cls}${intensity}`);
    }
  }

  const username = `@${data.user.username}`;
  const tokens = formatNumber(data.stats.totalTokens, compactNumbers);
  const cost = formatCurrency(data.stats.totalCost, compactNumbers);
  const rank = data.stats.rank ? `#${data.stats.rank}` : "\u2014";
  const updated = escapeXml(formatDateLabel(data.stats.updatedAt));

  const activeDates = visibleContributions
    .filter((c) => c.totalTokens > 0)
    .map((c) => c.date)
    .sort();
  const activeDays = activeDates.length;
  const dateRange = formatActiveDateRange(activeDates);

  const dataRailHeight = 72;
  const dataRailY = height - dataRailHeight;
  const metricWidth = (width - px * 2) / 4;
  const metrics = [
    { label: "Tokens", value: tokens },
    { label: "Cost", value: cost },
    { label: "Rank", value: rank },
    { label: "Active days", value: String(activeDays) },
  ];
  const metricMarkup = metrics
    .map((metric, index) => {
      const cellX = px + metricWidth * index;
      const textX = cellX + (index === 0 ? 0 : 16);
      const divider =
        index === 0
          ? ""
          : `<line x1="${cellX}" y1="${dataRailY + 15}" x2="${cellX}" y2="${height - 15}" stroke="${palette.divider}" stroke-opacity="${palette.dividerOpacity}"/>`;
      const valueColor = index === 0 ? palette.accent : palette.text;
      return `${divider}<text x="${textX}" y="${dataRailY + 24}" fill="${palette.muted}" font-size="10" font-weight="500" font-family="${FONT_STACK}">${metric.label}</text>${fittedText({ text: metric.value, x: textX, y: dataRailY + 49, maxWidth: metricWidth - (index === 0 ? 16 : 32), fill: valueColor, fontSize: 16, minFontSize: 8, fontFamily: FONT_STACK, fontWeight: 600 })}`;
    })
    .join("");

  const gridScale = 0.86;
  const gridTranslateX = +(((1 - gridScale) * width) / 2).toFixed(1);
  const gridTranslateY = +(
    headerH -
    (gridOriginY - MAX_HEIGHT) * gridScale
  ).toFixed(1);
  const faceCss = buildFaceCSS(cls, palette.graphGrade);
  const safeUsername = escapeXml(username);

  return `<?xml version="1.0" encoding="UTF-8"?>
<svg data-template="3d" width="${width}" height="${height}" viewBox="0 0 ${width} ${height}" fill="none" xmlns="http://www.w3.org/2000/svg" role="img" aria-label="Tokscale 3D contribution graph for ${safeUsername}">
  <title>${safeUsername} Tokscale 3D contribution graph</title>
  <style>${faceCss}</style>
  <rect width="${width}" height="${height}" rx="${rx}" fill="${palette.surface}"/>
  ${fittedText({ text: username, x: px, y: 29, maxWidth: 300, fill: palette.text, fontSize: 16, minFontSize: 9, fontFamily: FONT_STACK, fontWeight: 600 })}
  ${fittedText({ text: `tokscale.ai/u/${data.user.username}`, x: width - px, y: 29, maxWidth: 260, fill: palette.muted, fontSize: 11, minFontSize: 8, fontFamily: FONT_STACK, textAnchor: "end" })}
  <text x="${px}" y="49" fill="${palette.muted}" font-size="10" font-family="${FONT_STACK}">${updated}</text>
  ${fittedText({ text: dateRange, x: width - px, y: 49, maxWidth: 260, fill: palette.muted, fontSize: 10, minFontSize: 8, fontFamily: FONT_STACK, textAnchor: "end" })}
  <line x1="${px}" y1="62.5" x2="${width - px}" y2="62.5" stroke="${palette.divider}" stroke-opacity="${palette.dividerOpacity}"/>
  <g data-contribution-grid="3d" transform="translate(${gridTranslateX} ${gridTranslateY}) scale(${gridScale})">${cubes}</g>
  <line x1="${px}" y1="${dataRailY + 0.5}" x2="${width - px}" y2="${dataRailY + 0.5}" stroke="${palette.divider}" stroke-opacity="${palette.dividerOpacity}"/>
  <g font-variant-numeric="tabular-nums">${metricMarkup}</g>
  <rect x="0.5" y="0.5" width="${width - 1}" height="${height - 1}" rx="${rx - 0.5}" fill="none" stroke="${palette.border}" stroke-opacity="${palette.borderOpacity}"/>
</svg>`;
}

export function renderIsometric3DErrorSvg(
  message: string,
  options: { theme?: EmbedTheme } = {},
): string {
  const theme: EmbedTheme = options.theme === "light" ? "light" : "dark";
  const palette = THEMES[theme];
  const width = 540;
  const height = 120;
  const rx = 12;
  const px = 24;

  return `<?xml version="1.0" encoding="UTF-8"?>
<svg data-template="3d-error" width="${width}" height="${height}" viewBox="0 0 ${width} ${height}" fill="none" xmlns="http://www.w3.org/2000/svg" role="img" aria-label="Tokscale 3D embed error">
  <title>Tokscale 3D embed error</title>
  <rect width="${width}" height="${height}" rx="${rx}" fill="${palette.surface}"/>
  <text x="${px}" y="27" fill="${palette.text}" font-size="13" font-weight="600" font-family="${FONT_STACK}">Tokscale</text>
  <text x="${width - px}" y="27" fill="${palette.muted}" font-size="11" font-family="${FONT_STACK}" text-anchor="end">tokscale.ai</text>
  <line x1="${px}" y1="40.5" x2="${width - px}" y2="40.5" stroke="${palette.divider}" stroke-opacity="${palette.dividerOpacity}"/>
  ${fittedText({ text: message, x: px, y: 68, maxWidth: width - px * 2, fill: palette.text, fontSize: 15, minFontSize: 9, fontFamily: FONT_STACK, fontWeight: 600 })}
  ${fittedText({ text: "Check the profile or try again later.", x: px, y: 91, maxWidth: width - px * 2, fill: palette.muted, fontSize: 12, minFontSize: 8, fontFamily: FONT_STACK })}
  <rect x="0.5" y="0.5" width="${width - 1}" height="${height - 1}" rx="${rx - 0.5}" fill="none" stroke="${palette.border}" stroke-opacity="${palette.borderOpacity}"/>
</svg>`;
}
