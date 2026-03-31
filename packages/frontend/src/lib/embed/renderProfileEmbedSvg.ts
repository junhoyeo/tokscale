import type { UserEmbedStats } from "./getUserEmbedStats";
import { escapeXml, formatCompact, formatNumber, formatCurrency } from "../format";

export type EmbedTheme = "dark" | "light";
export type EmbedSortBy = "tokens" | "cost";

export interface RenderProfileEmbedOptions {
  theme?: EmbedTheme;
  compact?: boolean;
  compactNumbers?: boolean;
  sortBy?: EmbedSortBy;
}

type ThemePalette = {
  background: string;
  backgroundEnd: string;
  shell: string;
  shellEnd: string;
  surface: string;
  surfaceAlt: string;
  border: string;
  borderSoft: string;
  title: string;
  text: string;
  muted: string;
  accentSoft: string;
  accentGlow: string;
  tokenGradientStart: string;
  tokenGradientMid: string;
  tokenGradientEnd: string;
  costAccent: string;
  highlight: string;
};

const THEMES: Record<EmbedTheme, ThemePalette> = {
  dark: {
    background: "#10121C",
    backgroundEnd: "#080B12",
    shell: "#141A21",
    shellEnd: "#111113",
    surface: "#111822",
    surfaceAlt: "#0D141D",
    border: "#1E2733",
    borderSoft: "rgba(133, 202, 255, 0.14)",
    title: "#FFFFFF",
    text: "#E6EDF7",
    muted: "#7691B7",
    accentSoft: "#85CAFF",
    accentGlow: "rgba(22, 154, 255, 0.22)",
    tokenGradientStart: "#169AFF",
    tokenGradientMid: "#9FD4FB",
    tokenGradientEnd: "#B9DFF8",
    costAccent: "#53D18C",
    highlight: "rgba(83, 209, 243, 0.18)",
  },
  light: {
    background: "#F6FAFF",
    backgroundEnd: "#EAF2FF",
    shell: "#FFFFFF",
    shellEnd: "#F7FAFF",
    surface: "#F8FBFF",
    surfaceAlt: "#EEF5FF",
    border: "#D6E5F8",
    borderSoft: "rgba(22, 154, 255, 0.14)",
    title: "#0F172A",
    text: "#18324D",
    muted: "#5F7FA5",
    accentSoft: "#56B9FF",
    accentGlow: "rgba(22, 154, 255, 0.12)",
    tokenGradientStart: "#0073FF",
    tokenGradientMid: "#53B9FF",
    tokenGradientEnd: "#8FD6FF",
    costAccent: "#16804B",
    highlight: "rgba(83, 209, 243, 0.12)",
  },
};

const FIGTREE_FONT_STACK = "Figtree, -apple-system, BlinkMacSystemFont, Segoe UI, sans-serif";
const FIGTREE_FONT_IMPORT = "https://fonts.googleapis.com/css2?family=Figtree:wght@400;600;700;800&amp;display=swap";

function formatDateLabel(value: string | null): string {
  if (!value) {
    return "No submissions yet";
  }

  const date = new Date(value);
  if (Number.isNaN(date.getTime())) {
    return "Updated recently";
  }

  return `Updated ${new Intl.DateTimeFormat("en-US", {
    month: "short",
    day: "numeric",
    year: "numeric",
    timeZone: "UTC",
  }).format(date)} (UTC)`;
}

// Approximate average character width for Figtree 15px semibold (weight 600).
// Used to estimate rendered username width for dynamic display-name positioning.
const APPROX_CHAR_WIDTH_15_SEMIBOLD = 9;
// Approximate average character width for Figtree 13px (weight 400).
// Used to estimate rendered display name width for collision detection.
const APPROX_CHAR_WIDTH_13 = 8;

function getRankColor(rank: number | null, palette: ThemePalette): string {
  if (rank === 1) return "#EAB308";
  if (rank === 2) return "#94A3B8";
  if (rank === 3) return "#D97706";
  return palette.accentSoft;
}

