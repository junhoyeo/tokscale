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
const FONT_FAMILY = "Verdana,Geneva,DejaVu Sans,sans-serif";
const HORIZ_PADDING = 6;
// ~6.8px/char at Verdana 11px — shields.io standard heuristic
const CHAR_WIDTH = 6.8;
const LABEL_BG = "#555";
const MAX_LABEL_LENGTH = 40;

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

function textWidth(text: string): number {
  return text.length * CHAR_WIDTH;
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
  // Verdana 11px: shadow 1px above visible text for depth effect
  const shadowY = BADGE_HEIGHT / 2 + 1;
  const textY = BADGE_HEIGHT / 2 + 4;

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
  <g fill="#fff" text-anchor="middle" font-family="${FONT_FAMILY}" text-rendering="geometricPrecision" font-size="${FONT_SIZE}">
    <text x="${labelX}" y="${shadowY}" fill="#010101" fill-opacity=".3" textLength="${textWidth(label)}">${escapeXml(label)}</text>
    <text x="${labelX}" y="${textY}" fill="#fff" textLength="${textWidth(label)}">${escapeXml(label)}</text>
    <text x="${valueX}" y="${shadowY}" fill="#010101" fill-opacity=".3" textLength="${textWidth(value)}">${escapeXml(value)}</text>
    <text x="${valueX}" y="${textY}" fill="#fff" textLength="${textWidth(value)}">${escapeXml(value)}</text>
  </g>
</svg>`;
}

function renderFlatSquareBadge(label: string, value: string, labelBg: string, valueBg: string): string {
  const labelWidth = textWidth(label) + HORIZ_PADDING * 2;
  const valueWidth = textWidth(value) + HORIZ_PADDING * 2;
  const totalWidth = labelWidth + valueWidth;
  const labelX = labelWidth / 2;
  const valueX = labelWidth + valueWidth / 2;
  const textY = BADGE_HEIGHT / 2 + 4;

  return `<svg xmlns="http://www.w3.org/2000/svg" width="${totalWidth}" height="${BADGE_HEIGHT}" role="img" aria-label="${escapeXml(label)}: ${escapeXml(value)}">
  <title>${escapeXml(label)}: ${escapeXml(value)}</title>
  <g shape-rendering="crispEdges">
    <rect width="${labelWidth}" height="${BADGE_HEIGHT}" fill="${labelBg}"/>
    <rect x="${labelWidth}" width="${valueWidth}" height="${BADGE_HEIGHT}" fill="#${escapeXml(valueBg)}"/>
  </g>
  <g fill="#fff" text-anchor="middle" font-family="${FONT_FAMILY}" text-rendering="geometricPrecision" font-size="${FONT_SIZE}">
    <text x="${labelX}" y="${textY}">${escapeXml(label)}</text>
    <text x="${valueX}" y="${textY}">${escapeXml(value)}</text>
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
