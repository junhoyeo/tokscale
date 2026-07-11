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
  contributionPanel,
  divider,
  escapeXml,
  formatRank,
  getRankColor,
  layoutContributions,
  resolvePalette,
} from "./embedShared";

export interface RenderReceiptEmbedOptions {
  theme?: EmbedTheme;
  color?: EmbedColorName | null;
  sortBy?: "tokens" | "cost";
  tokensFormat?: EmbedNumberFormat;
  costFormat?: EmbedNumberFormat;
  rankFormat?: EmbedRankFormat;
  contributions?: EmbedContributionDay[] | null;
  graph?: boolean;
}

const W = 400;
const PAD = 22;

export function renderReceiptEmbedSvg(
  data: UserEmbedStats,
  options: RenderReceiptEmbedOptions = {},
): string {
  const theme: EmbedTheme = options.theme === "light" ? "light" : "dark";
  const palette = resolvePalette(theme, options.color ?? null);
  const contributions =
    options.graph && options.contributions?.length
      ? options.contributions
      : null;
  const layout = options.contributions?.length
    ? layoutContributions(options.contributions)
    : null;
  const right = W - PAD;
  const rank = data.stats.rank
    ? formatRank(
        data.stats.rank,
        data.stats.rankTotal ?? null,
        options.rankFormat,
      )
    : "Unranked";
  const rows = [
    [
      "Tokens",
      formatNumber(
        data.stats.totalTokens,
        (options.tokensFormat ?? "full") === "compact",
      ),
      palette.brand,
    ],
    [
      "Cost",
      formatCurrency(
        data.stats.totalCost,
        (options.costFormat ?? "full") === "compact",
      ),
      palette.cost,
    ],
    [
      `Rank · ${options.sortBy === "cost" ? "cost" : "tokens"}`,
      rank,
      getRankColor(data.stats.rank, palette),
    ],
    ...(layout
      ? [["Active days", String(layout.activeDays), palette.text]]
      : []),
  ] as const;
  const graphY = 224;
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
  const height = graph ? Math.ceil(graphY + graph.height + 32) : 250;

  return `<?xml version="1.0" encoding="UTF-8"?>
<svg data-template="receipt" width="${W}" height="${height}" viewBox="0 0 ${W} ${height}" fill="none" xmlns="http://www.w3.org/2000/svg" role="img" aria-label="Tokscale compact ledger for @${escapeXml(data.user.username)}">
  ${cardSurface(W, height, palette)}
  ${cardHeader({
    username: data.user.username,
    displayName: data.user.displayName,
    palette,
    x: PAD,
    y: 26,
    right,
    eyebrow: "Tokscale",
    mono: true,
  })}
  ${divider(PAD, right, 64, palette)}
  ${rows
    .map(([label, value, color], index) => {
      const y = 90 + index * 30;
      return `<text x="${PAD}" y="${y}" fill="${palette.muted}" font-size="11" font-family="${MONO_FONT_STACK}">${escapeXml(label)}</text>
  <text x="${right}" y="${y}" fill="${color}" font-size="12" font-weight="700" text-anchor="end" font-family="${MONO_FONT_STACK}">${escapeXml(value)}</text>
  <line x1="${PAD}" y1="${y + 11}" x2="${right}" y2="${y + 11}" stroke="${palette.divider}"/>`;
    })
    .join("\n  ")}
  ${graph ? `${divider(PAD, right, 212, palette)}\n  ${graph.svg}` : ""}
  ${cardFooter({
    username: data.user.username,
    updatedAt: data.stats.updatedAt,
    palette,
    x: PAD,
    right,
    y: height - 16,
    mono: true,
  })}
</svg>`;
}