function metricCard(args: {
  x: number;
  y: number;
  width: number;
  height: number;
  label: string;
  value: string;
  valueFill: string;
  palette: ThemePalette;
  compact: boolean;
}): string {
  const { x, y, width, height, label, value, valueFill, palette, compact } = args;
  const labelY = y + 24;
  const valueY = compact ? y + 50 : y + 56;
  const valueSize = compact ? 24 : 28;

  return [
    `<rect x="${x}" y="${y}" width="${width}" height="${height}" rx="16" fill="${palette.surfaceAlt}" stroke="${palette.borderSoft}"/>`,
    `<rect x="${x + 1}" y="${y + 1}" width="${width - 2}" height="${Math.max(16, height - 2)}" rx="15" fill="url(#metric-sheen)" opacity="0.9"/>`,
    `<text x="${x + 18}" y="${labelY}" fill="${palette.muted}" font-size="12" font-weight="600" font-family="${FIGTREE_FONT_STACK}" letter-spacing="0.02em">${escapeXml(label)}</text>`,
    `<text x="${x + 18}" y="${valueY}" fill="${valueFill}" font-size="${valueSize}" font-weight="800" font-family="${FIGTREE_FONT_STACK}">${escapeXml(value)}</text>`,
  ].join("");
}

function renderProfileCardSvg(data: UserEmbedStats, options: RenderProfileEmbedOptions = {}): string {
  const theme: EmbedTheme = options.theme === "light" ? "light" : "dark";
  const compact = options.compact ?? false;
  const compactNumbers = options.compactNumbers ?? false;
  const sortBy: EmbedSortBy = options.sortBy === "cost" ? "cost" : "tokens";
  const palette = THEMES[theme];

  const width = compact ? 460 : 680;
  const height = compact ? 162 : 186;
  const rx = 18;
  const paddingX = compact ? 18 : 20;
  const headerY = compact ? 18 : 20;
  const headerHeight = compact ? 58 : 68;
  const metricsY = headerY + headerHeight + 12;
  const footerY = height - 16;
  const eyebrowY = headerY + (compact ? 18 : 24);
  const titleY = headerY + (compact ? 38 : 46);
  const userY = headerY + (compact ? 54 : 64);

  const username = `@${data.user.username}`;
  const displayNameRaw = data.user.displayName;
  const displayName = displayNameRaw ? escapeXml(displayNameRaw) : null;
  const tokens = formatNumber(data.stats.totalTokens, compactNumbers);
  const cost = formatCurrency(data.stats.totalCost, compactNumbers);
  const rank = data.stats.rank ? `#${data.stats.rank}` : "N/A";
  const updated = escapeXml(formatDateLabel(data.stats.updatedAt));
  const rankLabel = compact ? `Rank · ${sortBy === "cost" ? "Cost" : "Tokens"}` : `Rank (${sortBy === "cost" ? "Cost" : "Tokens"})`;

  const usernameEstimatedWidth = username.length * APPROX_CHAR_WIDTH_15_SEMIBOLD;
  const displayNameX = paddingX + 18 + usernameEstimatedWidth + 8;
  const badgeWidth = compact ? 92 : 110;
  const badgeX = width - paddingX - badgeWidth;
  const displayNameEstimatedWidth = displayNameRaw ? displayNameRaw.length * APPROX_CHAR_WIDTH_13 : 0;
  const showDisplayName = Boolean(displayName) && displayNameX + displayNameEstimatedWidth < badgeX - 12;

  const metricsGap = 12;
  const metricsWidth = width - paddingX * 2;
  const metricWidth = (metricsWidth - metricsGap * 2) / 3;
  const metricHeight = compact ? 56 : 64;
  const rankColor = getRankColor(data.stats.rank, palette);

  return `<?xml version="1.0" encoding="UTF-8"?>
<svg width="${width}" height="${height}" viewBox="0 0 ${width} ${height}" fill="none" xmlns="http://www.w3.org/2000/svg" role="img" aria-label="Tokscale profile stats for ${escapeXml(username)}">
  <defs>
    <style>@import url('${FIGTREE_FONT_IMPORT}');</style>
    <linearGradient id="card-bg" x1="24" y1="0" x2="${width}" y2="${height}" gradientUnits="userSpaceOnUse">
      <stop offset="0" stop-color="${palette.background}"/>
      <stop offset="1" stop-color="${palette.backgroundEnd}"/>
    </linearGradient>
    <linearGradient id="shell-bg" x1="${width / 2}" y1="0" x2="${width / 2}" y2="${height}" gradientUnits="userSpaceOnUse">
      <stop offset="0" stop-color="${palette.shell}"/>
      <stop offset="1" stop-color="${palette.shellEnd}"/>
    </linearGradient>
    <linearGradient id="header-bg" x1="${paddingX}" y1="${headerY}" x2="${width - paddingX}" y2="${headerY + headerHeight}" gradientUnits="userSpaceOnUse">
      <stop offset="0" stop-color="${palette.surface}"/>
      <stop offset="1" stop-color="${palette.surfaceAlt}"/>
    </linearGradient>
    <linearGradient id="metric-sheen" x1="${paddingX}" y1="${metricsY}" x2="${width - paddingX}" y2="${metricsY + metricHeight}" gradientUnits="userSpaceOnUse">
      <stop offset="0" stop-color="${palette.highlight}"/>
      <stop offset="1" stop-color="rgba(255,255,255,0)"/>
    </linearGradient>
    <linearGradient id="badge-bg" x1="${badgeX}" y1="${headerY + 14}" x2="${badgeX + badgeWidth}" y2="${headerY + 42}" gradientUnits="userSpaceOnUse">
      <stop offset="0" stop-color="${palette.surfaceAlt}"/>
      <stop offset="1" stop-color="${palette.accentGlow}"/>
    </linearGradient>
    <linearGradient id="tokens-gradient" x1="${paddingX}" y1="${metricsY}" x2="${paddingX + metricWidth}" y2="${metricsY + metricHeight}" gradientUnits="userSpaceOnUse">
      <stop offset="0" stop-color="${palette.tokenGradientStart}"/>
      <stop offset="0.5" stop-color="${palette.tokenGradientMid}"/>
      <stop offset="1" stop-color="${palette.tokenGradientEnd}"/>
    </linearGradient>
    <linearGradient id="rail-gradient" x1="0" y1="0" x2="${width}" y2="0" gradientUnits="userSpaceOnUse">
      <stop offset="0" stop-color="rgba(0,0,0,0)"/>
      <stop offset="0.3" stop-color="${palette.accentSoft}"/>
      <stop offset="1" stop-color="rgba(0,0,0,0)"/>
    </linearGradient>
    <radialGradient id="glow-orb" cx="0" cy="0" r="1" gradientUnits="userSpaceOnUse" gradientTransform="translate(${width - 96} 26) rotate(135) scale(180 120)">
      <stop offset="0" stop-color="${palette.accentSoft}" stop-opacity="0.18"/>
      <stop offset="1" stop-color="${palette.accentSoft}" stop-opacity="0"/>
    </radialGradient>
    <clipPath id="card-clip">
      <rect width="${width}" height="${height}" rx="${rx}"/>
    </clipPath>
    <filter id="soft-glow" x="-50%" y="-50%" width="200%" height="200%" color-interpolation-filters="sRGB">
      <feGaussianBlur stdDeviation="20"/>
    </filter>
  </defs>
  <rect width="${width}" height="${height}" rx="${rx}" fill="url(#card-bg)"/>
  <rect x="1" y="1" width="${width - 2}" height="${height - 2}" rx="${rx - 1}" fill="url(#shell-bg)" stroke="${palette.border}"/>
  <rect x="${paddingX}" y="${headerY}" width="${width - paddingX * 2}" height="${headerHeight}" rx="20" fill="url(#header-bg)" stroke="${palette.borderSoft}"/>
  <rect x="${paddingX}" y="${headerY + headerHeight + 4}" width="${width - paddingX * 2}" height="1" fill="url(#rail-gradient)" opacity="0.8"/>
  <circle cx="${width - 74}" cy="26" r="54" fill="${palette.accentGlow}" filter="url(#soft-glow)" opacity="0.7"/>
  <rect width="${width}" height="${height}" rx="${rx}" fill="url(#glow-orb)" clip-path="url(#card-clip)"/>
  <text x="${paddingX + 18}" y="${eyebrowY}" fill="${palette.accentSoft}" font-size="11" font-weight="700" font-family="${FIGTREE_FONT_STACK}" letter-spacing="0.08em">README EMBED</text>
  <text x="${paddingX + 18}" y="${titleY}" fill="${palette.title}" font-size="${compact ? 18 : 20}" font-weight="800" font-family="${FIGTREE_FONT_STACK}">Tokscale Stats</text>
  <text x="${paddingX + 18}" y="${userY}" fill="${palette.text}" font-size="15" font-weight="600" font-family="${FIGTREE_FONT_STACK}">${escapeXml(username)}</text>
  ${
    showDisplayName
      ? `<text x="${displayNameX}" y="${userY}" fill="${palette.muted}" font-size="13" font-family="${FIGTREE_FONT_STACK}">${displayName}</text>`
      : ""
  }
  <rect x="${badgeX}" y="${headerY + 14}" width="${badgeWidth}" height="28" rx="14" fill="url(#badge-bg)" stroke="${rankColor}" stroke-opacity="0.35"/>
  <text x="${badgeX + badgeWidth / 2}" y="${headerY + 32}" fill="${palette.title}" font-size="12" font-weight="700" font-family="${FIGTREE_FONT_STACK}" text-anchor="middle">${sortBy === "cost" ? "RANK · COST" : "RANK · TOKENS"}</text>
  ${metricCard({
    x: paddingX,
    y: metricsY,
    width: metricWidth,
    height: metricHeight,
    label: "Tokens",
    value: tokens,
    valueFill: "url(#tokens-gradient)",
    palette,
    compact,
  })}
  ${metricCard({
    x: paddingX + metricWidth + metricsGap,
    y: metricsY,
    width: metricWidth,
    height: metricHeight,
    label: "Cost",
    value: cost,
    valueFill: palette.costAccent,
    palette,
    compact,
  })}
  ${metricCard({
    x: paddingX + metricWidth * 2 + metricsGap * 2,
    y: metricsY,
    width: metricWidth,
    height: metricHeight,
    label: rankLabel,
    value: rank,
    valueFill: rankColor,
    palette,
    compact,
  })}
  <text x="${paddingX}" y="${footerY}" fill="${palette.muted}" font-size="11" font-family="${FIGTREE_FONT_STACK}">${updated}</text>
  <text x="${width - paddingX}" y="${footerY}" fill="${palette.muted}" font-size="11" font-family="${FIGTREE_FONT_STACK}" text-anchor="end">tokscale.ai/u/${escapeXml(
    data.user.username
  )}</text>
</svg>`;
}

