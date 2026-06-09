import type { UserEmbedStats } from "./getUserEmbedStats";
import { escapeXml, formatNumber, formatCurrency } from "../format";

export type BadgeMetric = "tokens" | "cost" | "rank";
export type BadgeStyle = "flat" | "flat-square";
export type BadgeSortBy = "tokens" | "cost";

export interface RenderProfileBadgeOptions {
  metric?: BadgeMetric;
  style?: BadgeStyle;
  label?: string;
  color?: string;
  sort?: BadgeSortBy;
  compact?: boolean;
}

const BADGE_HEIGHT = 20;
const FONT_SIZE = 11;
const FONT_SCALE = 10;
const FONT_FAMILY = "Verdana,Geneva,DejaVu Sans,sans-serif";
const HORIZ_PADDING = 6;
const LABEL_BG = "#555";
const MAX_LABEL_LENGTH = 40;

// prettier-ignore
// Verdana 11px normal per-character widths (px), charCode 32–126. Source: anafanafo 2.0
const VERDANA_WIDTHS = [
  3.87,4.33,5.05,9,6.99,11.84,7.99,2.95,5,5,6.99,9,4,5,4,5,
  6.99,6.99,6.99,6.99,6.99,6.99,6.99,6.99,6.99,6.99,5,5,9,9,9,6,11,
  7.52,7.54,7.68,8.48,6.96,6.32,8.53,8.27,4.63,5,7.62,6.12,9.27,8.23,8.66,6.63,
  8.66,7.65,7.52,6.78,8.05,7.52,10.88,7.54,6.77,7.54,5,5,5,9,6.99,6.99,
  6.61,6.85,5.73,6.85,6.55,3.87,6.85,6.96,3.02,3.79,6.51,3.02,10.7,6.96,6.68,
  6.85,6.85,4.69,5.73,4.33,6.96,6.51,9,6.51,6.51,5.78,6.98,5,6.98,9,
];

const METRIC_COLORS: Record<BadgeMetric, string> = {
  tokens: "0073FF",
  cost: "16804B",
  rank: "D97706",
};

const METRIC_LABELS: Record<BadgeMetric, string> = {
  tokens: "Tokscale Tokens",
  cost: "Tokscale Cost",
  rank: "Tokscale Rank",
};

function isFullWidth(code: number): boolean {
  return (
    (code >= 0x1100 && code <= 0x115f) ||
    (code >= 0x2e80 && code <= 0x9fff) ||
    (code >= 0xac00 && code <= 0xd7af) ||
    (code >= 0xf900 && code <= 0xfaff) ||
    (code >= 0xfe10 && code <= 0xfe6f) ||
    (code >= 0xff01 && code <= 0xff60) ||
    (code >= 0xffe0 && code <= 0xffe6) ||
    (code >= 0x20000 && code <= 0x2fa1f)
  );
}

function textWidth(text: string): number {
  let w = 0;
  for (let i = 0; i < text.length; ) {
    const code = text.codePointAt(i)!;
    const idx = code - 32;
    if (idx >= 0 && idx < VERDANA_WIDTHS.length) {
      w += VERDANA_WIDTHS[idx];
    } else if (isFullWidth(code)) {
      w += FONT_SIZE;
    } else {
      w += 6.99;
    }
    i += code > 0xffff ? 2 : 1;
  }
  return w;
}

function formatMetricValue(data: UserEmbedStats, metric: BadgeMetric, compact: boolean): string {
  switch (metric) {
    case "tokens":
      return formatNumber(data.stats.totalTokens, compact);
    case "cost":
      return formatCurrency(data.stats.totalCost, compact);
    case "rank":
      return data.stats.rank ? `#${data.stats.rank}` : "N/A";
  }
}

