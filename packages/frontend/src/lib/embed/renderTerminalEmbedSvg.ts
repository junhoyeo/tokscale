import type { UserEmbedStats, EmbedContributionDay } from "./getUserEmbedStats";
import { formatCurrency, formatNumber } from "../format";
import {
  type EmbedColorName,
  type EmbedNumberFormat,
  type EmbedRankFormat,
  type EmbedTheme,
  MONO_FONT_STACK,
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

export interface RenderTerminalEmbedOptions {
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
const PAD = 22;

export function renderTerminalEmbedSvg(
  data: UserEmbedStats,
  options: RenderTerminalEmbedOptions = {},
): string {
  const theme: EmbedTheme = options.theme === "light" ? "light" : "dark";
  const palette = resolvePalette(theme, options.color ?? null);
  const sortBy = options.sortBy === "cost" ? "cost" : "tokens";
  const contributions = options.graph ? (options.contributions ?? []) : null;
  const right = W - PAD;
  const rank = data.stats.rank
    ? formatRank(
        data.stats.rank,
        data.stats.rankTotal ?? null,
        options.rankFormat,
      )
    : "unranked";
  const rows = [
    {
      label: "tokens",
      value: formatNumber(
        data.stats.totalTokens,
        (options.tokensFormat ?? "full") === "compact",
      ),
      color: palette.brand,
    },
    {
      label: "cost",
      value: formatCurrency(
        data.stats.totalCost,
        (options.costFormat ?? "compact") === "compact",
      ),
      color: palette.cost,
    },
    {
      label: `rank.${sortBy}`,
      value: rank,
      color: getRankColor(data.stats.rank, palette),
    },
  ];
  const graphY = 158;
  const graph = contributions
    ? contributionPanel({
        x: PAD,
        y: graphY,
        width: W - PAD * 2,
        palette,
        contributions,
        mono: true,
      })
    : null;
  const height = graph ? Math.ceil(graphY + graph.height + 32) : 176;

  return `<?xml version="1.0" encoding="UTF-8"?>
<svg data-template="terminal" width="${W}" height="${height}" viewBox="0 0 ${W} ${height}" fill="none" xmlns="http://www.w3.org/2000/svg" role="img" aria-label="Tokscale stats for @${escapeXml(data.user.username)}">
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
  <g data-readout="usage">
  ${rows
    .map((row, index) => {
      const y = 82 + index * 24;
      return `<text x="${PAD}" y="${y}" fill="${palette.muted}" font-size="12" font-family="${MONO_FONT_STACK}">${escapeXml(row.label)}</text>
  ${fittedText({ text: row.value, x: right, y, maxWidth: 320, fill: row.color, fontSize: 13, minFontSize: 8, fontFamily: MONO_FONT_STACK, fontWeight: 600, textAnchor: "end" })}`;
    })
    .join("\n  ")}
  </g>
  ${graph ? `${divider(PAD, right, 146, palette)}\n  ${graph.svg}` : ""}
  ${cardFooter({
    updatedAt: data.stats.updatedAt,
    palette,
    x: PAD,
    right,
    y: height - 16,
  })}
</svg>`;
}
