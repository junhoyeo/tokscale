/**
 * "orbit" embed template — a 270-degree arc gauge dramatizing leaderboard
 * standing (rank percentile), with token / cost / active-day stats stacked
 * alongside.
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
  getRankColor,
  formatDateLabel,
  brandIcon,
  arcPath,
  polarPoint,
  FIGTREE_FONT_STACK,
  FIGTREE_FONT_IMPORT,
  escapeXml,
  formatRank,
  type EmbedRankFormat,
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
  const tokensFormat: EmbedNumberFormat = options.tokensFormat ?? "compact";
  const costFormat: EmbedNumberFormat = options.costFormat ?? "compact";
  const sortBy = options.sortBy === "cost" ? "cost" : "tokens";

  const rank = data.stats.rank;
  const total = data.stats.rankTotal ?? null;
  const showGraph = options.graph ?? false;
  const layout = options.contributions && options.contributions.length > 0
    ? layoutContributions(options.contributions)
    : null;
  const activeDays = layout ? layout.activeDays : null;
  const H = showGraph && layout ? 369 : 248;

  const fraction = rank && total && total > 0
    ? Math.max(0.02, (total - rank + 1) / total)
    : rank ? 0.5 : 0;
  const rankText = rank ? formatRank(rank, total, options.rankFormat) : "—";
  const rankSize = rankText.length <= 5 ? 34 : rankText.length <= 9 ? 26 : 20;
  const rankColor = getRankColor(rank, palette);

  const cx = 152;
  const cy = 142;
  const R = 78;
  const START = -135;
  const SPAN = 270;
  const endDeg = START + SPAN * fraction;
  const [tipX, tipY] = polarPoint(cx, cy, R, endDeg);

  const updated = escapeXml(formatDateLabel(data.stats.updatedAt));

  const stats: { label: string; value: string; fill: string }[] = [
    { label: "TOKENS", value: formatNumber(data.stats.totalTokens, tokensFormat === "compact"), fill: palette.tokenEnd },
    { label: "COST", value: formatCurrency(data.stats.totalCost, costFormat === "compact"), fill: palette.cost },
  ];
  if (activeDays !== null) {
    stats.push({ label: "ACTIVE DAYS", value: String(activeDays), fill: palette.text });
  }

  const colX = 312;
  const parts: string[] = [];
  const add = (s: string) => parts.push(s);

  add(`<?xml version="1.0" encoding="UTF-8"?>`);
  add(`<svg width="${W}" height="${H}" viewBox="0 0 ${W} ${H}" fill="none" xmlns="http://www.w3.org/2000/svg" role="img" aria-label="Tokscale rank gauge for @${escapeXml(data.user.username)}">`);
  add(`  <defs>`);
  add(`    <style>@import url('${FIGTREE_FONT_IMPORT}');</style>`);
  add(`    <linearGradient id="bg" x1="0" y1="0" x2="${W}" y2="${H}" gradientUnits="userSpaceOnUse"><stop offset="0" stop-color="${palette.bgStart}"/><stop offset="1" stop-color="${palette.bgEnd}"/></linearGradient>`);
  add(`    <radialGradient id="glow" cx="0.25" cy="0.4" r="0.7"><stop offset="0" stop-color="${palette.glowColor}" stop-opacity="${palette.glowOpacity + 0.05}"/><stop offset="1" stop-color="${palette.glowColor}" stop-opacity="0"/></radialGradient>`);
  add(`    <linearGradient id="arc" x1="0" y1="1" x2="1" y2="0"><stop offset="0" stop-color="${palette.tokenStart}"/><stop offset="1" stop-color="${palette.brand}"/></linearGradient>`);
  add(`    <clipPath id="card-clip"><rect width="${W}" height="${H}" rx="16"/></clipPath>`);
  add(`  </defs>`);
  add(`  <rect width="${W}" height="${H}" rx="16" fill="url(#bg)"/>`);
  add(`  <rect width="${W}" height="${H}" rx="16" fill="url(#glow)" clip-path="url(#card-clip)"/>`);
  add(`  <rect x="0.5" y="0.5" width="${W - 1}" height="${H - 1}" rx="15.5" fill="none" stroke="${palette.border}"/>`);

  // Header.
  add(`  ${brandIcon(PAD, 38, palette.brand)}`);
  add(`  <text x="${PAD + 20}" y="38" fill="${palette.text}" font-size="15" font-weight="700" font-family="${FIGTREE_FONT_STACK}">@${escapeXml(data.user.username)}</text>`);

  // Rank gauge.
  add(`  <path d="${arcPath(cx, cy, R, START, START + SPAN)}" fill="none" stroke="${palette.divider}" stroke-width="13" stroke-linecap="round" opacity="0.5"/>`);
  if (fraction > 0) {
    add(`  <path d="${arcPath(cx, cy, R, START, endDeg)}" fill="none" stroke="url(#arc)" stroke-width="13" stroke-linecap="round"/>`);
    add(`  <circle cx="${tipX.toFixed(2)}" cy="${tipY.toFixed(2)}" r="8" fill="${palette.brand}" opacity="0.25"/>`);
    add(`  <circle cx="${tipX.toFixed(2)}" cy="${tipY.toFixed(2)}" r="4.5" fill="${palette.tokenEnd}"/>`);
  }
  add(`  <text x="${cx}" y="${cy - 18}" fill="${palette.muted}" font-size="10" font-weight="700" letter-spacing="0.1em" text-anchor="middle" font-family="${FIGTREE_FONT_STACK}">RANK · ${sortBy === "cost" ? "COST" : "TOKENS"}</text>`);
  add(`  <text x="${cx}" y="${cy + 13}" fill="${rankColor}" font-size="${rankSize}" font-weight="800" text-anchor="middle" font-family="${FIGTREE_FONT_STACK}">${escapeXml(rankText)}</text>`);

  // Stat column.
  stats.forEach((s, i) => {
    const y = 78 + i * 52;
    add(`  <rect x="${colX}" y="${y - 13}" width="3" height="34" rx="1.5" fill="${palette.brand}" opacity="0.55"/>`);
    add(`  <text x="${colX + 14}" y="${y}" fill="${palette.muted}" font-size="10.5" font-weight="600" letter-spacing="0.05em" font-family="${FIGTREE_FONT_STACK}">${s.label}</text>`);
    add(`  <text x="${colX + 14}" y="${y + 24}" fill="${s.fill}" font-size="22" font-weight="800" font-family="${FIGTREE_FONT_STACK}">${escapeXml(s.value)}</text>`);
  });

  // Optional contribution graph.
  if (showGraph && layout) {
    const colors = gradeColors(palette);
    const CELL = 7, GAP = 2, STRIDE = 9;
    const gridW = layout.numWeeks * STRIDE - GAP;
    const gx = PAD + Math.max(0, (W - PAD * 2 - gridW) / 2);
    add(`  <text x="${PAD}" y="244" fill="${palette.muted}" font-size="11" font-weight="600" font-family="${FIGTREE_FONT_STACK}">Daily token activity</text>`);
    for (const m of layout.months) {
      add(`  <text x="${(gx + m.week * STRIDE).toFixed(1)}" y="260" fill="${palette.muted}" font-size="9" font-family="${FIGTREE_FONT_STACK}">${m.label}</text>`);
    }
    for (const c of layout.cells) {
      add(`  <rect x="${(gx + c.week * STRIDE).toFixed(1)}" y="${266 + c.day * STRIDE}" width="${CELL}" height="${CELL}" rx="1.5" fill="${colors[c.intensity]}"/>`);
    }
  }

  // Footer.
  add(`  <text x="${PAD}" y="${H - 16}" fill="${palette.muted}" font-size="11" font-family="${FIGTREE_FONT_STACK}">${updated}</text>`);
  add(`  <text x="${W - PAD}" y="${H - 16}" fill="${palette.muted}" font-size="11" text-anchor="end" font-family="${FIGTREE_FONT_STACK}">tokscale.ai/u/${escapeXml(data.user.username)}</text>`);
  add(`</svg>`);

  return parts.join("\n");
}
