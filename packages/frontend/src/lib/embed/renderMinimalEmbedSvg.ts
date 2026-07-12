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

export interface RenderMinimalEmbedOptions {
  theme?: EmbedTheme;
  color?: EmbedColorName | null;
  sortBy?: "tokens" | "cost";
  tokensFormat?: EmbedNumberFormat;
  costFormat?: EmbedNumberFormat;
  rankFormat?: EmbedRankFormat;
  contributions?: EmbedContributionDay[] | null;
  graph?: boolean;
}

const W = 600;
const PAD = 26;

export function renderMinimalEmbedSvg(
  data: UserEmbedStats,
  options: RenderMinimalEmbedOptions = {},
): string {
  const theme: EmbedTheme = options.theme === "light" ? "light" : "dark";
  const palette = resolvePalette(theme, options.color ?? null);
  const tokensFormat = options.tokensFormat ?? "compact";
  const costFormat = options.costFormat ?? "compact";
  const contributions = options.graph ? (options.contributions ?? []) : null;
  const right = W - PAD;

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
    : "Unranked";
  const rankColor = getRankColor(data.stats.rank, palette);
  const graphY = 148;
  const graph = contributions
    ? contributionPanel({
        x: PAD,
        y: graphY,
        width: W - PAD * 2,
        palette,
        contributions,
      })
    : null;
  const height = graph ? Math.ceil(graphY + graph.height + 32) : 162;

  return `<?xml version="1.0" encoding="UTF-8"?>
<svg data-template="minimal" width="${W}" height="${height}" viewBox="0 0 ${W} ${height}" fill="none" xmlns="http://www.w3.org/2000/svg" role="img" aria-label="Tokscale stats for @${escapeXml(data.user.username)}">
  ${cardTextStyle()}
  ${cardSurface(W, height, palette)}
  ${cardHeader({
    username: data.user.username,
    displayName: data.user.displayName,
    palette,
    x: PAD,
    y: 28,
    right,
  })}
  ${divider(PAD, right, 60, palette)}
  <text x="${PAD}" y="82" fill="${palette.muted}" font-size="11" font-weight="600" font-family="${FIGTREE_FONT_STACK}">Total tokens</text>
  ${fittedText({ text: tokens, x: PAD, y: 118, maxWidth: 296, fill: palette.brand, fontSize: 34, minFontSize: 14, fontWeight: 600 })}
  <line x1="350" y1="74" x2="350" y2="132" stroke="${palette.divider}"/>
  <text x="372" y="80" fill="${palette.muted}" font-size="10" font-weight="600" font-family="${FIGTREE_FONT_STACK}">Cost</text>
  ${fittedText({ text: cost, x: 372, y: 99, maxWidth: 202, fill: palette.cost, fontSize: 17, minFontSize: 8, fontWeight: 600 })}
  <text x="372" y="114" fill="${palette.muted}" font-size="10" font-weight="600" font-family="${FIGTREE_FONT_STACK}">Rank</text>
  ${fittedText({ text: rank, x: 372, y: 132, maxWidth: 202, fill: rankColor, fontSize: 16, minFontSize: 8, fontWeight: 600 })}
  ${graph ? `${divider(PAD, right, 136, palette)}\n  ${graph.svg}` : ""}
  ${cardFooter({
    updatedAt: data.stats.updatedAt,
    palette,
    x: PAD,
    right,
    y: height - 16,
  })}
</svg>`;
}
