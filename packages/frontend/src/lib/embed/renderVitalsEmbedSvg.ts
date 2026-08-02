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
  divider,
  escapeXml,
  fittedText,
  formatRank,
  getRankColor,
  layoutContributions,
  resolvePalette,
} from "./embedShared";

export interface RenderVitalsEmbedOptions {
  theme?: EmbedTheme;
  color?: EmbedColorName | null;
  sortBy?: "tokens" | "cost";
  tokensFormat?: EmbedNumberFormat;
  costFormat?: EmbedNumberFormat;
  rankFormat?: EmbedRankFormat;
  contributions?: EmbedContributionDay[] | null;
}

const W = 520;
const H = 250;
const PAD = 24;

export function renderVitalsEmbedSvg(
  data: UserEmbedStats,
  options: RenderVitalsEmbedOptions = {},
): string {
  const theme: EmbedTheme = options.theme === "light" ? "light" : "dark";
  const palette = resolvePalette(theme, options.color ?? null);
  const contributions = options.contributions ?? [];
  const layout = layoutContributions(contributions);
  const rangeCells = layout.cells.filter(({ inRange }) => inRange);
  const avgIntensity = rangeCells.length
    ? rangeCells.reduce((sum, day) => sum + day.intensity, 0) /
      rangeCells.length
    : 0;
  const rank = data.stats.rank;
  const rankTotal = data.stats.rankTotal ?? null;
  const rankText = rank
    ? formatRank(rank, rankTotal, options.rankFormat)
    : "Unranked";
  const activityCoverage = Math.min(
    1,
    layout.activeDays / Math.max(1, layout.rangeDays),
  );
  const right = W - PAD;
  const tokens = formatNumber(
    data.stats.totalTokens,
    (options.tokensFormat ?? "compact") === "compact",
  );
  const cost = formatCurrency(
    data.stats.totalCost,
    (options.costFormat ?? "compact") === "compact",
  );

  return `<?xml version="1.0" encoding="UTF-8"?>
<svg data-template="vitals" width="${W}" height="${H}" viewBox="0 0 ${W} ${H}" fill="none" xmlns="http://www.w3.org/2000/svg" role="img" aria-label="Tokscale usage signals for @${escapeXml(data.user.username)}">
  ${cardTextStyle()}
  ${cardSurface(W, H, palette)}
  ${cardHeader({
    username: data.user.username,
    displayName: data.user.displayName,
    palette,
    x: PAD,
    y: 27,
    right,
  })}
  ${divider(PAD, right, 62, palette)}
  <text x="${PAD}" y="82" fill="${palette.muted}" font-size="10" font-weight="600" font-family="${FIGTREE_FONT_STACK}">Active days · 1y</text>
  ${fittedText({
    text: String(layout.activeDays),
    x: PAD,
    y: 111,
    maxWidth: 180,
    fill: palette.brand,
    fontSize: 30,
    minFontSize: 10,
    fontWeight: 600,
  })}
  <rect x="${PAD}" y="121" width="${right - PAD}" height="5" rx="2.5" fill="${palette.graphGrade0}"/>
  <rect x="${PAD}" y="121" width="${((right - PAD) * activityCoverage).toFixed(1)}" height="5" rx="2.5" fill="${palette.brand}"/>
  <text x="${right}" y="141" fill="${palette.muted}" font-size="10" text-anchor="end" font-family="${FIGTREE_FONT_STACK}">${Math.round(activityCoverage * 100)}% of trailing year</text>
  ${divider(PAD, right, 150, palette)}
  <text x="${PAD}" y="166" fill="${palette.muted}" font-size="10" font-weight="600" font-family="${FIGTREE_FONT_STACK}">Tokens</text>
  ${fittedText({
    text: tokens,
    x: PAD,
    y: 186,
    maxWidth: 210,
    fill: palette.text,
    fontSize: 16,
    minFontSize: 8,
    fontWeight: 600,
  })}
  <text x="264" y="166" fill="${palette.muted}" font-size="10" font-weight="600" font-family="${FIGTREE_FONT_STACK}">Cost</text>
  ${fittedText({
    text: cost,
    x: right,
    y: 186,
    maxWidth: 210,
    fill: palette.cost,
    fontSize: 16,
    minFontSize: 8,
    fontWeight: 600,
    textAnchor: "end",
  })}
  <text x="${PAD}" y="201" fill="${palette.muted}" font-size="10" font-weight="600" font-family="${FIGTREE_FONT_STACK}">Average intensity · 1y</text>
  ${fittedText({
    text: `${avgIntensity.toFixed(1)} / 4`,
    x: PAD,
    y: 219,
    maxWidth: 210,
    fill: palette.text,
    fontSize: 13,
    minFontSize: 8,
    fontWeight: 600,
  })}
  <text x="264" y="201" fill="${palette.muted}" font-size="10" font-weight="600" font-family="${FIGTREE_FONT_STACK}">Rank · ${options.sortBy === "cost" ? "cost" : "tokens"}</text>
  ${fittedText({
    text: rankText,
    x: right,
    y: 219,
    maxWidth: 210,
    fill: getRankColor(rank, palette),
    fontSize: 13,
    minFontSize: 8,
    fontWeight: 600,
    textAnchor: "end",
  })}
  ${cardFooter({
    updatedAt: data.stats.updatedAt,
    palette,
    x: PAD,
    right,
    y: H - 16,
  })}
</svg>`;
}
