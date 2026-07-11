import type { UserEmbedStats, EmbedContributionDay } from "./getUserEmbedStats";
import { formatCurrency, formatNumber } from "../format";
import {
  type EmbedColorName,
  type EmbedNumberFormat,
  type EmbedRankFormat,
  type EmbedTheme,
  FIGTREE_FONT_IMPORT,
  FIGTREE_FONT_STACK,
  cardFooter,
  cardHeader,
  cardSurface,
  contributionPanel,
  divider,
  escapeXml,
  formatRank,
  getRankColor,
  resolvePalette,
} from "./embedShared";

export type { EmbedTheme } from "./embedShared";
export type EmbedSortBy = "tokens" | "cost";

export interface RenderProfileEmbedOptions {
  theme?: EmbedTheme;
  color?: EmbedColorName | null;
  compact?: boolean;
  /** Legacy flag: when true, both card size and numbers are compact. */
  compactNumbers?: boolean;
  tokensFormat?: EmbedNumberFormat;
  costFormat?: EmbedNumberFormat;
  rankFormat?: EmbedRankFormat;
  sortBy?: EmbedSortBy;
  contributions?: EmbedContributionDay[] | null;
}

const CHAR_WIDTH_RATIO = 0.6;

function fitValueFontSize(
  text: string,
  maxWidth: number,
  baseSize: number,
): number {
  const estimatedWidth = text.length * baseSize * CHAR_WIDTH_RATIO;
  if (estimatedWidth <= maxWidth) return baseSize;
  return Math.max(
    Math.ceil(baseSize * 0.5),
    Math.floor(baseSize * (maxWidth / estimatedWidth)),
  );
}

function renderProfileCardSvg(
  data: UserEmbedStats,
  options: RenderProfileEmbedOptions = {},
): string {
  const theme: EmbedTheme = options.theme === "light" ? "light" : "dark";
  const palette = resolvePalette(theme, options.color ?? null);
  const compact = options.compact ?? false;
  const compactNumbers = options.compactNumbers ?? false;
  const tokensFormat =
    options.tokensFormat ?? (compactNumbers ? "compact" : "full");
  const costFormat =
    options.costFormat ?? (compactNumbers ? "compact" : "full");
  const sortBy: EmbedSortBy = options.sortBy === "cost" ? "cost" : "tokens";
  const contributions =
    !compact && options.contributions?.length ? options.contributions : null;

  const width = compact ? 460 : 680;
  const x = compact ? 18 : 24;
  const right = width - x;
  const innerWidth = right - x;
  const graphY = 154;
  const graph = contributions
    ? contributionPanel({
        x,
        y: graphY,
        width: innerWidth,
        palette,
        contributions,
        showDayLabels: true,
        showLegend: true,
      })
    : null;
  const height = compact
    ? 162
    : graph
      ? Math.ceil(graphY + graph.height + 32)
      : 186;
  const headerY = compact ? 26 : 30;
  const metricTop = compact ? 70 : 78;
  const footerY = height - (compact ? 14 : 16);
  const fontBase = compact ? 22 : 28;
  const columnWidth = innerWidth / 3;

  const tokens = formatNumber(
    data.stats.totalTokens,
    tokensFormat === "compact",
  );
  const cost = formatCurrency(data.stats.totalCost, costFormat === "compact");
  const rank = data.stats.rank
    ? formatRank(
        data.stats.rank,
        data.stats.rankTotal ?? null,
        options.rankFormat,
      )
    : "N/A";
  const rankLabel = `Rank (${sortBy === "cost" ? "Cost" : "Tokens"})`;
  const rankColor = getRankColor(data.stats.rank, palette);
  const metrics = [
    { label: "Tokens", value: tokens, color: palette.brand },
    { label: "Cost", value: cost, color: palette.cost },
    { label: rankLabel, value: rank, color: rankColor },
  ];

  const metricSvg = metrics
    .map((metric, index) => {
      const metricX = x + index * columnWidth + (index === 0 ? 0 : 16);
      const available = columnWidth - (index === 0 ? 16 : 32);
      const valueSize = fitValueFontSize(metric.value, available, fontBase);
      return [
        index > 0
          ? `<line x1="${(x + index * columnWidth).toFixed(1)}" y1="${metricTop - 4}" x2="${(x + index * columnWidth).toFixed(1)}" y2="${metricTop + 54}" stroke="${palette.divider}"/>`
          : "",
        `<text x="${metricX.toFixed(1)}" y="${metricTop + 10}" fill="${palette.muted}" font-size="${compact ? 10 : 11}" font-weight="600" font-family="${FIGTREE_FONT_STACK}">${escapeXml(metric.label)}</text>`,
        `<text x="${metricX.toFixed(1)}" y="${metricTop + 43}" fill="${metric.color}" font-size="${valueSize}" font-weight="700" font-family="${FIGTREE_FONT_STACK}">${escapeXml(metric.value)}</text>`,
      ]
        .filter(Boolean)
        .join("\n  ");
    })
    .join("\n  ");

  return `<?xml version="1.0" encoding="UTF-8"?>
<svg data-template="classic" width="${width}" height="${height}" viewBox="0 0 ${width} ${height}" fill="none" xmlns="http://www.w3.org/2000/svg" role="img" aria-label="Tokscale profile stats for @${escapeXml(data.user.username)}">
  <defs><style>@import url('${FIGTREE_FONT_IMPORT}');</style></defs>
  ${cardSurface(width, height, palette)}
  ${cardHeader({
    username: data.user.username,
    displayName: data.user.displayName,
    palette,
    x,
    y: headerY,
    right,
  })}
  ${divider(x, right, compact ? 58 : 64, palette)}
  ${metricSvg}
  ${graph ? `${divider(x, right, 144, palette)}\n  ${graph.svg}` : ""}
  ${cardFooter({
    username: data.user.username,
    updatedAt: data.stats.updatedAt,
    palette,
    x,
    right,
    y: footerY,
  })}
</svg>`;
}

