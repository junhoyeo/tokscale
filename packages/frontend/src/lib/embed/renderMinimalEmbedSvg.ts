/**
 * "minimal" embed template — a modern card led by a large token count, with
 * cost and rank shown as header badges and an optional contribution graph.
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
  FIGTREE_FONT_STACK,
  FIGTREE_FONT_IMPORT,
  brandIcon,
  formatDateLabel,
  getRankColor,
  escapeXml,
  formatRank,
  type EmbedRankFormat,
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
const INNER = W - PAD * 2;
const CELL = 8;
const GAP = 2;
const STRIDE = CELL + GAP;

export function renderMinimalEmbedSvg(
  data: UserEmbedStats,
  options: RenderMinimalEmbedOptions = {},
): string {
  const theme: EmbedTheme = options.theme === "light" ? "light" : "dark";
  const palette = resolvePalette(theme, options.color ?? null);
  const tokensFormat: EmbedNumberFormat = options.tokensFormat ?? "compact";
  const costFormat: EmbedNumberFormat = options.costFormat ?? "compact";

  const showGraph = options.graph ?? false;
  const contributions = showGraph && options.contributions && options.contributions.length > 0
    ? options.contributions
    : null;
  const layout = contributions ? layoutContributions(contributions) : null;

  const username = `@${data.user.username}`;
  const tokens = formatNumber(data.stats.totalTokens, tokensFormat === "compact");
  const cost = formatCurrency(data.stats.totalCost, costFormat === "compact");
  const rankText = data.stats.rank
    ? formatRank(data.stats.rank, data.stats.rankTotal ?? null, options.rankFormat)
    : "Unranked";
  const rankColor = getRankColor(data.stats.rank, palette);
  const updated = escapeXml(formatDateLabel(data.stats.updatedAt));

  const dividerY = 138;
  const graphLabelY = 164;
  const monthY = 180;
  const gridTop = 185;
  const gridBottom = gridTop + 7 * STRIDE - GAP;
  const height = layout ? gridBottom + 34 : 162;
  const footerY = height - 16;

  // Header badges (rank + cost), stacked top-right; each pill grows to fit.
  const pillW = (text: string) => Math.round(20 + text.length * 6.6);
  const rankLabel = `RANK ${rankText}`;
  const rankW = pillW(rankLabel);
  const costW = pillW(cost);
  const rankX = W - PAD - rankW;
  const costX = W - PAD - costW;

  // The token hero shrinks for very long full-format numbers.
  const tokenSize = tokens.length > 13 ? 27 : tokens.length > 10 ? 31 : 34;

  const parts: string[] = [];
  const add = (s: string) => parts.push(s);

  add(`<?xml version="1.0" encoding="UTF-8"?>`);
  add(`<svg width="${W}" height="${height}" viewBox="0 0 ${W} ${height}" fill="none" xmlns="http://www.w3.org/2000/svg" role="img" aria-label="Tokscale stats for ${escapeXml(username)}">`);
  add(`  <defs>`);
  add(`    <style>@import url('${FIGTREE_FONT_IMPORT}');</style>`);
  add(`    <linearGradient id="bg" x1="0" y1="0" x2="${W}" y2="${height}" gradientUnits="userSpaceOnUse"><stop offset="0" stop-color="${palette.bgStart}"/><stop offset="1" stop-color="${palette.bgEnd}"/></linearGradient>`);
  add(`    <radialGradient id="glow" cx="0.85" cy="0.05" r="0.8"><stop offset="0" stop-color="${palette.glowColor}" stop-opacity="${palette.glowOpacity + 0.04}"/><stop offset="1" stop-color="${palette.glowColor}" stop-opacity="0"/></radialGradient>`);
  add(`    <linearGradient id="token-grad" x1="0" y1="0" x2="1" y2="0"><stop offset="0" stop-color="${palette.tokenStart}"/><stop offset="1" stop-color="${palette.tokenEnd}"/></linearGradient>`);
  add(`    <linearGradient id="divider-grad" x1="${PAD}" y1="0" x2="${W - PAD}" y2="0" gradientUnits="userSpaceOnUse"><stop offset="0" stop-color="${palette.divider}" stop-opacity="0"/><stop offset="0.5" stop-color="${palette.divider}" stop-opacity="0.7"/><stop offset="1" stop-color="${palette.divider}" stop-opacity="0"/></linearGradient>`);
  add(`    <clipPath id="card-clip"><rect width="${W}" height="${height}" rx="16"/></clipPath>`);
  add(`  </defs>`);
  add(`  <rect width="${W}" height="${height}" rx="16" fill="url(#bg)"/>`);
  add(`  <rect width="${W}" height="${height}" rx="16" fill="url(#glow)" clip-path="url(#card-clip)"/>`);
  add(`  <rect x="0.5" y="0.5" width="${W - 1}" height="${height - 1}" rx="15.5" fill="none" stroke="${palette.border}"/>`);

  // Header: brand + username (left), cost + rank badges (right).
  add(`  ${brandIcon(PAD, 38, palette.brand)}`);
  add(`  <text x="${PAD + 20}" y="38" fill="${palette.text}" font-size="15" font-weight="700" font-family="${FIGTREE_FONT_STACK}">${escapeXml(username)}</text>`);
  add(`  <rect x="${rankX}" y="22" width="${rankW}" height="23" rx="11.5" fill="${palette.badgeBg}" stroke="${palette.badgeBorder}"/>`);
  add(`  <text x="${rankX + rankW / 2}" y="37.5" fill="${rankColor}" font-size="10.5" font-weight="700" letter-spacing="0.04em" text-anchor="middle" font-family="${FIGTREE_FONT_STACK}">${escapeXml(rankLabel)}</text>`);
  add(`  <rect x="${costX}" y="48" width="${costW}" height="23" rx="11.5" fill="${palette.badgeBg}" stroke="${palette.badgeBorder}"/>`);
  add(`  <text x="${costX + costW / 2}" y="63.5" fill="${palette.cost}" font-size="10.5" font-weight="700" text-anchor="middle" font-family="${FIGTREE_FONT_STACK}">${escapeXml(cost)}</text>`);

  // Token hero.
  add(`  <text x="${PAD}" y="74" fill="${palette.muted}" font-size="10.5" font-weight="600" letter-spacing="0.1em" font-family="${FIGTREE_FONT_STACK}">TOTAL TOKENS</text>`);
  add(`  <text x="${PAD}" y="112" fill="url(#token-grad)" font-size="${tokenSize}" font-weight="800" font-family="${FIGTREE_FONT_STACK}">${escapeXml(tokens)}</text>`);

  // Optional contribution graph.
  if (layout) {
    const colors = gradeColors(palette);
    const gridW = layout.numWeeks * STRIDE - GAP;
    const gx = PAD + Math.max(0, (INNER - gridW) / 2);
    add(`  <rect x="${PAD}" y="${dividerY}" width="${INNER}" height="1" fill="url(#divider-grad)"/>`);
    add(`  <text x="${PAD}" y="${graphLabelY}" fill="${palette.muted}" font-size="11" font-weight="600" font-family="${FIGTREE_FONT_STACK}">Daily token activity · ${layout.activeDays} active days</text>`);
    for (const m of layout.months) {
      add(`  <text x="${(gx + m.week * STRIDE).toFixed(1)}" y="${monthY}" fill="${palette.muted}" font-size="9" font-family="${FIGTREE_FONT_STACK}">${m.label}</text>`);
    }
    for (const c of layout.cells) {
      add(`  <rect x="${(gx + c.week * STRIDE).toFixed(1)}" y="${gridTop + c.day * STRIDE}" width="${CELL}" height="${CELL}" rx="2" fill="${colors[c.intensity]}"/>`);
    }
  }

  // Footer.
  add(`  <text x="${PAD}" y="${footerY}" fill="${palette.muted}" font-size="11" font-family="${FIGTREE_FONT_STACK}">${updated}</text>`);
  add(`  <text x="${W - PAD}" y="${footerY}" fill="${palette.muted}" font-size="11" text-anchor="end" font-family="${FIGTREE_FONT_STACK}">tokscale.ai/u/${escapeXml(data.user.username)}</text>`);
  add(`</svg>`);

  return parts.join("\n");
}
