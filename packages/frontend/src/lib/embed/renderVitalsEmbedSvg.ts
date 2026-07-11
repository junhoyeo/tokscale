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
  divider,
  escapeXml,
  formatRank,
  gradeColors,
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
  const colors = gradeColors(palette);
  const contributions = options.contributions ?? [];
  const layout = layoutContributions(contributions);
  const avgIntensity = contributions.length
    ? contributions.reduce((sum, day) => sum + day.intensity, 0) /
      contributions.length
    : 0;
  const rank = data.stats.rank;
  const rankTotal = data.stats.rankTotal ?? null;
  const rankFraction =
    rank && rankTotal ? (rankTotal - rank + 1) / rankTotal : 0;
  const signals = [
    {
      label: "Leaderboard",
      value: rank
        ? formatRank(rank, rankTotal, options.rankFormat)
        : "Unranked",
      fraction: rankFraction,
      color: colors[4],
    },
    {
      label: "Active days",
      value: String(layout.activeDays),
      fraction: Math.min(1, layout.activeDays / 365),
      color: colors[3],
    },
    {
      label: "Average intensity",
      value: `${avgIntensity.toFixed(1)} / 4`,
      fraction: Math.min(1, avgIntensity / 4),
      color: colors[2],
    },
  ];
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
  <defs><style>@import url('${FIGTREE_FONT_IMPORT}');</style></defs>
  ${cardSurface(W, H, palette)}
  ${cardHeader({
    username: data.user.username,
    displayName: data.user.displayName,
    palette,
    x: PAD,
    y: 27,
    right,
    eyebrow: "Tokscale",
  })}
  <text x="${PAD}" y="88" fill="${palette.brand}" font-size="25" font-weight="700" font-family="${FIGTREE_FONT_STACK}">${escapeXml(tokens)}</text>
  <text x="${PAD}" y="104" fill="${palette.muted}" font-size="9" font-weight="600" letter-spacing="0.08em" font-family="${FIGTREE_FONT_STACK}">TOKENS</text>
  <text x="${right}" y="88" fill="${palette.cost}" font-size="18" font-weight="700" text-anchor="end" font-family="${FIGTREE_FONT_STACK}">${escapeXml(cost)}</text>
  <text x="${right}" y="104" fill="${palette.muted}" font-size="9" font-weight="600" letter-spacing="0.08em" text-anchor="end" font-family="${FIGTREE_FONT_STACK}">COST</text>
  ${divider(PAD, right, 118, palette)}
  ${signals
    .map((signal, index) => {
      const y = 140 + index * 28;
      return `<text x="${PAD}" y="${y}" fill="${palette.muted}" font-size="10" font-weight="600" font-family="${FIGTREE_FONT_STACK}">${signal.label}</text>
  <rect x="154" y="${y - 7}" width="244" height="5" rx="2.5" fill="${palette.graphGrade0}"/>
  <rect x="154" y="${y - 7}" width="${(244 * Math.max(0, Math.min(1, signal.fraction))).toFixed(1)}" height="5" rx="2.5" fill="${signal.color}"/>
  <text x="${right}" y="${y}" fill="${palette.text}" font-size="11" font-weight="700" text-anchor="end" font-family="${FIGTREE_FONT_STACK}">${escapeXml(signal.value)}</text>`;
    })
    .join("\n  ")}
  ${cardFooter({
    username: data.user.username,
    updatedAt: data.stats.updatedAt,
    palette,
    x: PAD,
    right,
    y: H - 16,
  })}
</svg>`;
}