function parseColor(color: string | undefined, fallback: string): string {
  if (!color) return fallback;
  const hex = color.replace(/^#/, "");
  if (/^(?:[0-9a-fA-F]{3}|[0-9a-fA-F]{4}|[0-9a-fA-F]{6}|[0-9a-fA-F]{8})$/.test(hex)) return hex;
  return fallback;
}

function renderFlatBadge(label: string, value: string, labelBg: string, valueBg: string): string {
  const labelWidth = textWidth(label) + HORIZ_PADDING * 2;
  const valueWidth = textWidth(value) + HORIZ_PADDING * 2;
  const totalWidth = labelWidth + valueWidth;
  const labelX = labelWidth / 2;
  const valueX = labelWidth + valueWidth / 2;

  const s = FONT_SCALE;
  const labelX10 = Math.round(labelX * s);
  const valueX10 = Math.round(valueX * s);
  const labelTextLen = Math.round(textWidth(label) * s);
  const valueTextLen = Math.round(textWidth(value) * s);

  return `<svg xmlns="http://www.w3.org/2000/svg" width="${totalWidth}" height="${BADGE_HEIGHT}" role="img" aria-label="${escapeXml(label)}: ${escapeXml(value)}">
  <title>${escapeXml(label)}: ${escapeXml(value)}</title>
  <linearGradient id="s" x2="0" y2="100%">
    <stop offset="0" stop-color="#bbb" stop-opacity=".1"/>
    <stop offset="1" stop-opacity=".1"/>
  </linearGradient>
  <clipPath id="r">
    <rect width="${totalWidth}" height="${BADGE_HEIGHT}" rx="3" fill="#fff"/>
  </clipPath>
  <g clip-path="url(#r)">
    <rect width="${labelWidth}" height="${BADGE_HEIGHT}" fill="${labelBg}"/>
    <rect x="${labelWidth}" width="${valueWidth}" height="${BADGE_HEIGHT}" fill="#${escapeXml(valueBg)}"/>
    <rect width="${totalWidth}" height="${BADGE_HEIGHT}" fill="url(#s)"/>
  </g>
  <g fill="#fff" text-anchor="middle" font-family="${FONT_FAMILY}" text-rendering="geometricPrecision" font-size="${FONT_SIZE * s}">
    <text aria-hidden="true" x="${labelX10}" y="150" fill="#010101" fill-opacity=".3" transform="scale(.1)" textLength="${labelTextLen}">${escapeXml(label)}</text>
    <text x="${labelX10}" y="140" transform="scale(.1)" fill="#fff" textLength="${labelTextLen}">${escapeXml(label)}</text>
    <text aria-hidden="true" x="${valueX10}" y="150" fill="#010101" fill-opacity=".3" transform="scale(.1)" textLength="${valueTextLen}">${escapeXml(value)}</text>
    <text x="${valueX10}" y="140" transform="scale(.1)" fill="#fff" textLength="${valueTextLen}">${escapeXml(value)}</text>
  </g>
</svg>`;
}

function renderFlatSquareBadge(label: string, value: string, labelBg: string, valueBg: string): string {
  const labelWidth = textWidth(label) + HORIZ_PADDING * 2;
  const valueWidth = textWidth(value) + HORIZ_PADDING * 2;
  const totalWidth = labelWidth + valueWidth;
  const labelX = labelWidth / 2;
  const valueX = labelWidth + valueWidth / 2;

  const s = FONT_SCALE;
  const labelX10 = Math.round(labelX * s);
  const valueX10 = Math.round(valueX * s);
  const labelTextLen = Math.round(textWidth(label) * s);
  const valueTextLen = Math.round(textWidth(value) * s);

  return `<svg xmlns="http://www.w3.org/2000/svg" width="${totalWidth}" height="${BADGE_HEIGHT}" role="img" aria-label="${escapeXml(label)}: ${escapeXml(value)}">
  <title>${escapeXml(label)}: ${escapeXml(value)}</title>
  <g shape-rendering="crispEdges">
    <rect width="${labelWidth}" height="${BADGE_HEIGHT}" fill="${labelBg}"/>
    <rect x="${labelWidth}" width="${valueWidth}" height="${BADGE_HEIGHT}" fill="#${escapeXml(valueBg)}"/>
  </g>
  <g fill="#fff" text-anchor="middle" font-family="${FONT_FAMILY}" text-rendering="geometricPrecision" font-size="${FONT_SIZE * s}">
    <text x="${labelX10}" y="140" transform="scale(.1)" fill="#fff" textLength="${labelTextLen}">${escapeXml(label)}</text>
    <text x="${valueX10}" y="140" transform="scale(.1)" fill="#fff" textLength="${valueTextLen}">${escapeXml(value)}</text>
  </g>
</svg>`;
}

export function renderProfileBadgeSvg(
  data: UserEmbedStats,
  options: RenderProfileBadgeOptions = {},
): string {
  const metric: BadgeMetric = (["tokens", "cost", "rank"] as const).includes(options.metric as BadgeMetric)
    ? (options.metric as BadgeMetric)
    : "tokens";
  const style: BadgeStyle = options.style === "flat-square" ? "flat-square" : "flat";
  const rawLabel = options.label ?? METRIC_LABELS[metric];
  const label = rawLabel.length > MAX_LABEL_LENGTH ? rawLabel.slice(0, MAX_LABEL_LENGTH) : rawLabel;
  const compact = options.compact ?? false;
  const valueBg = parseColor(options.color, METRIC_COLORS[metric]);
  const value = formatMetricValue(data, metric, compact);

  if (style === "flat-square") {
    return renderFlatSquareBadge(label, value, LABEL_BG, valueBg);
  }
  return renderFlatBadge(label, value, LABEL_BG, valueBg);
}

export function renderBadgeErrorSvg(
  message: string,
  options: Pick<RenderProfileBadgeOptions, "style" | "label"> = {},
): string {
  const style: BadgeStyle = options.style === "flat-square" ? "flat-square" : "flat";
  const rawLabel = options.label ?? "Tokscale";
  const label = rawLabel.length > MAX_LABEL_LENGTH ? rawLabel.slice(0, MAX_LABEL_LENGTH) : rawLabel;
  const value = message;

  if (style === "flat-square") {
    return renderFlatSquareBadge(label, value, LABEL_BG, "e05d44");
  }
  return renderFlatBadge(label, value, LABEL_BG, "e05d44");
}
