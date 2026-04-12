import type { UserEmbedStats, EmbedContributionDay } from "./getUserEmbedStats";
import { escapeXml, formatNumber, formatCurrency } from "../format";

export type EmbedTheme = "dark" | "light";

type ThemePalette = {
  bgStart: string;
  bgEnd: string;
  border: string;
  glowColor: string;
  glowOpacity: number;
  title: string;
  text: string;
  muted: string;
  brand: string;
  accent: string;
  divider: string;
  boxBg: string;
  boxBorder: string;
  graphGrade0: string;
  graphGrade1: string;
  graphGrade2: string;
  graphGrade3: string;
  graphGrade4: string;
};

const THEMES: Record<EmbedTheme, ThemePalette> = {
  dark: {
    bgStart: "#0D1117",
    bgEnd: "#010409",
    border: "#30363D",
    glowColor: "#388BFD",
    glowOpacity: 0.07,
    title: "#F0F6FC",
    text: "#E6EDF3",
    muted: "#8B949E",
    brand: "#58A6FF",
    accent: "#79B8FF",
    divider: "#30363D",
    boxBg: "#1A212A",
    boxBorder: "#1E2733",
    graphGrade0: "#1A212A",
    graphGrade1: "#79b8ff",
    graphGrade2: "#388bfd",
    graphGrade3: "#1f6feb",
    graphGrade4: "#0d419d",
  },
  light: {
    bgStart: "#FFFFFF",
    bgEnd: "#F6F8FA",
    border: "#D0D7DE",
    glowColor: "#0969DA",
    glowOpacity: 0.04,
    title: "#1F2328",
    text: "#1F2328",
    muted: "#656D76",
    brand: "#0969DA",
    accent: "#0969DA",
    divider: "#D0D7DE",
    boxBg: "#F6F8FA",
    boxBorder: "#D0D7DE",
    graphGrade0: "#EBEDF0",
    graphGrade1: "#79b8ff",
    graphGrade2: "#388bfd",
    graphGrade3: "#1f6feb",
    graphGrade4: "#0d419d",
  },
};

const FIGTREE_FONT_STACK = "Figtree, -apple-system, BlinkMacSystemFont, Segoe UI, sans-serif";
const FIGTREE_FONT_IMPORT = "https://fonts.googleapis.com/css2?family=Figtree:wght@400;600;700;800&amp;display=swap";

const CELL = 10;
const MAX_HEIGHT = 40;
const INTENSITY_HEIGHTS: readonly number[] = [2, 10, 20, 32, MAX_HEIGHT];

/**
 * 2:1 dimetric projection: col (X) goes screen right-down,
 * row (Y) goes screen left-down, z goes screen up.
 */
function toScreen(col: number, row: number, z: number = 0): { x: number; y: number } {
  return {
    x: (col - row) * CELL,
    y: (col + row) * (CELL / 2) - z,
  };
}

function darkenHex(hex: string, factor: number): string {
  const r = parseInt(hex.slice(1, 3), 16);
  const g = parseInt(hex.slice(3, 5), 16);
  const b = parseInt(hex.slice(5, 7), 16);
  const clamp = (v: number) => Math.max(0, Math.min(255, Math.round(v * factor)));
  return (
    "#" +
    clamp(r).toString(16).padStart(2, "0") +
    clamp(g).toString(16).padStart(2, "0") +
    clamp(b).toString(16).padStart(2, "0")
  );
}

function pt(ox: number, oy: number, p: { x: number; y: number }): string {
  return `${(ox + p.x).toFixed(1)},${(oy + p.y).toFixed(1)}`;
}

function renderCube(
  ox: number,
  oy: number,
  col: number,
  row: number,
  h: number,
  color: string,
): string {
  const tN = toScreen(col, row, h);
  const tE = toScreen(col + 1, row, h);
  const tS = toScreen(col + 1, row + 1, h);
  const tW = toScreen(col, row + 1, h);
  const bE = toScreen(col + 1, row, 0);
  const bS = toScreen(col + 1, row + 1, 0);
  const bW = toScreen(col, row + 1, 0);

  const topColor = color;
  const leftColor = darkenHex(color, 0.7);
  const rightColor = darkenHex(color, 0.85);

  // Face orientation: left=Y+, right=X+, top=Z+
  const left = `<polygon points="${pt(ox, oy, tW)} ${pt(ox, oy, tS)} ${pt(ox, oy, bS)} ${pt(ox, oy, bW)}" fill="${leftColor}" stroke="${leftColor}" stroke-width="0.5"/>`;
  const right = `<polygon points="${pt(ox, oy, tE)} ${pt(ox, oy, bE)} ${pt(ox, oy, bS)} ${pt(ox, oy, tS)}" fill="${rightColor}" stroke="${rightColor}" stroke-width="0.5"/>`;
  const top = `<polygon points="${pt(ox, oy, tN)} ${pt(ox, oy, tE)} ${pt(ox, oy, tS)} ${pt(ox, oy, tW)}" fill="${topColor}" stroke="${topColor}" stroke-width="0.5"/>`;

  return left + right + top;
}

