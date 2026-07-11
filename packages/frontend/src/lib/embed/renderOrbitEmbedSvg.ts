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
  layoutContributions,
  resolvePalette,
} from "./embedShared";

export interface RenderOrbitEmbedOptions {
  theme?: EmbedTheme;
  color?: EmbedColorName | null;
  sortBy?: "tokens" | "cost";
  tokensFormat?: EmbedNumberFormat;
  costFormat?: EmbedNumberFormat;
  rankFormat?: EmbedRankFormat;
  contributions?: EmbedContributionDay[] | null;
  graph?: boolean;
}

const W = 560;
const PAD = 24;

export function renderOrbitEmbedSvg(
  data: UserEmbedStats,
  options: RenderOrbitEmbedOptions = {},
): string {
  const theme: EmbedTheme = options.theme === "light" ? "light" : "dark";
  const palette = resolvePalette(theme, options.color ?? null);
  const sortBy = options.sortBy === "cost" ? "cost" : "tokens";
  const rank = data.stats.rank;
  const rankTotal = data.stats.rankTotal ?? null;
  const rankText = rank
    ? formatRank(rank, rankTotal, options.rankFormat)
    : "Unranked";
  const standing =
    rank && rankTotal
      ? Math.max(0, Math.min(1, (rankTotal - rank + 1) / rankTotal))
      : 0;
  const layout = options.contributions?.length
    ? layoutContributions(options.contributions)
    : null;
  const contributions =
    options.graph && options.contributions?.length
      ? options.contributions
      : null;
  const right = W - PAD;
  const graphY = 236;
  const graph = contributions
    ? contributionPanel({
        x: PAD,
        y: graphY,
        width: W - PAD * 2,
        palette,
        contributions,
      })
    : null;
  const height = graph ? Math.ceil(graphY + graph.height + 32) : 248;
  const statRows = [
    {
      label: "Tokens",
      value: formatNumber(
        data.stats.totalTokens,
        (options.tokensFormat ?? "compact") === "compact",
      ),
      color: palette.brand,
    },
    {
      label: "Cost",
      value: formatCurrency(
        data.stats.totalCost,
        (options.costFormat ?? "compact") === "compact",
      ),
      color: palette.cost,
    },
    ...(layout
      ? [
          {
            label: "Active days",
            value: String(layout.activeDays),
            color: palette.text,
          },
        ]
      : []),
  ];

  return `<?xml version="1.0" encoding="UTF-8"?>
<svg data-template="orbit" width="${W}" height="${height}" viewBox="0 0 ${W} ${height}" fill="none" xmlns="http://www.w3.org/2000/svg" role="img" aria-label="Tokscale leaderboard standing for @${escapeXml(data.user.username)}">
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
  ${divider(PAD, right, 66, palette)}
  <text x="${PAD}" y="92" fill="${palette.muted}" font-size="10" font-weight="600" font-family="${FIGTREE_FONT_STACK}">Rank · ${sortBy === "cost" ? "cost" : "tokens"}</text>
  <text x="${PAD}" y="136" fill="${getRankColor(rank, palette)}" font-size="34" font-weight="700" font-family="${FIGTREE_FONT_STACK}">${escapeXml(rankText)}</text>
  <rect x="${PAD}" y="154" width="218" height="5" rx="2.5" fill="${palette.graphGrade0}"/>
  <rect x="${PAD}" y="154" width="${(218 * standing).toFixed(1)}" height="5" rx="2.5" fill="${palette.brand}"/>
  <text x="${PAD}" y="178" fill="${palette.muted}" font-size="10" font-family="${FIGTREE_FONT_STACK}">${rank && rankTotal ? `Ahead of ${Math.round(standing * 100)}% of ranked profiles` : "Ranking pending"}</text>
  ${statRows
    .map((stat, index) => {
      const y = 91 + index * 43;
      return `<text x="318" y="${y}" fill="${palette.muted}" font-size="10" font-weight="600" font-family="${FIGTREE_FONT_STACK}">${stat.label}</text>
  <text x="${right}" y="${y + 20}" fill="${stat.color}" font-size="18" font-weight="700" text-anchor="end" font-family="${FIGTREE_FONT_STACK}">${escapeXml(stat.value)}</text>`;
    })
    .join("\n  ")}
  ${graph ? `${divider(PAD, right, 224, palette)}\n  ${graph.svg}` : ""}
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
