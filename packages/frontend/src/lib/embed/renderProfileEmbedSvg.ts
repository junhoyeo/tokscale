import type { UserEmbedStats, EmbedContributionDay } from "./getUserEmbedStats";
import { formatCurrency, formatNumber } from "../format";
import {
  type EmbedColorName,
  type EmbedNumberFormat,
  type EmbedRankFormat,
  type EmbedTheme,
  FIGTREE_FONT_STACK,
  cardFooter,
  cardHeader,
  cardSurface,
  cardTextStyle,
  contributionPanel,
  divider,
  escapeXml,
  fittedText,
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
  const contributions = !compact ? (options.contributions ?? null) : null;

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
      return [
        index > 0
          ? `<line x1="${(x + index * columnWidth).toFixed(1)}" y1="${metricTop - 4}" x2="${(x + index * columnWidth).toFixed(1)}" y2="${metricTop + 54}" stroke="${palette.divider}"/>`
          : "",
        `<text x="${metricX.toFixed(1)}" y="${metricTop + 10}" fill="${palette.muted}" font-size="${compact ? 10 : 11}" font-weight="600" font-family="${FIGTREE_FONT_STACK}">${escapeXml(metric.label)}</text>`,
        fittedText({
          text: metric.value,
          x: Number(metricX.toFixed(1)),
          y: metricTop + 43,
          maxWidth: available,
          fill: metric.color,
          fontSize: fontBase,
          minFontSize: 9,
          fontWeight: 600,
        }),
      ]
        .filter(Boolean)
        .join("\n  ");
    })
    .join("\n  ");

  return `<?xml version="1.0" encoding="UTF-8"?>
<svg data-template="classic" width="${width}" height="${height}" viewBox="0 0 ${width} ${height}" fill="none" xmlns="http://www.w3.org/2000/svg" role="img" aria-label="Tokscale profile stats for @${escapeXml(data.user.username)}">
  ${cardTextStyle()}
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
  ${cardTextStyle()}
  <g id="err-bg">
    ${cardSurface(width, height, palette)}
  </g>
  <text x="${x}" y="32" fill="${palette.text}" font-size="14" font-weight="600" font-family="${FIGTREE_FONT_STACK}">Tokscale</text>
  ${divider(x, right, 46, palette)}
  ${fittedText({ text: message, x, y: 72, maxWidth: right - x, fill: palette.text, fontSize: 15, minFontSize: 9, fontWeight: 600 })}
  ${fittedText({ text: "Check the profile or try again later.", x, y: 94, maxWidth: right - x, fill: palette.muted, fontSize: 11, minFontSize: 8 })}
  <text x="${right}" y="106" fill="${palette.muted}" font-size="10" text-anchor="end" font-family="${FIGTREE_FONT_STACK}">tokscale.ai</text>
</svg>`;
}
