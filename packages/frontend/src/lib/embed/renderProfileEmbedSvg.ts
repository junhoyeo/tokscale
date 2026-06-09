import type { UserEmbedStats, EmbedContributionDay } from "./getUserEmbedStats";
import { formatNumber, formatCurrency } from "../format";
import {
  type ThemePalette,
  type EmbedTheme,
  type EmbedColorName,
  type EmbedNumberFormat,
  resolvePalette,
  FIGTREE_FONT_STACK,
  FIGTREE_FONT_IMPORT,
  brandIcon,
  formatDateLabel,
  getRankColor,
  escapeXml,
  formatRank,
  type EmbedRankFormat,
} from "./embedShared";

export type { EmbedTheme } from "./embedShared";
export type EmbedSortBy = "tokens" | "cost";

export interface RenderProfileEmbedOptions {
  theme?: EmbedTheme;
  color?: EmbedColorName | null;
  compact?: boolean;
  /** Legacy flag: when true, both card size and numbers are compact. */
  compactNumbers?: boolean;
  /** Token count format; overrides the legacy `compactNumbers` default. */
  tokensFormat?: EmbedNumberFormat;
  /** Cost format; overrides the legacy `compactNumbers` default. */
  costFormat?: EmbedNumberFormat;
  rankFormat?: EmbedRankFormat;
  sortBy?: EmbedSortBy;
  contributions?: EmbedContributionDay[] | null;
}

// Approximate character-width ratio for Figtree at various weights.
// Used to estimate rendered text width for dynamic positioning / collision.
const CHAR_WIDTH_RATIO = 0.6;
const APPROX_CHAR_WIDTH_13 = 8;

/**
 * Auto-scale font size so rendered text fits within maxWidth.
 * Uses a conservative character-width estimate to prevent overflow.
 */
function fitValueFontSize(text: string, maxWidth: number, baseSize: number): number {
  const estWidth = text.length * baseSize * CHAR_WIDTH_RATIO;
  if (estWidth <= maxWidth) return baseSize;
  return Math.max(Math.ceil(baseSize * 0.5), Math.floor(baseSize * (maxWidth / estWidth)));
}

function metricCard(args: {
  x: number;
  y: number;
  width: number;
  height: number;
  label: string;
  value: string;
  accentId: string;
  valueFill: string;
  palette: ThemePalette;
  compact: boolean;
}): string {
  const { x, y, width, height, label, value, accentId, valueFill, palette, compact } = args;
  const labelY = y + (compact ? 20 : 24);
  const baseValueSize = compact ? 22 : 28;
  const availableWidth = width - 28;
  const valueSize = fitValueFontSize(value, availableWidth, baseValueSize);
  const valueY = compact ? y + height - 12 : y + height - 14;

  return [
    `<rect x="${x}" y="${y}" width="${width}" height="${height}" rx="10" fill="${palette.metricBg}" stroke="${palette.metricBorder}"/>`,
    `<rect x="${x}" y="${y + 10}" width="2.5" height="${height - 20}" rx="1.25" fill="url(#${accentId})"/>`,
    `<text x="${x + 14}" y="${labelY}" fill="${palette.muted}" font-size="${compact ? 11 : 12}" font-weight="600" font-family="${FIGTREE_FONT_STACK}" letter-spacing="0.03em">${escapeXml(label)}</text>`,
    `<text x="${x + 14}" y="${valueY}" fill="${valueFill}" font-size="${valueSize}" font-weight="800" font-family="${FIGTREE_FONT_STACK}">${escapeXml(value)}</text>`,
  ].join("");
}

const GRAPH_CELL = 9;
const GRAPH_GAP = 2;
const GRAPH_STRIDE = GRAPH_CELL + GRAPH_GAP;
const GRAPH_DAY_LABEL_W = 26;
const GRAPH_MONTH_NAMES = ["Jan", "Feb", "Mar", "Apr", "May", "Jun", "Jul", "Aug", "Sep", "Oct", "Nov", "Dec"];