export function renderProfileEmbedSvg(
  data: UserEmbedStats,
  options: RenderProfileEmbedOptions = {}
): string {
  return renderProfileCardSvg(data, options);
}

export function renderProfileEmbedErrorSvg(
  message: string,
  options: RenderProfileEmbedOptions = {}
): string {
  const theme: EmbedTheme = options.theme === "light" ? "light" : "dark";
  const palette = THEMES[theme];
  const safeMessage = escapeXml(message);

  return `<?xml version="1.0" encoding="UTF-8"?>
<svg width="540" height="120" viewBox="0 0 540 120" fill="none" xmlns="http://www.w3.org/2000/svg" role="img" aria-label="Tokscale embed error">
  <defs>
    <style>@import url('${FIGTREE_FONT_IMPORT}');</style>
    <linearGradient id="error-bg" x1="0" y1="0" x2="540" y2="120" gradientUnits="userSpaceOnUse">
      <stop offset="0" stop-color="${palette.background}"/>
      <stop offset="1" stop-color="${palette.backgroundEnd}"/>
    </linearGradient>
    <linearGradient id="error-shell" x1="24" y1="18" x2="516" y2="102" gradientUnits="userSpaceOnUse">
      <stop offset="0" stop-color="${palette.surface}"/>
      <stop offset="1" stop-color="${palette.surfaceAlt}"/>
    </linearGradient>
    <linearGradient id="error-rail" x1="20" y1="0" x2="520" y2="0" gradientUnits="userSpaceOnUse">
      <stop offset="0" stop-color="rgba(0,0,0,0)"/>
      <stop offset="0.45" stop-color="${palette.accentSoft}"/>
      <stop offset="1" stop-color="rgba(0,0,0,0)"/>
    </linearGradient>
  </defs>
  <rect width="540" height="120" rx="18" fill="url(#error-bg)"/>
  <rect x="1" y="1" width="538" height="118" rx="17" fill="${palette.shell}" stroke="${palette.border}"/>
  <rect x="18" y="16" width="504" height="88" rx="18" fill="url(#error-shell)" stroke="${palette.borderSoft}"/>
  <rect x="18" y="46" width="504" height="1" fill="url(#error-rail)" opacity="0.9"/>
  <text x="36" y="37" fill="${palette.accentSoft}" font-size="11" font-weight="700" font-family="${FIGTREE_FONT_STACK}" letter-spacing="0.08em">README EMBED</text>
  <text x="36" y="68" fill="${palette.title}" font-size="19" font-weight="800" font-family="${FIGTREE_FONT_STACK}">Tokscale Stats</text>
  <text x="36" y="88" fill="${palette.text}" font-size="13" font-family="${FIGTREE_FONT_STACK}">${safeMessage}</text>
  <text x="36" y="104" fill="${palette.muted}" font-size="11" font-family="${FIGTREE_FONT_STACK}">Try checking the username or submitting usage first.</text>
</svg>`;
}
