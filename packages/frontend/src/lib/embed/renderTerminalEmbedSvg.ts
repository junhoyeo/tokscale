/**
 * "terminal" embed template — a terminal-window card with a monospace
 * command-output aesthetic: window chrome, a prompt line, aligned key/value
 * stats, and the contribution graph as a block grid.
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
  MONO_FONT_STACK,
  escapeXml,
  formatRank,
  type EmbedRankFormat,
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
const INNER = W - PAD * 2;
const CELL = 8;
const GAP = 2;
const STRIDE = CELL + GAP;
const TITLE_BAR = 38;

export function renderTerminalEmbedSvg(
  data: UserEmbedStats,
  options: RenderTerminalEmbedOptions = {},
): string {
  const theme: EmbedTheme = options.theme === "light" ? "light" : "dark";
  const palette = resolvePalette(theme, options.color ?? null);
  const tokensFormat: EmbedNumberFormat = options.tokensFormat ?? "full";
  const costFormat: EmbedNumberFormat = options.costFormat ?? "compact";
  const sortBy = options.sortBy === "cost" ? "cost" : "tokens";

  const showGraph = options.graph ?? false;
  const contributions = showGraph && options.contributions && options.contributions.length > 0
    ? options.contributions
    : null;
  const layout = contributions ? layoutContributions(contributions) : null;

  const tokens = formatNumber(data.stats.totalTokens, tokensFormat === "compact");
  const cost = formatCurrency(data.stats.totalCost, costFormat === "compact");
  const rankText = data.stats.rank
    ? formatRank(data.stats.rank, data.stats.rankTotal ?? null, options.rankFormat)
    : "unranked";
  const rankColor = getRankColor(data.stats.rank, palette);
  const updated = escapeXml(formatDateLabel(data.stats.updatedAt));

  const rows: { label: string; value: string; color: string }[] = [
    { label: "tokens", value: tokens, color: palette.brand },
    { label: "cost", value: cost, color: palette.cost },
    { label: "rank", value: `${rankText}  (${sortBy})`, color: rankColor },
  ];
  if (layout) {
    rows.push({ label: "active", value: `${layout.activeDays} days`, color: palette.text });
  }

  const promptY = TITLE_BAR + 32;
  const rowsTop = promptY + 30;
  const rowGap = 24;
  const rowsBottom = rowsTop + rows.length * rowGap;
  const graphLabelY = rowsBottom + 14;
  const monthY = graphLabelY + 16;
  const gridTop = monthY + 6;
  const gridBottom = gridTop + 7 * STRIDE - GAP;
  const height = (layout ? gridBottom + 36 : rowsBottom + 24) + 4;
  const footerY = height - 16;

  const parts: string[] = [];
  const add = (s: string) => parts.push(s);

  add(`<?xml version="1.0" encoding="UTF-8"?>`);
  add(`<svg width="${W}" height="${height}" viewBox="0 0 ${W} ${height}" fill="none" xmlns="http://www.w3.org/2000/svg" role="img" aria-label="Tokscale stats for @${escapeXml(data.user.username)}">`);
  add(`  <defs><clipPath id="win"><rect width="${W}" height="${height}" rx="12"/></clipPath></defs>`);

  // Window body + title bar.
  add(`  <rect width="${W}" height="${height}" rx="12" fill="${palette.bgStart}"/>`);
  add(`  <g clip-path="url(#win)"><rect width="${W}" height="${TITLE_BAR}" fill="${palette.graphGrade0}"/></g>`);
  add(`  <line x1="0" y1="${TITLE_BAR}" x2="${W}" y2="${TITLE_BAR}" stroke="${palette.border}"/>`);
  add(`  <rect x="0.5" y="0.5" width="${W - 1}" height="${height - 1}" rx="11.5" fill="none" stroke="${palette.border}"/>`);

  // Traffic-light dots + window title.
  const dotY = TITLE_BAR / 2;
  add(`  <circle cx="${PAD + 4}" cy="${dotY}" r="5.5" fill="#FF5F56"/>`);
  add(`  <circle cx="${PAD + 22}" cy="${dotY}" r="5.5" fill="#FFBD2E"/>`);
  add(`  <circle cx="${PAD + 40}" cy="${dotY}" r="5.5" fill="#27C93F"/>`);
  add(`  <text x="${PAD + 60}" y="${dotY + 4}" fill="${palette.muted}" font-size="12" font-family="${MONO_FONT_STACK}">tokscale — @${escapeXml(data.user.username)}</text>`);

  // Prompt line.
  add(`  <text x="${PAD}" y="${promptY}" font-size="13" font-family="${MONO_FONT_STACK}"><tspan fill="${palette.brand}" font-weight="700">$</tspan><tspan fill="${palette.text}"> tokscale stats</tspan></text>`);

  // Aligned key/value rows.
  rows.forEach((row, i) => {
    const y = rowsTop + i * rowGap;
    add(`  <text x="${PAD}" y="${y}" fill="${palette.muted}" font-size="13" font-family="${MONO_FONT_STACK}">${row.label.padEnd(9, " ")}</text>`);
    add(`  <text x="${PAD + 92}" y="${y}" fill="${row.color}" font-size="13" font-weight="700" font-family="${MONO_FONT_STACK}">${escapeXml(row.value)}</text>`);
  });

  // Contribution graph.
  if (layout) {
    const colors = gradeColors(palette);
    const gridW = layout.numWeeks * STRIDE - GAP;
    const gx = PAD + Math.max(0, (INNER - gridW) / 2);
    add(`  <text x="${PAD}" y="${graphLabelY}" fill="${palette.muted}" font-size="12" font-family="${MONO_FONT_STACK}">contribution graph</text>`);
    for (const m of layout.months) {
      add(`  <text x="${(gx + m.week * STRIDE).toFixed(1)}" y="${monthY}" fill="${palette.muted}" font-size="9" font-family="${MONO_FONT_STACK}">${m.label}</text>`);
    }
    for (const c of layout.cells) {
      add(`  <rect x="${(gx + c.week * STRIDE).toFixed(1)}" y="${gridTop + c.day * STRIDE}" width="${CELL}" height="${CELL}" rx="1.5" fill="${colors[c.intensity]}"/>`);
    }
  }

  // Footer.
  add(`  <text x="${PAD}" y="${footerY}" fill="${palette.muted}" font-size="11" font-family="${MONO_FONT_STACK}">${updated}</text>`);
  add(`  <text x="${W - PAD}" y="${footerY}" fill="${palette.muted}" font-size="11" text-anchor="end" font-family="${MONO_FONT_STACK}">tokscale.ai/u/${escapeXml(data.user.username)}</text>`);
  add(`</svg>`);

  return parts.join("\n");
}
