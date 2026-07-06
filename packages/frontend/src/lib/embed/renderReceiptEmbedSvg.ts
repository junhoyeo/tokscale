/**
 * "receipt" embed template — usage rendered as a thermal-printer receipt:
 * a narrow torn-edge column of monospace line items with dot leaders, a
 * total, and a faux barcode. The narrowest template.
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
const AMP = 7;
const PERIOD = 14;
const MX = 34;
const CHAR = 7.05;

function zigzag(fromX: number, toX: number, base: number, peak: number): string {
  let d = "";
  const dir = toX > fromX ? 1 : -1;
  const steps = Math.round(Math.abs(toX - fromX) / PERIOD);
  for (let i = 0; i < steps; i++) {
    const x0 = fromX + dir * i * PERIOD;
    d += ` L ${(x0 + dir * PERIOD / 2).toFixed(1)} ${peak} L ${(x0 + dir * PERIOD).toFixed(1)} ${base}`;
  }
  return d;
}

export function renderReceiptEmbedSvg(
  data: UserEmbedStats,
  options: RenderReceiptEmbedOptions = {},
): string {
  const theme: EmbedTheme = options.theme === "light" ? "light" : "dark";
  const palette = resolvePalette(theme, options.color ?? null);
  const tokensFormat: EmbedNumberFormat = options.tokensFormat ?? "full";
  const costFormat: EmbedNumberFormat = options.costFormat ?? "full";

  const paper = theme === "light" ? "#f6f4ee" : "#171a20";
  const ink = theme === "light" ? "#2f2d28" : "#d7dae1";
  const faint = theme === "light" ? "#b3afa3" : "#5b606b";
  const accent = palette.brand;

  const layout = layoutContributions(options.contributions ?? []);
  const showGraph = options.graph ?? false;
  const drawGraph = showGraph && (options.contributions?.length ?? 0) > 0;
  const GBAND = 72;
  const H = 354 + (drawGraph ? GBAND : 0);
  const rankText = data.stats.rank
    ? formatRank(data.stats.rank, data.stats.rankTotal ?? null, options.rankFormat)
    : "UNRANKED";
  const items: [string, string][] = [
    ["TOKENS", formatNumber(data.stats.totalTokens, tokensFormat === "compact")],
    ["ACTIVE DAYS", String(layout.activeDays)],
    ["LEADERBOARD", rankText],
  ];
  const total = formatCurrency(data.stats.totalCost, costFormat === "compact");
  const dateLabel = formatDateLabel(data.stats.updatedAt).replace(/^Updated /, "");

  const parts: string[] = [];
  const add = (s: string) => parts.push(s);

  add(`<?xml version="1.0" encoding="UTF-8"?>`);
  add(`<svg width="${W}" height="${H}" viewBox="0 0 ${W} ${H}" fill="none" xmlns="http://www.w3.org/2000/svg" role="img" aria-label="Tokscale usage receipt for @${escapeXml(data.user.username)}">`);

  // Torn-edge paper shape.
  const paperPath = `M 0 ${AMP}${zigzag(0, W, AMP, 0)} L ${W} ${H - AMP}${zigzag(W, 0, H - AMP, H)} Z`;
  add(`  <path d="${paperPath}" fill="${paper}"/>`);

  const mono = `font-family="${MONO_FONT_STACK}"`;
  const center = W / 2;

  // Header.
  add(`  <text x="${center}" y="44" fill="${ink}" font-size="17" font-weight="700" letter-spacing="0.16em" text-anchor="middle" ${mono}>TOKSCALE</text>`);
  add(`  <text x="${center}" y="62" fill="${faint}" font-size="10" letter-spacing="0.18em" text-anchor="middle" ${mono}>AI USAGE RECEIPT</text>`);
  add(`  <line x1="${MX}" y1="78" x2="${W - MX}" y2="78" stroke="${faint}" stroke-width="1" stroke-dasharray="2 3"/>`);

  // Meta.
  add(`  <text x="${MX}" y="98" fill="${ink}" font-size="11" ${mono}>CUSTOMER  @${escapeXml(data.user.username)}</text>`);
  add(`  <text x="${MX}" y="115" fill="${ink}" font-size="11" ${mono}>DATE      ${escapeXml(dateLabel)}</text>`);
  add(`  <line x1="${MX}" y1="129" x2="${W - MX}" y2="129" stroke="${faint}" stroke-width="1" stroke-dasharray="2 3"/>`);

  // Line items with dot leaders.
  items.forEach(([label, value], i) => {
    const y = 153 + i * 26;
    add(`  <text x="${MX}" y="${y}" fill="${ink}" font-size="12" font-weight="600" ${mono}>${escapeXml(label)}</text>`);
    add(`  <text x="${W - MX}" y="${y}" fill="${ink}" font-size="12" font-weight="700" text-anchor="end" ${mono}>${escapeXml(value)}</text>`);
    const dotsX1 = MX + label.length * CHAR + 8;
    const dotsX2 = W - MX - value.length * CHAR - 8;
    if (dotsX2 > dotsX1) {
      add(`  <line x1="${dotsX1.toFixed(1)}" y1="${y - 3}" x2="${dotsX2.toFixed(1)}" y2="${y - 3}" stroke="${faint}" stroke-width="1" stroke-dasharray="1 3"/>`);
    }
  });

  // Total.
  const ty = 153 + items.length * 26 + 6;
  add(`  <line x1="${MX}" y1="${ty}" x2="${W - MX}" y2="${ty}" stroke="${ink}" stroke-width="1.5"/>`);
  add(`  <text x="${MX}" y="${ty + 26}" fill="${ink}" font-size="14" font-weight="700" ${mono}>TOTAL SPENT</text>`);
  add(`  <text x="${W - MX}" y="${ty + 26}" fill="${accent}" font-size="17" font-weight="800" text-anchor="end" ${mono}>${escapeXml(total)}</text>`);

  // Optional contribution graph.
  if (drawGraph) {
    const colors = gradeColors(palette);
    // grade0 is tuned for the card background, not the receipt paper — give
    // empty days a tone that actually reads against it.
    const emptyCell = theme === "light" ? "#e4e0d4" : "#2c303a";
    const GAP = 1;
    const STRIDE = (W - MX * 2) / layout.numWeeks;
    const CELL = STRIDE - GAP;
    const gTop = ty + 52;
    add(`  <text x="${center}" y="${ty + 44}" fill="${faint}" font-size="10" letter-spacing="0.14em" text-anchor="middle" ${mono}>DAILY ACTIVITY</text>`);
    for (const c of layout.cells) {
      const fill = c.intensity === 0 ? emptyCell : colors[c.intensity];
      add(`  <rect x="${(MX + c.week * STRIDE).toFixed(2)}" y="${(gTop + c.day * STRIDE).toFixed(2)}" width="${CELL.toFixed(2)}" height="${CELL.toFixed(2)}" fill="${fill}"/>`);
    }
  }

  // Barcode (deterministic from the username).
  const name = data.user.username || "tokscale";
  let bx = MX;
  const barTop = ty + 44 + (drawGraph ? GBAND : 0);
  while (bx < W - MX - 2) {
    const code = name.charCodeAt(Math.floor((bx - MX) / 4) % name.length);
    const bw = 1 + (code % 3);
    if (code % 2 === 0) {
      add(`  <rect x="${bx.toFixed(1)}" y="${barTop}" width="${bw}" height="34" fill="${ink}"/>`);
    }
    bx += bw + 2;
  }
  add(`  <text x="${center}" y="${barTop + 50}" fill="${faint}" font-size="9" letter-spacing="0.22em" text-anchor="middle" ${mono}>THANK YOU FOR VIBE CODING</text>`);
  add(`  <text x="${center}" y="${barTop + 64}" fill="${faint}" font-size="9" letter-spacing="0.1em" text-anchor="middle" ${mono}>tokscale.ai/u/${escapeXml(data.user.username)}</text>`);

  add(`</svg>`);
  return parts.join("\n");
}