function renderContributionGrid(
  contributions: EmbedContributionDay[],
  palette: ThemePalette,
  x: number,
  y: number,
): string {
  const gridX = x + GRAPH_DAY_LABEL_W;
  const intensityMap = new Map<string, number>();
  for (const c of contributions) intensityMap.set(c.date, c.intensity);

  const colors = [palette.graphGrade0, palette.graphGrade1, palette.graphGrade2, palette.graphGrade3, palette.graphGrade4];

  const now = new Date();
  const today = new Date(Date.UTC(now.getUTCFullYear(), now.getUTCMonth(), now.getUTCDate()));
  const start = new Date(today);
  start.setUTCFullYear(start.getUTCFullYear() - 1);
  start.setUTCDate(start.getUTCDate() + 1);
  start.setUTCDate(start.getUTCDate() - start.getUTCDay());

  const diffDays = Math.ceil((today.getTime() - start.getTime()) / 86_400_000);
  const numWeeks = Math.ceil((diffDays + 1) / 7);

  const monthLabelY = y + 9;
  const gridTopY = y + 14;
  let svg = "";

  let lastMonth = -1;
  for (let w = 0; w < numWeeks; w++) {
    const d = new Date(start);
    d.setUTCDate(d.getUTCDate() + w * 7);
    const m = d.getUTCMonth();
    if (m !== lastMonth) {
      lastMonth = m;
      svg += `<text x="${gridX + w * GRAPH_STRIDE}" y="${monthLabelY}" fill="${palette.muted}" font-size="9" font-family="${FIGTREE_FONT_STACK}">${GRAPH_MONTH_NAMES[m]}</text>`;
    }
  }

  const dayLabels: [number, string][] = [[1, "Mon"], [3, "Wed"], [5, "Fri"]];
  for (const [row, label] of dayLabels) {
    svg += `<text x="${x}" y="${gridTopY + row * GRAPH_STRIDE + GRAPH_CELL - 1}" fill="${palette.muted}" font-size="9" font-family="${FIGTREE_FONT_STACK}">${label}</text>`;
  }

  for (let w = 0; w < numWeeks; w++) {
    for (let d = 0; d < 7; d++) {
      const date = new Date(start);
      date.setUTCDate(date.getUTCDate() + w * 7 + d);
      if (date > today) continue;
      const dateStr = date.toISOString().split("T")[0];
      const intensity = intensityMap.get(dateStr) ?? 0;
      svg += `<rect x="${gridX + w * GRAPH_STRIDE}" y="${gridTopY + d * GRAPH_STRIDE}" width="${GRAPH_CELL}" height="${GRAPH_CELL}" rx="2" fill="${colors[intensity]}"/>`;
    }
  }

  const legendY = gridTopY + 7 * GRAPH_STRIDE + 6;
  const gridRightX = gridX + (numWeeks - 1) * GRAPH_STRIDE + GRAPH_CELL;
  const legendW = 28 + 5 * GRAPH_STRIDE + 28;
  const legendX = gridRightX - legendW;
  svg += `<text x="${legendX}" y="${legendY + GRAPH_CELL - 1}" fill="${palette.muted}" font-size="9" font-family="${FIGTREE_FONT_STACK}">Less</text>`;
  let lx = legendX + 28;
  for (let i = 0; i < 5; i++) {
    svg += `<rect x="${lx}" y="${legendY}" width="${GRAPH_CELL}" height="${GRAPH_CELL}" rx="2" fill="${colors[i]}"/>`;
    lx += GRAPH_STRIDE;
  }
  svg += `<text x="${lx + 3}" y="${legendY + GRAPH_CELL - 1}" fill="${palette.muted}" font-size="9" font-family="${FIGTREE_FONT_STACK}">More</text>`;

  return svg;
}

