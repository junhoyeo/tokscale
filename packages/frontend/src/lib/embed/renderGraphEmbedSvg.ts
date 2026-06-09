/**
 * "graph" embed template — a wide card where the contribution graph is the
 * hero: large cells span the full width, with a compact stat strip in the
 * header and a Less/More legend below.
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
  FIGTREE_FONT_STACK,
  FIGTREE_FONT_IMPORT,
  escapeXml,
  formatRank,
  type EmbedRankFormat,
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

const W = 900;
const PAD = 28;
const INNER = W - PAD * 2;
const DAY_LABEL_W = 30;
const GAP = 3;
const DAY_LABELS: [number, string][] = [[1, "Mon"], [3, "Wed"], [5, "Fri"]];

export function renderGraphEmbedSvg(
  data: UserEmbedStats,
  options: RenderGraphEmbedOptions = {},
): string {
  const theme: EmbedTheme = options.theme === "light" ? "light" : "dark";
  const palette = resolvePalette(theme, options.color ?? null);
  const tokensFormat: EmbedNumberFormat = options.tokensFormat ?? "compact";
  const costFormat: EmbedNumberFormat = options.costFormat ?? "compact";
  const sortBy = options.sortBy === "cost" ? "cost" : "tokens";

  const layout = layoutContributions(options.contributions ?? []);
  const colors = gradeColors(palette);

  const tokens = formatNumber(data.stats.totalTokens, tokensFormat === "compact");
  const cost = formatCurrency(data.stats.totalCost, costFormat === "compact");
  const rankText = data.stats.rank
    ? formatRank(data.stats.rank, data.stats.rankTotal ?? null, options.rankFormat)
    : "Unranked";
  const rankColor = getRankColor(data.stats.rank, palette);
  const updated = escapeXml(formatDateLabel(data.stats.updatedAt));

  // Cell size derived so the full year of weeks spans the available width.
  const gridAvailW = INNER - DAY_LABEL_W;
  const stride = (gridAvailW + GAP) / layout.numWeeks;
  const cell = stride - GAP;

  const gridX = PAD + DAY_LABEL_W;
  const monthY = 78;
  const gridTop = 86;
  const gridBottom = gridTop + 7 * stride - GAP;
  const legendY = gridBottom + 24;
  const footerY = legendY + 36;
  const height = footerY + 16;

  const parts: string[] = [];
  const add = (s: string) => parts.push(s);

  add(`<?xml version="1.0" encoding="UTF-8"?>`);
  add(`<svg width="${W}" height="${height}" viewBox="0 0 ${W} ${height}" fill="none" xmlns="http://www.w3.org/2000/svg" role="img" aria-label="Tokscale contribution graph for @${escapeXml(data.user.username)}">`);
  add(`  <defs>`);
  add(`    <style>@import url('${FIGTREE_FONT_IMPORT}');</style>`);
  add(`    <linearGradient id="bg" x1="0" y1="0" x2="${W}" y2="${height}" gradientUnits="userSpaceOnUse"><stop offset="0" stop-color="${palette.bgStart}"/><stop offset="1" stop-color="${palette.bgEnd}"/></linearGradient>`);
  add(`    <radialGradient id="glow" cx="0.9" cy="0.0" r="0.7"><stop offset="0" stop-color="${palette.glowColor}" stop-opacity="${palette.glowOpacity + 0.03}"/><stop offset="1" stop-color="${palette.glowColor}" stop-opacity="0"/></radialGradient>`);
  add(`    <clipPath id="card-clip"><rect width="${W}" height="${height}" rx="16"/></clipPath>`);
  add(`  </defs>`);
  add(`  <rect width="${W}" height="${height}" rx="16" fill="url(#bg)"/>`);
  add(`  <rect width="${W}" height="${height}" rx="16" fill="url(#glow)" clip-path="url(#card-clip)"/>`);
  add(`  <rect x="0.5" y="0.5" width="${W - 1}" height="${height - 1}" rx="15.5" fill="none" stroke="${palette.border}"/>`);

  // Header: brand + username (left), stat strip (right).
  add(`  ${brandIcon(PAD, 40, palette.brand)}`);
  add(`  <text x="${PAD + 20}" y="40" fill="${palette.text}" font-size="16" font-weight="700" font-family="${FIGTREE_FONT_STACK}">@${escapeXml(data.user.username)}</text>`);
  const stat = (label: string, value: string, color: string): string =>
    `<tspan fill="${color}" font-weight="800">${escapeXml(value)}</tspan><tspan fill="${palette.muted}" font-weight="600"> ${label}</tspan>`;
  const sep = `<tspan fill="${palette.divider}">      </tspan>`;
  add(`  <text x="${W - PAD}" y="40" font-size="14" text-anchor="end" font-family="${FIGTREE_FONT_STACK}" xml:space="preserve">${[
    stat("tokens", tokens, palette.brand),
    stat("spent", cost, palette.cost),
    stat(`rank (${sortBy})`, rankText, rankColor),
  ].join(sep)}</text>`);

  // Month labels.
  for (const m of layout.months) {
    add(`  <text x="${(gridX + m.week * stride).toFixed(1)}" y="${monthY}" fill="${palette.muted}" font-size="10" font-family="${FIGTREE_FONT_STACK}">${m.label}</text>`);
  }

  // Day-of-week labels.
  for (const [row, label] of DAY_LABELS) {
    add(`  <text x="${PAD}" y="${(gridTop + row * stride + cell - 1).toFixed(1)}" fill="${palette.muted}" font-size="10" font-family="${FIGTREE_FONT_STACK}">${label}</text>`);
  }

  // The contribution graph — the hero element.
  for (const c of layout.cells) {
    add(`  <rect x="${(gridX + c.week * stride).toFixed(1)}" y="${(gridTop + c.day * stride).toFixed(1)}" width="${cell.toFixed(2)}" height="${cell.toFixed(2)}" rx="2.5" fill="${colors[c.intensity]}"/>`);
  }

  // Legend.
  const legendCell = 11;
  const legendStride = 15;
  let lx = W - PAD - (28 + 5 * legendStride + 34);
  add(`  <text x="${lx}" y="${legendY + legendCell - 1}" fill="${palette.muted}" font-size="10" font-family="${FIGTREE_FONT_STACK}">Less</text>`);
  lx += 28;
  for (let i = 0; i < 5; i++) {
    add(`  <rect x="${lx}" y="${legendY}" width="${legendCell}" height="${legendCell}" rx="2.5" fill="${colors[i]}"/>`);
    lx += legendStride;
  }
  add(`  <text x="${lx + 4}" y="${legendY + legendCell - 1}" fill="${palette.muted}" font-size="10" font-family="${FIGTREE_FONT_STACK}">More</text>`);
  add(`  <text x="${PAD}" y="${legendY + legendCell - 1}" fill="${palette.muted}" font-size="11" font-family="${FIGTREE_FONT_STACK}">${layout.activeDays} active days in the last year</text>`);

  // Footer.
  add(`  <text x="${PAD}" y="${footerY}" fill="${palette.muted}" font-size="11" font-family="${FIGTREE_FONT_STACK}">${updated}</text>`);
  add(`  <text x="${W - PAD}" y="${footerY}" fill="${palette.muted}" font-size="11" text-anchor="end" font-family="${FIGTREE_FONT_STACK}">tokscale.ai/u/${escapeXml(data.user.username)}</text>`);
  add(`</svg>`);

  return parts.join("\n");
}
