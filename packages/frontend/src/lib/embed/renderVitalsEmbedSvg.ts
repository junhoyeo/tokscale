/**
 * "vitals" embed template — three concentric activity rings (rank percentile,
 * active-days ratio, average intensity) with the token total at the center
 * and a labeled legend alongside.
 */
import type { UserEmbedStats, EmbedContributionDay } from "./getUserEmbedStats";
import { formatNumber, formatCurrency } from "../format";
import {
  type EmbedTheme,
  type EmbedColorName,
  type EmbedNumberFormat,
  resolvePalette,
  layoutContributions,
  gradeColors,
  formatDateLabel,
  brandIcon,
  arcPath,
  FIGTREE_FONT_STACK,
  FIGTREE_FONT_IMPORT,
  escapeXml,
  formatRank,
  type EmbedRankFormat,
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
const SW = 12;

export function renderVitalsEmbedSvg(
  data: UserEmbedStats,
  options: RenderVitalsEmbedOptions = {},
): string {
  const theme: EmbedTheme = options.theme === "light" ? "light" : "dark";
  const palette = resolvePalette(theme, options.color ?? null);
  const grades = gradeColors(palette);
  const tokensFormat: EmbedNumberFormat = options.tokensFormat ?? "compact";
  const costFormat: EmbedNumberFormat = options.costFormat ?? "compact";

  const contributions = options.contributions ?? [];
  const layout = layoutContributions(contributions);
  const rank = data.stats.rank;
  const total = data.stats.rankTotal ?? null;
  const avgIntensity = contributions.length
    ? contributions.reduce((s, c) => s + c.intensity, 0) / contributions.length
    : 0;

  const rankFrac = rank && total && total > 0 ? Math.max(0.02, (total - rank + 1) / total) : 0;
  const activeFrac = Math.min(1, layout.activeDays / 365);
  const intensityFrac = Math.min(1, avgIntensity / 4);

  const cx = 140;
  const cy = 132;
  const rings = [
    { r: 80, frac: rankFrac, color: grades[4] },
    { r: 62, frac: activeFrac, color: grades[3] },
    { r: 46, frac: intensityFrac, color: grades[2] },
  ];

  const legend = [
    { name: "Rank", value: rank ? formatRank(rank, total, options.rankFormat) : "—", sub: rank ? "" : "unranked", color: grades[4] },
    { name: "Active days", value: String(layout.activeDays), sub: "of ~365", color: grades[3] },
    { name: "Avg intensity", value: avgIntensity.toFixed(1), sub: "of 4.0", color: grades[2] },
  ];

  const tokens = formatNumber(data.stats.totalTokens, tokensFormat === "compact");
  const cost = formatCurrency(data.stats.totalCost, costFormat === "compact");
  const updated = escapeXml(formatDateLabel(data.stats.updatedAt));
  const legendX = 268;

  const parts: string[] = [];
  const add = (s: string) => parts.push(s);

  add(`<?xml version="1.0" encoding="UTF-8"?>`);
  add(`<svg width="${W}" height="${H}" viewBox="0 0 ${W} ${H}" fill="none" xmlns="http://www.w3.org/2000/svg" role="img" aria-label="Tokscale activity rings for @${escapeXml(data.user.username)}">`);
  add(`  <defs>`);
  add(`    <style>@import url('${FIGTREE_FONT_IMPORT}');</style>`);
  add(`    <linearGradient id="bg" x1="0" y1="0" x2="${W}" y2="${H}" gradientUnits="userSpaceOnUse"><stop offset="0" stop-color="${palette.bgStart}"/><stop offset="1" stop-color="${palette.bgEnd}"/></linearGradient>`);
  add(`    <radialGradient id="glow" cx="0.27" cy="0.45" r="0.7"><stop offset="0" stop-color="${palette.glowColor}" stop-opacity="${palette.glowOpacity + 0.05}"/><stop offset="1" stop-color="${palette.glowColor}" stop-opacity="0"/></radialGradient>`);
  add(`    <clipPath id="card-clip"><rect width="${W}" height="${H}" rx="16"/></clipPath>`);
  add(`  </defs>`);
  add(`  <rect width="${W}" height="${H}" rx="16" fill="url(#bg)"/>`);
  add(`  <rect width="${W}" height="${H}" rx="16" fill="url(#glow)" clip-path="url(#card-clip)"/>`);
  add(`  <rect x="0.5" y="0.5" width="${W - 1}" height="${H - 1}" rx="15.5" fill="none" stroke="${palette.border}"/>`);

  // Header.
  add(`  ${brandIcon(PAD, 38, palette.brand)}`);
  add(`  <text x="${PAD + 20}" y="38" fill="${palette.text}" font-size="15" font-weight="700" font-family="${FIGTREE_FONT_STACK}">@${escapeXml(data.user.username)}</text>`);

  // Rings: track circle + filled arc.
  for (const ring of rings) {
    add(`  <circle cx="${cx}" cy="${cy}" r="${ring.r}" fill="none" stroke="${palette.divider}" stroke-width="${SW}" opacity="0.4"/>`);
    if (ring.frac > 0) {
      const end = Math.min(359.9, 360 * ring.frac);
      add(`  <path d="${arcPath(cx, cy, ring.r, 0, end)}" fill="none" stroke="${ring.color}" stroke-width="${SW}" stroke-linecap="round"/>`);
    }
  }

  // Center: token total + cost.
  add(`  <text x="${cx}" y="${cy - 2}" fill="${palette.tokenEnd}" font-size="25" font-weight="800" text-anchor="middle" font-family="${FIGTREE_FONT_STACK}">${escapeXml(tokens)}</text>`);
  add(`  <text x="${cx}" y="${cy + 14}" fill="${palette.muted}" font-size="9" font-weight="600" letter-spacing="0.12em" text-anchor="middle" font-family="${FIGTREE_FONT_STACK}">TOKENS</text>`);
  add(`  <text x="${cx}" y="${cy + 33}" fill="${palette.cost}" font-size="12" font-weight="700" text-anchor="middle" font-family="${FIGTREE_FONT_STACK}">${escapeXml(cost)} spent</text>`);

  // Legend.
  legend.forEach((row, i) => {
    const y = 86 + i * 50;
    add(`  <circle cx="${legendX + 5}" cy="${y - 4}" r="5" fill="${row.color}"/>`);
    add(`  <text x="${legendX + 20}" y="${y}" fill="${palette.muted}" font-size="11" font-weight="600" font-family="${FIGTREE_FONT_STACK}">${escapeXml(row.name)}</text>`);
    add(`  <text x="${legendX + 20}" y="${y + 23}" font-family="${FIGTREE_FONT_STACK}"><tspan fill="${palette.text}" font-size="19" font-weight="800">${escapeXml(row.value)}</tspan><tspan fill="${palette.muted}" font-size="12" font-weight="600"> ${escapeXml(row.sub)}</tspan></text>`);
  });

  // Footer.
  add(`  <text x="${PAD}" y="${H - 16}" fill="${palette.muted}" font-size="11" font-family="${FIGTREE_FONT_STACK}">${updated}</text>`);
  add(`  <text x="${W - PAD}" y="${H - 16}" fill="${palette.muted}" font-size="11" text-anchor="end" font-family="${FIGTREE_FONT_STACK}">tokscale.ai/u/${escapeXml(data.user.username)}</text>`);
  add(`</svg>`);

  return parts.join("\n");
}