function renderProfileCardSvg(data: UserEmbedStats, options: RenderProfileEmbedOptions = {}): string {
  const theme: EmbedTheme = options.theme === "light" ? "light" : "dark";
  const compact = options.compact ?? false;
  const compactNumbers = options.compactNumbers ?? false;
  const tokensFormat: EmbedNumberFormat = options.tokensFormat ?? (compactNumbers ? "compact" : "full");
  const costFormat: EmbedNumberFormat = options.costFormat ?? (compactNumbers ? "compact" : "full");
  const sortBy: EmbedSortBy = options.sortBy === "cost" ? "cost" : "tokens";
  const contributions = (!compact && options.contributions) ? options.contributions : null;
  const palette = resolvePalette(theme, options.color ?? null);

  const width = compact ? 460 : 680;
  const height = (compact ? 162 : 186) + (contributions ? 120 : 0);
  const rx = 16;
  const px = compact ? 18 : 24;

  const brandY = compact ? 26 : 30;
  const usernameY = compact ? 44 : 52;
  const dividerY = compact ? 54 : 64;
  const metricsY = compact ? 64 : 76;
  const metricH = compact ? 58 : 68;
  const footerY = height - (compact ? 14 : 16);

  const username = `@${data.user.username}`;
  const displayNameRaw = data.user.displayName;
  const displayName = displayNameRaw ? escapeXml(displayNameRaw) : null;
  const tokens = formatNumber(data.stats.totalTokens, tokensFormat === "compact");
  const cost = formatCurrency(data.stats.totalCost, costFormat === "compact");
  const rank = data.stats.rank
    ? formatRank(data.stats.rank, data.stats.rankTotal ?? null, options.rankFormat)
    : "N/A";
  const updated = escapeXml(formatDateLabel(data.stats.updatedAt));
  const rankLabel = compact
    ? `Rank · ${sortBy === "cost" ? "Cost" : "Tokens"}`
    : `Rank (${sortBy === "cost" ? "Cost" : "Tokens"})`;

  const usernameFontSize = compact ? 15 : 17;
  const usernameEstWidth = username.length * usernameFontSize * CHAR_WIDTH_RATIO;
  const displayNameX = Math.round(px + usernameEstWidth + 8);
  const displayNameEstWidth = displayNameRaw ? displayNameRaw.length * APPROX_CHAR_WIDTH_13 : 0;
  const showDisplayName = Boolean(displayName) && displayNameX + displayNameEstWidth < width - px - 12;

  const metricsGap = compact ? 8 : 10;
  const metricsW = width - px * 2;
  const metricW = (metricsW - metricsGap * 2) / 3;
  const rankColor = getRankColor(data.stats.rank, palette);
  const rankAccent = (data.stats.rank && data.stats.rank <= 3)
    ? getRankColor(data.stats.rank, palette)
    : palette.accentRank;

  return `<?xml version="1.0" encoding="UTF-8"?>
<svg width="${width}" height="${height}" viewBox="0 0 ${width} ${height}" fill="none" xmlns="http://www.w3.org/2000/svg" role="img" aria-label="Tokscale profile stats for ${escapeXml(username)}">
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
    <linearGradient id="token-grad" x1="0" y1="0" x2="1" y2="0">
      <stop offset="0" stop-color="${palette.tokenStart}"/>
      <stop offset="1" stop-color="${palette.tokenEnd}"/>
    </linearGradient>
    <linearGradient id="divider-grad" x1="${px}" y1="0" x2="${width - px}" y2="0" gradientUnits="userSpaceOnUse">
      <stop offset="0" stop-color="${palette.divider}" stop-opacity="0"/>
      <stop offset="0.5" stop-color="${palette.divider}" stop-opacity="0.6"/>
      <stop offset="1" stop-color="${palette.divider}" stop-opacity="0"/>
    </linearGradient>
    <linearGradient id="acc-tokens" x1="0" y1="0" x2="0" y2="1">
      <stop offset="0.15" stop-color="${palette.accentTokens}" stop-opacity="0.9"/>
      <stop offset="1" stop-color="${palette.accentTokens}" stop-opacity="0"/>
    </linearGradient>
    <linearGradient id="acc-cost" x1="0" y1="0" x2="0" y2="1">
      <stop offset="0.15" stop-color="${palette.accentCost}" stop-opacity="0.9"/>
      <stop offset="1" stop-color="${palette.accentCost}" stop-opacity="0"/>
    </linearGradient>
    <linearGradient id="acc-rank" x1="0" y1="0" x2="0" y2="1">
      <stop offset="0.15" stop-color="${rankAccent}" stop-opacity="0.9"/>
      <stop offset="1" stop-color="${rankAccent}" stop-opacity="0"/>
    </linearGradient>
    <clipPath id="card-clip">
      <rect width="${width}" height="${height}" rx="${rx}"/>
    </clipPath>
  </defs>
  <rect width="${width}" height="${height}" rx="${rx}" fill="url(#bg)"/>
  <rect x="0.5" y="0.5" width="${width - 1}" height="${height - 1}" rx="${rx - 0.5}" fill="none" stroke="${palette.border}"/>
  <rect width="${width}" height="${height}" rx="${rx}" fill="url(#glow)" clip-path="url(#card-clip)"/>
  ${brandIcon(px, brandY, palette.brand)}
  <text x="${px + 18}" y="${brandY}" fill="${palette.muted}" font-size="12" font-weight="600" font-family="${FIGTREE_FONT_STACK}">Tokscale Stats</text>
  <text x="${px}" y="${usernameY}" fill="${palette.text}" font-size="${usernameFontSize}" font-weight="700" font-family="${FIGTREE_FONT_STACK}">${escapeXml(username)}</text>
  ${
    showDisplayName
      ? `<text x="${displayNameX}" y="${usernameY}" fill="${palette.muted}" font-size="13" font-family="${FIGTREE_FONT_STACK}">${displayName}</text>`
      : ""
  }
  <rect x="${px}" y="${dividerY}" width="${metricsW}" height="1" fill="url(#divider-grad)"/>
  ${metricCard({
    x: px,
    y: metricsY,
    width: metricW,
    height: metricH,
    label: "Tokens",
    value: tokens,
    accentId: "acc-tokens",
    valueFill: "url(#token-grad)",
    palette,
    compact,
  })}
  ${metricCard({
    x: px + metricW + metricsGap,
    y: metricsY,
    width: metricW,
    height: metricH,
    label: "Cost",
    value: cost,
    accentId: "acc-cost",
    valueFill: palette.cost,
    palette,
    compact,
  })}
  ${metricCard({
    x: px + metricW * 2 + metricsGap * 2,
    y: metricsY,
    width: metricW,
    height: metricH,
    label: rankLabel,
    value: rank,
    accentId: "acc-rank",
    valueFill: rankColor,
    palette,
    compact,
  })}
  ${contributions ? renderContributionGrid(contributions, palette, px, metricsY + metricH + 12) : ""}
  <text x="${px}" y="${footerY}" fill="${palette.muted}" font-size="11" font-family="${FIGTREE_FONT_STACK}">${updated}</text>
  <text x="${width - px}" y="${footerY}" fill="${palette.muted}" font-size="11" font-family="${FIGTREE_FONT_STACK}" text-anchor="end">tokscale.ai/u/${escapeXml(
    data.user.username
  )}</text>
</svg>`;
}

export function renderProfileEmbedSvg(
  data: UserEmbedStats,
  options: RenderProfileEmbedOptions = {}
): string {
  return renderProfileCardSvg(data, options);
}

export function renderProfileEmbedErrorSvg(
  message: string,
  options: RenderProfileEmbedOptions = {}
): string {
  const theme: EmbedTheme = options.theme === "light" ? "light" : "dark";
  const palette = resolvePalette(theme, options.color ?? null);
  const safeMessage = escapeXml(message);
  const width = 540;
  const height = 120;
  const rx = 16;
  const px = 24;

  return `<?xml version="1.0" encoding="UTF-8"?>
<svg width="${width}" height="${height}" viewBox="0 0 ${width} ${height}" fill="none" xmlns="http://www.w3.org/2000/svg" role="img" aria-label="Tokscale embed error">
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
  <text x="${px}" y="90" fill="${palette.muted}" font-size="12" font-family="${FIGTREE_FONT_STACK}">Try checking the username or submitting usage first.</text>
  <text x="${width - px}" y="108" fill="${palette.muted}" font-size="11" font-family="${FIGTREE_FONT_STACK}" text-anchor="end">tokscale.ai</text>
</svg>`;
}