export function renderProfileEmbedSvg(
  data: UserEmbedStats,
  options: RenderProfileEmbedOptions = {},
): string {
  return renderProfileCardSvg(data, options);
}

export function renderProfileEmbedErrorSvg(
  message: string,
  options: RenderProfileEmbedOptions = {},
): string {
  const theme: EmbedTheme = options.theme === "light" ? "light" : "dark";
  const palette = resolvePalette(theme, options.color ?? null);
  const width = 540;
  const height = 120;
  const x = 24;
  const right = width - x;

  return `<?xml version="1.0" encoding="UTF-8"?>
<svg width="${width}" height="${height}" viewBox="0 0 ${width} ${height}" fill="none" xmlns="http://www.w3.org/2000/svg" role="img" aria-label="Tokscale embed error">
  <defs><style>@import url('${FIGTREE_FONT_IMPORT}');</style></defs>
  <g id="err-bg">
    ${cardSurface(width, height, palette)}
  </g>
  <rect x="${x}" y="22" width="3" height="14" rx="1.5" fill="${palette.brand}"/>
  <text x="${x + 12}" y="33" fill="${palette.muted}" font-size="10" font-weight="700" letter-spacing="0.1em" font-family="${FIGTREE_FONT_STACK}">Tokscale</text>
  ${divider(x, right, 46, palette)}
  <text x="${x}" y="72" fill="${palette.title}" font-size="15" font-weight="700" font-family="${FIGTREE_FONT_STACK}">${escapeXml(message)}</text>
  <text x="${x}" y="94" fill="${palette.muted}" font-size="11" font-family="${FIGTREE_FONT_STACK}">Check the username or submit usage first.</text>
  <text x="${right}" y="106" fill="${palette.muted}" font-size="10" text-anchor="end" font-family="${FIGTREE_FONT_STACK}">tokscale.ai</text>
</svg>`;
}
