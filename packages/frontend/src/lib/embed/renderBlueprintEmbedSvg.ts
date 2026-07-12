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
  getContributionWindow,
  getRankColor,
  layoutContributions,
  resolvePalette,
} from "./embedShared";

export interface RenderBlueprintEmbedOptions {
  theme?: EmbedTheme;
  color?: EmbedColorName | null;
  sortBy?: "tokens" | "cost";
  tokensFormat?: EmbedNumberFormat;
  costFormat?: EmbedNumberFormat;
  rankFormat?: EmbedRankFormat;
  contributions?: EmbedContributionDay[] | null;
  graph?: boolean;
}

const W = 640;
const PAD = 22;

export function renderBlueprintEmbedSvg(
  data: UserEmbedStats,
  options: RenderBlueprintEmbedOptions = {},
): string {
  const theme: EmbedTheme = options.theme === "light" ? "light" : "dark";
  const palette = resolvePalette(theme, options.color ?? null);
  const contributionDays = options.contributions ?? [];
  const scopedContributionDays = getContributionWindow(contributionDays).days;
  const contributions = options.graph ? contributionDays : null;
  const right = W - PAD;
  const innerWidth = W - PAD * 2;
  const tokensCompact = (options.tokensFormat ?? "full") === "compact";
  const costCompact = (options.costFormat ?? "full") === "compact";
  const rank = data.stats.rank
    ? formatRank(
        data.stats.rank,
        data.stats.rankTotal ?? null,
        options.rankFormat,
      )
    : "Unranked";
  const activity = layoutContributions(contributionDays);
  const yearTokens = scopedContributionDays.reduce(
    (total, day) => total + Math.max(0, day.totalTokens || 0),
    0,
  );
  const yearCost = scopedContributionDays.reduce(
    (total, day) => total + Math.max(0, day.totalCost || 0),
    0,
  );
  const peakDay = [...scopedContributionDays].sort(
    (left, right) =>
      right.totalTokens - left.totalTokens ||
      right.totalCost - left.totalCost ||
      right.date.localeCompare(left.date),
  )[0];
  const peakDate = peakDay
    ? new Intl.DateTimeFormat("en-US", {
        day: "numeric",
        month: "short",
        timeZone: "UTC",
      }).format(new Date(`${peakDay.date}T00:00:00.000Z`))
    : "No activity";
  const details = [
    {
      id: "total-tokens",
      value: formatNumber(data.stats.totalTokens, tokensCompact),
      label: "Tokens · lifetime",
      color: palette.brand,
    },
    {
      id: "total-cost",
      value: formatCurrency(data.stats.totalCost, costCompact),
      label: "Cost · lifetime",
      color: palette.cost,
    },
    {
      id: "rank",
      value: rank,
      label: `Rank · ${options.sortBy === "cost" ? "cost" : "tokens"}`,
      color: getRankColor(data.stats.rank, palette),
    },
    {
      id: "submissions",
      value: data.stats.submissionCount.toLocaleString("en-US"),
      label: "Submissions",
      color: palette.text,
    },
    {
      id: "active-days",
      value: activity.activeDays.toLocaleString("en-US"),
      label: "Active days · 1y",
      color: palette.text,
    },
    {
      id: "peak-day",
      value: peakDate,
      label: "Peak day · 1y",
      color: palette.text,
    },
    {
      id: "year-tokens",
      value: formatNumber(yearTokens, tokensCompact),
      label: "Tokens · 1y",
      color: palette.text,
    },
    {
      id: "year-cost",
      value: formatCurrency(yearCost, costCompact),
      label: "Cost · 1y",
      color: palette.cost,
    },
  ];
  const detailBottom = 190;
  const graphY = 208;
  const graph = contributions
    ? contributionPanel({
        x: PAD,
        y: graphY,
        width: innerWidth,
        palette,
        contributions,
      })
    : null;
  const height = graph ? 367 : 232;

  return `<?xml version="1.0" encoding="UTF-8"?>
<svg data-template="blueprint" width="${W}" height="${height}" viewBox="0 0 ${W} ${height}" fill="none" xmlns="http://www.w3.org/2000/svg" role="img" aria-label="Tokscale usage data sheet for @${escapeXml(data.user.username)}">
  ${cardTextStyle()}
  ${cardSurface(W, height, palette)}
  ${cardHeader({
    username: data.user.username,
    displayName: data.user.displayName,
    palette,
    x: PAD,
    y: 26,
    right,
  })}
  ${divider(PAD, right, 64, palette)}
  ${details
    .map((detail, index) => {
      const column = index % 2;
      const row = Math.floor(index / 2);
      const columnWidth = innerWidth / 2;
      const x = PAD + column * columnWidth;
      const y = 87 + row * 29;
      const valueX = x + columnWidth - 14;
      return `<g data-detail="${detail.id}">
    <text x="${(x + 14).toFixed(1)}" y="${y}" fill="${palette.muted}" font-size="10" font-family="${FIGTREE_FONT_STACK}">${escapeXml(detail.label)}</text>
    ${fittedText({
      text: detail.value,
      x: valueX,
      y,
      maxWidth: 130,
      fill: detail.color,
      fontSize: 12,
      minFontSize: 8,
      fontWeight: 600,
      textAnchor: "end",
    })}
    <line x1="${(x + 14).toFixed(1)}" y1="${y + 11}" x2="${valueX.toFixed(1)}" y2="${y + 11}" stroke="${palette.divider}"/>
  </g>`;
    })
    .join("\n  ")}
  <line x1="${(PAD + innerWidth / 2).toFixed(1)}" y1="76" x2="${(PAD + innerWidth / 2).toFixed(1)}" y2="${detailBottom}" stroke="${palette.divider}"/>
  ${graph ? `${divider(PAD, right, 196, palette)}\n  ${graph.svg}` : ""}
  ${cardFooter({
    updatedAt: data.stats.updatedAt,
    palette,
    x: PAD,
    right,
    y: height - 16,
  })}
</svg>`;
}