function brandIcon(x: number, baselineY: number, color: string): string {
  const top = baselineY - 12;
  return [
    `<rect x="${x}" y="${top + 8}" width="3" height="6" rx="1" fill="${color}" opacity="0.45"/>`,
    `<rect x="${x + 5}" y="${top}" width="3" height="14" rx="1" fill="${color}"/>`,
    `<rect x="${x + 10}" y="${top + 4}" width="3" height="10" rx="1" fill="${color}" opacity="0.7"/>`,
  ].join("");
}

function formatDateLabel(value: string | null): string {
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

function formatShortDate(dateStr: string): string {
  const d = new Date(dateStr + "T00:00:00Z");
  if (Number.isNaN(d.getTime())) return dateStr;
  const mm = String(d.getUTCMonth() + 1).padStart(2, "0");
  const dd = String(d.getUTCDate()).padStart(2, "0");
  return `${mm}/${dd}`;
}

function computeStreaks(contributions: EmbedContributionDay[]): { longest: number; current: number } {
  const activeSet = new Set<string>();
  for (const c of contributions) {
    if (c.intensity > 0) activeSet.add(c.date);
  }
  if (activeSet.size === 0) return { longest: 0, current: 0 };

  const sorted = [...activeSet].sort();
  let longest = 1;
  let run = 1;
  for (let i = 1; i < sorted.length; i++) {
    const prev = new Date(sorted[i - 1] + "T00:00:00Z");
    const curr = new Date(sorted[i] + "T00:00:00Z");
    const diff = (curr.getTime() - prev.getTime()) / 86_400_000;
    if (diff === 1) {
      run++;
      if (run > longest) longest = run;
    } else {
      run = 1;
    }
  }

  let current = 0;
  const now = new Date();
  const todayStr = new Date(Date.UTC(now.getUTCFullYear(), now.getUTCMonth(), now.getUTCDate()))
    .toISOString()
    .split("T")[0];
  let cursor = todayStr;
  while (activeSet.has(cursor)) {
    current++;
    const d = new Date(cursor + "T00:00:00Z");
    d.setUTCDate(d.getUTCDate() - 1);
    cursor = d.toISOString().split("T")[0];
  }

  return { longest, current };
}

function renderStatsBox(
  x: number,
  y: number,
  w: number,
  title: string,
  items: Array<{ value: string; label: string; sub?: string }>,
  footer: string | null,
  palette: ThemePalette,
): string {
  const titleH = 20;
  const itemH = 42;
  const footerH = footer ? 22 : 0;
  const boxPad = 10;
  const innerH = items.length * itemH;
  const totalH = titleH + boxPad * 2 + innerH + footerH;

  const boxY = y + titleH + 4;
  const boxInnerH = innerH + boxPad * 2 + footerH;

  let svg = "";
  svg += `<text x="${x}" y="${y + 13}" fill="${palette.text}" font-size="12" font-weight="700" font-family="${FIGTREE_FONT_STACK}">${escapeXml(title)}</text>`;
  svg += `<rect x="${x}" y="${boxY}" width="${w}" height="${boxInnerH}" rx="8" fill="${palette.boxBg}" stroke="${palette.boxBorder}" opacity="0.92"/>`;

  let iy = boxY + boxPad;
  for (const item of items) {
    svg += `<text x="${x + boxPad}" y="${iy + 16}" fill="${palette.accent}" font-size="15" font-weight="700" font-family="${FIGTREE_FONT_STACK}">${escapeXml(item.value)}</text>`;
    svg += `<text x="${x + boxPad}" y="${iy + 30}" fill="${palette.text}" font-size="10" font-weight="600" font-family="${FIGTREE_FONT_STACK}">${escapeXml(item.label)}</text>`;
    if (item.sub) {
      svg += `<text x="${x + boxPad}" y="${iy + 40}" fill="${palette.muted}" font-size="9" font-family="${FIGTREE_FONT_STACK}">${escapeXml(item.sub)}</text>`;
    }
    iy += itemH;
  }

  if (footer) {
    svg += `<text x="${x + boxPad}" y="${iy + 14}" fill="${palette.muted}" font-size="10" font-family="${FIGTREE_FONT_STACK}">${footer}</text>`;
  }

  return svg;
}

export function renderIsometric3DEmbedSvg(
  data: UserEmbedStats,
  contributions: EmbedContributionDay[],
  options: { theme?: EmbedTheme } = {},
): string {
  const theme: EmbedTheme = options.theme === "light" ? "light" : "dark";
  const palette = THEMES[theme];

  const intensityMap = new Map<string, number>();
  for (const c of contributions) intensityMap.set(c.date, c.intensity);

  const colors = [
    palette.graphGrade0,
    palette.graphGrade1,
    palette.graphGrade2,
    palette.graphGrade3,
    palette.graphGrade4,
  ];

  const now = new Date();
  const today = new Date(Date.UTC(now.getUTCFullYear(), now.getUTCMonth(), now.getUTCDate()));
  const start = new Date(today);
  start.setUTCFullYear(start.getUTCFullYear() - 1);
  start.setUTCDate(start.getUTCDate() + 1);
  start.setUTCDate(start.getUTCDate() - start.getUTCDay());

  const diffDays = Math.ceil((today.getTime() - start.getTime()) / 86_400_000);
  const numWeeks = Math.ceil((diffDays + 1) / 7);

  const px = 24;
  const width = 680;
  const headerH = 70;
  const gridXExtent = (numWeeks + 7) * CELL;
  const gridYExtent = (numWeeks + 7) * (CELL / 2) + MAX_HEIGHT;
  const footerH = 30;
  const height = headerH + gridYExtent + footerH;
  const rx = 16;

  const gridOriginX = px + 7 * CELL + Math.max(0, (width - 2 * px - gridXExtent) / 2);
  const gridOriginY = headerH + MAX_HEIGHT + 4;

  let cubes = "";
  for (let w = 0; w < numWeeks; w++) {
    for (let d = 0; d < 7; d++) {
      const date = new Date(start);
      date.setUTCDate(date.getUTCDate() + w * 7 + d);
      if (date > today) continue;

      const dateStr = date.toISOString().split("T")[0];
      const intensity = (intensityMap.get(dateStr) ?? 0) as 0 | 1 | 2 | 3 | 4;
      const h = INTENSITY_HEIGHTS[intensity];
      const color = colors[intensity];

      cubes += renderCube(gridOriginX, gridOriginY, w, d, h, color);
    }
  }

  const username = `@${data.user.username}`;
  const tokens = formatNumber(data.stats.totalTokens, true);
  const cost = formatCurrency(data.stats.totalCost, true);
  const rank = data.stats.rank ? `#${data.stats.rank}` : "\u2014";
  const updated = escapeXml(formatDateLabel(data.stats.updatedAt));
  const footerY = height - 14;

  const activeDays = contributions.filter((c) => c.intensity > 0).length;
  const activeDates = contributions.filter((c) => c.intensity > 0).map((c) => c.date).sort();
  const dateRange =
    activeDates.length >= 2
      ? `${formatShortDate(activeDates[0])} \u2192 ${formatShortDate(activeDates[activeDates.length - 1])}`
      : activeDates.length === 1
        ? formatShortDate(activeDates[0])
        : "";
  const streaks = computeStreaks(contributions);

  const statsBoxW = 148;
  const tokenUsageX = width - px - statsBoxW;
  const tokenUsageY = headerH + 8;
  const tokenUsageItems: Array<{ value: string; label: string; sub?: string }> = [
    { value: cost, label: "Total", sub: dateRange },
    { value: tokens, label: "Tokens", sub: `${activeDays} active days` },
  ];
  const tokenUsageFooter = `Rank <tspan fill="${palette.accent}" font-weight="700">${escapeXml(rank)}</tspan>`;

  const streaksBoxW = 130;
  const streaksX = px;
  const streaksY = height - footerH - 120;
  const streaksItems: Array<{ value: string; label: string }> = [
    { value: `${streaks.longest} days`, label: "Longest" },
    { value: `${streaks.current} days`, label: "Current" },
  ];

  return `<?xml version="1.0" encoding="UTF-8"?>
<svg width="${width}" height="${height}" viewBox="0 0 ${width} ${height}" fill="none" xmlns="http://www.w3.org/2000/svg" role="img" aria-label="Tokscale 3D contribution graph for ${escapeXml(username)}">
  <defs>
    <style>@import url('${FIGTREE_FONT_IMPORT}');</style>
    <linearGradient id="bg" x1="0" y1="0" x2="${width}" y2="${height}" gradientUnits="userSpaceOnUse">
      <stop offset="0" stop-color="${palette.bgStart}"/>
      <stop offset="1" stop-color="${palette.bgEnd}"/>
    </linearGradient>
    <radialGradient id="glow" cx="0.82" cy="0.12" r="0.55">
      <stop offset="0" stop-color="${palette.glowColor}" stop-opacity="${palette.glowOpacity}"/>
      <stop offset="1" stop-color="${palette.glowColor}" stop-opacity="0"/>
    </radialGradient>
    <linearGradient id="divider-grad" x1="${px}" y1="0" x2="${width - px}" y2="0" gradientUnits="userSpaceOnUse">
      <stop offset="0" stop-color="${palette.divider}" stop-opacity="0"/>
      <stop offset="0.5" stop-color="${palette.divider}" stop-opacity="0.6"/>
      <stop offset="1" stop-color="${palette.divider}" stop-opacity="0"/>
    </linearGradient>
    <clipPath id="card-clip">
      <rect width="${width}" height="${height}" rx="${rx}"/>
    </clipPath>
  </defs>
  <rect width="${width}" height="${height}" rx="${rx}" fill="url(#bg)"/>
  <rect x="0.5" y="0.5" width="${width - 1}" height="${height - 1}" rx="${rx - 0.5}" fill="none" stroke="${palette.border}"/>
  <rect width="${width}" height="${height}" rx="${rx}" fill="url(#glow)" clip-path="url(#card-clip)"/>
  ${brandIcon(px, 30, palette.brand)}
  <text x="${px + 18}" y="30" fill="${palette.muted}" font-size="12" font-weight="600" font-family="${FIGTREE_FONT_STACK}">Tokscale Stats</text>
  <text x="${px}" y="52" fill="${palette.text}" font-size="17" font-weight="700" font-family="${FIGTREE_FONT_STACK}">${escapeXml(username)}</text>
  <rect x="${px}" y="62" width="${width - px * 2}" height="1" fill="url(#divider-grad)"/>
  ${cubes}
  ${renderStatsBox(tokenUsageX, tokenUsageY, statsBoxW, "Token Usage", tokenUsageItems, tokenUsageFooter, palette)}
  ${renderStatsBox(streaksX, streaksY, streaksBoxW, "Streaks", streaksItems, null, palette)}
  <text x="${px}" y="${footerY}" fill="${palette.muted}" font-size="11" font-family="${FIGTREE_FONT_STACK}">${updated}</text>
  <text x="${width - px}" y="${footerY}" fill="${palette.muted}" font-size="11" font-family="${FIGTREE_FONT_STACK}" text-anchor="end">tokscale.ai/u/${escapeXml(data.user.username)}</text>
</svg>`;
}

export function renderIsometric3DErrorSvg(
  message: string,
  options: { theme?: EmbedTheme } = {},
): string {
  const theme: EmbedTheme = options.theme === "light" ? "light" : "dark";
  const palette = THEMES[theme];
  const safeMessage = escapeXml(message);
  const width = 540;
  const height = 120;
  const rx = 16;
  const px = 24;

  return `<?xml version="1.0" encoding="UTF-8"?>
<svg width="${width}" height="${height}" viewBox="0 0 ${width} ${height}" fill="none" xmlns="http://www.w3.org/2000/svg" role="img" aria-label="Tokscale 3D embed error">
  <defs>
    <style>@import url('${FIGTREE_FONT_IMPORT}');</style>
    <linearGradient id="err-bg" x1="0" y1="0" x2="${width}" y2="${height}" gradientUnits="userSpaceOnUse">
      <stop offset="0" stop-color="${palette.bgStart}"/>
      <stop offset="1" stop-color="${palette.bgEnd}"/>
    </linearGradient>
    <linearGradient id="err-divider" x1="${px}" y1="0" x2="${width - px}" y2="0" gradientUnits="userSpaceOnUse">
      <stop offset="0" stop-color="${palette.divider}" stop-opacity="0"/>
      <stop offset="0.5" stop-color="${palette.divider}" stop-opacity="0.6"/>
      <stop offset="1" stop-color="${palette.divider}" stop-opacity="0"/>
    </linearGradient>
  </defs>
  <rect width="${width}" height="${height}" rx="${rx}" fill="url(#err-bg)"/>
  <rect x="0.5" y="0.5" width="${width - 1}" height="${height - 1}" rx="${rx - 0.5}" fill="none" stroke="${palette.border}"/>
  ${brandIcon(px, 30, palette.brand)}
  <text x="${px + 18}" y="30" fill="${palette.muted}" font-size="12" font-weight="600" font-family="${FIGTREE_FONT_STACK}">Tokscale Stats</text>
  <rect x="${px}" y="40" width="${width - px * 2}" height="1" fill="url(#err-divider)"/>
  <text x="${px}" y="66" fill="${palette.title}" font-size="15" font-weight="700" font-family="${FIGTREE_FONT_STACK}">${safeMessage}</text>
  <text x="${px}" y="90" fill="${palette.muted}" font-size="12" font-family="${FIGTREE_FONT_STACK}">Submit your usage data first, then try again with view=3d.</text>
  <text x="${width - px}" y="108" fill="${palette.muted}" font-size="11" font-family="${FIGTREE_FONT_STACK}" text-anchor="end">tokscale.ai</text>
</svg>`;
}
