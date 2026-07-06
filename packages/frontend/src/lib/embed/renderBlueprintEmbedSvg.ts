/**
 * "blueprint" embed template — an engineering-schematic card: a fine grid,
 * dimensioned stat values, a contribution "activity profile", and a drawing
 * title block along the bottom.
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
  MONO_FONT_STACK,
  escapeXml,
  formatRank,
  type EmbedRankFormat,
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
const INNER = W - PAD * 2;
const CELL = 7;
const GAP = 1.5;
const STRIDE = CELL + GAP;

export function renderBlueprintEmbedSvg(
  data: UserEmbedStats,
  options: RenderBlueprintEmbedOptions = {},
): string {
  const theme: EmbedTheme = options.theme === "light" ? "light" : "dark";
  const palette = resolvePalette(theme, options.color ?? null);
  const grades = gradeColors(palette);
  const tokensFormat: EmbedNumberFormat = options.tokensFormat ?? "full";
  const costFormat: EmbedNumberFormat = options.costFormat ?? "full";
  const accent = palette.brand;
  const showGraph = options.graph ?? false;
  const H = showGraph ? 278 : 198;

  const layout = layoutContributions(options.contributions ?? []);
  const rankText = data.stats.rank
    ? formatRank(data.stats.rank, data.stats.rankTotal ?? null, options.rankFormat)
    : "UNRANKED";

  const stats = [
    { value: formatNumber(data.stats.totalTokens, tokensFormat === "compact"), label: "TOKENS" },
    { value: formatCurrency(data.stats.totalCost, costFormat === "compact"), label: "COST" },
    { value: rankText, label: "LEADERBOARD RANK" },
  ];

  const updated = escapeXml(formatDateLabel(data.stats.updatedAt).replace(/^Updated /, ""));
  const parts: string[] = [];
  const add = (s: string) => parts.push(s);

  add(`<?xml version="1.0" encoding="UTF-8"?>`);
  add(`<svg width="${W}" height="${H}" viewBox="0 0 ${W} ${H}" fill="none" xmlns="http://www.w3.org/2000/svg" role="img" aria-label="Tokscale usage schematic for @${escapeXml(data.user.username)}">`);
  add(`  <defs>`);
  add(`    <pattern id="grid" width="24" height="24" patternUnits="userSpaceOnUse"><path d="M24 0H0V24" fill="none" stroke="${palette.divider}" stroke-width="0.5" opacity="0.45"/></pattern>`);
  add(`    <clipPath id="card-clip"><rect width="${W}" height="${H}" rx="14"/></clipPath>`);
  add(`  </defs>`);
  add(`  <rect width="${W}" height="${H}" rx="14" fill="${palette.bgStart}"/>`);
  add(`  <rect width="${W}" height="${H}" fill="url(#grid)" clip-path="url(#card-clip)"/>`);
  add(`  <rect x="0.5" y="0.5" width="${W - 1}" height="${H - 1}" rx="13.5" fill="none" stroke="${palette.border}"/>`);
  add(`  <rect x="9" y="9" width="${W - 18}" height="${H - 18}" fill="none" stroke="${accent}" stroke-opacity="0.4"/>`);

  // Corner registration ticks.
  for (const [cx, cy, dx, dy] of [[9, 9, 1, 1], [W - 9, 9, -1, 1], [9, H - 9, 1, -1], [W - 9, H - 9, -1, -1]] as const) {
    add(`  <path d="M${cx} ${cy + dy * 10}V${cy}H${cx + dx * 10}" fill="none" stroke="${accent}" stroke-width="1.5"/>`);
  }

  // Header.
  add(`  <text x="${PAD}" y="40" font-family="${MONO_FONT_STACK}"><tspan fill="${accent}" font-size="16" font-weight="700" letter-spacing="0.04em">TOKSCALE</tspan><tspan fill="${palette.muted}" font-size="11" letter-spacing="0.14em">  AI USAGE SCHEMATIC</tspan></text>`);
  add(`  <text x="${W - PAD}" y="40" fill="${palette.text}" font-size="12" font-weight="700" text-anchor="end" font-family="${MONO_FONT_STACK}">@${escapeXml(data.user.username)}</text>`);
  add(`  <line x1="${PAD}" y1="52" x2="${W - PAD}" y2="52" stroke="${palette.divider}"/>`);

  // Dimensioned stats.
  stats.forEach((s, i) => {
    const cx = PAD + INNER / 6 + i * (INNER / 3);
    // Shrink long values so they never collide with the neighbouring column.
    const vSize = Math.max(13, Math.min(24, 168 / (s.value.length * 0.62)));
    add(`  <text x="${cx.toFixed(1)}" y="100" fill="${palette.text}" font-size="${vSize.toFixed(1)}" font-weight="700" text-anchor="middle" font-family="${MONO_FONT_STACK}">${escapeXml(s.value)}</text>`);
    const half = 84;
    add(`  <line x1="${cx - half}" y1="116" x2="${cx + half}" y2="116" stroke="${accent}" stroke-width="1"/>`);
    add(`  <line x1="${cx - half}" y1="112" x2="${cx - half}" y2="120" stroke="${accent}" stroke-width="1"/>`);
    add(`  <line x1="${cx + half}" y1="112" x2="${cx + half}" y2="120" stroke="${accent}" stroke-width="1"/>`);
    add(`  <text x="${cx.toFixed(1)}" y="134" fill="${palette.muted}" font-size="10" letter-spacing="0.1em" text-anchor="middle" font-family="${MONO_FONT_STACK}">${s.label}</text>`);
  });

  // Activity profile (contribution grid).
  if (showGraph) {
    add(`  <text x="${PAD}" y="162" fill="${palette.muted}" font-size="10" letter-spacing="0.1em" font-family="${MONO_FONT_STACK}">ACTIVITY PROFILE / ${layout.activeDays} ACTIVE DAYS</text>`);
    const gridW = layout.numWeeks * STRIDE - GAP;
    const gx = PAD + (INNER - gridW) / 2;
    const gy = 170;
    for (const c of layout.cells) {
      add(`  <rect x="${(gx + c.week * STRIDE).toFixed(2)}" y="${gy + c.day * STRIDE}" width="${CELL}" height="${CELL}" fill="${grades[c.intensity]}" stroke="${palette.bgStart}" stroke-width="0.5"/>`);
    }
  }

  // Title block.
  const tbY = H - 40;
  const tbH = 28;
  const shortUser = data.user.username.length > 20
    ? `${data.user.username.slice(0, 19)}…`
    : data.user.username;
  const cells = [
    `BY    @${shortUser}`,
    `DATE    ${updated}`,
    `SHEET    tokscale.ai`,
  ];
  add(`  <rect x="${PAD}" y="${tbY}" width="${INNER}" height="${tbH}" fill="none" stroke="${accent}" stroke-opacity="0.55"/>`);
  cells.forEach((text, i) => {
    const cw = INNER / 3;
    const cxs = PAD + i * cw;
    if (i > 0) add(`  <line x1="${cxs}" y1="${tbY}" x2="${cxs}" y2="${tbY + tbH}" stroke="${accent}" stroke-opacity="0.55"/>`);
    add(`  <text x="${cxs + 12}" y="${tbY + tbH / 2 + 4}" fill="${palette.muted}" font-size="10" font-family="${MONO_FONT_STACK}">${escapeXml(text)}</text>`);
  });

  add(`</svg>`);
  return parts.join("\n");
}
