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
  escapeXml,
  fittedText,
  formatRank,
  getRankColor,
  resolvePalette,
} from "./embedShared";

export interface RenderGraphEmbedOptions {
  theme?: EmbedTheme;
  color?: EmbedColorName | null;
  sortBy?: "tokens" | "cost";
  tokensFormat?: EmbedNumberFormat;
  costFormat?: EmbedNumberFormat;
  rankFormat?: EmbedRankFormat;
  contributions?: EmbedContributionDay[] | null;
}

const W = 680;
const PAD = 24;

export function renderGraphEmbedSvg(
  data: UserEmbedStats,
  options: RenderGraphEmbedOptions = {},
): string {
  const theme: EmbedTheme = options.theme === "light" ? "light" : "dark";
  const palette = resolvePalette(theme, options.color ?? null);
  const sortBy = options.sortBy === "cost" ? "cost" : "tokens";
  const right = W - PAD;
  const tokens = formatNumber(
    data.stats.totalTokens,
    (options.tokensFormat ?? "compact") === "compact",
  );
  const cost = formatCurrency(
    data.stats.totalCost,
    (options.costFormat ?? "compact") === "compact",
  );
  const rank = data.stats.rank
    ? formatRank(
        data.stats.rank,
        data.stats.rankTotal ?? null,
        options.rankFormat,
      )
    : "Unranked";
  const graph = contributionPanel({
    x: PAD,
    y: 58,
    width: W - PAD * 2,
    palette,
    contributions: options.contributions ?? [],
    showDayLabels: true,
    showLegend: true,
  });
  const H = Math.ceil(58 + graph.height + 32);
  const stats = [
    { value: tokens, label: "Tokens", color: palette.brand },
    { value: cost, label: "Cost", color: palette.cost },
    {
      value: rank,
      label: `Rank · ${sortBy}`,
      color: getRankColor(data.stats.rank, palette),
    },
  ];

  return `<?xml version="1.0" encoding="UTF-8"?>
<svg data-template="graph" width="${W}" height="${H}" viewBox="0 0 ${W} ${H}" fill="none" xmlns="http://www.w3.org/2000/svg" role="img" aria-label="Tokscale contribution graph for @${escapeXml(data.user.username)}">
  ${cardTextStyle()}
  ${cardSurface(W, H, palette)}
  ${cardHeader({
    username: data.user.username,
    displayName: null,
    palette,
    x: PAD,
    y: 34,
    right: 336,
  })}
  ${stats
    .map((stat, index) => {
      const x = 392 + index * 88;
      const maxWidth = index === 2 ? 80 : 76;
      return `<text x="${x}" y="22" fill="${palette.muted}" font-size="10" font-weight="600" font-family="${FIGTREE_FONT_STACK}">${escapeXml(stat.label)}</text>
  ${fittedText({ text: stat.value, x, y: 42, maxWidth, fill: stat.color, fontSize: 14, minFontSize: 7, fontWeight: 600 })}`;
    })
    .join("\n  ")}
  ${graph.svg}
  ${cardFooter({
    updatedAt: data.stats.updatedAt,
    palette,
    x: PAD,
    right,
    y: H - 16,
  })}
</svg>`;
}
