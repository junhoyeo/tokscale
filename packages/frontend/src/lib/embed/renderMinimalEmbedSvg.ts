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
  const contributions =
    options.graph && options.contributions?.length
      ? options.contributions
      : null;
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
  const tokenSize = tokens.length > 13 ? 27 : tokens.length > 10 ? 31 : 34;
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
  <defs><style>@import url('${FIGTREE_FONT_IMPORT}');</style></defs>
  ${cardSurface(W, height, palette)}
  ${cardHeader({
    username: data.user.username,
    displayName: data.user.displayName,
    palette,
    x: PAD,
    y: 28,
    right,
    eyebrow: "Tokscale",
  })}
  <text x="${PAD}" y="82" fill="${palette.muted}" font-size="10" font-weight="600" letter-spacing="0.08em" font-family="${FIGTREE_FONT_STACK}">TOTAL TOKENS</text>
  <text x="${PAD}" y="118" fill="${palette.brand}" font-size="${tokenSize}" font-weight="700" font-family="${FIGTREE_FONT_STACK}">${escapeXml(tokens)}</text>
  <line x1="338" y1="76" x2="338" y2="122" stroke="${palette.divider}"/>
  <text x="360" y="87" fill="${palette.muted}" font-size="10" font-weight="600" font-family="${FIGTREE_FONT_STACK}">Cost</text>
  <text x="360" y="113" fill="${palette.cost}" font-size="19" font-weight="700" font-family="${FIGTREE_FONT_STACK}">${escapeXml(cost)}</text>
  <text x="485" y="87" fill="${palette.muted}" font-size="10" font-weight="600" font-family="${FIGTREE_FONT_STACK}">Rank</text>
  <text x="485" y="113" fill="${rankColor}" font-size="19" font-weight="700" font-family="${FIGTREE_FONT_STACK}">${escapeXml(rank)}</text>
  ${graph ? `${divider(PAD, right, 136, palette)}\n  ${graph.svg}` : ""}
  ${cardFooter({
    username: data.user.username,
    updatedAt: data.stats.updatedAt,
    palette,
    x: PAD,
    right,
    y: height - 16,
  })}
</svg>`;
}
