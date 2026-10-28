"use client";

/**
 * Shared frame for dither-kit cartesian charts on the explore screens.
 * The kit's defaults are a docs-site look (10px mono axis text, entrance
 * animation, sparkles); the app-screen rules want 12px ticks in the UI face,
 * 3–5 faint gridlines, no entrance animation and no decorative motion.
 * Charts sit in an explicitly sized box (the canvas needs it) and keep all
 * labels and values in real DOM text around them.
 */
import styled, { css } from "styled-components";

/** Sizing box: give every chart an explicit height from its container. */
export const ChartBox = styled.div<{ $height: number }>`
  position: relative;
  width: 100%;
  height: ${({ $height }) => $height}px;
`;

/**
 * Kit chrome overrides. The kit's axis <g> carries its monospace utility and
 * text-[10px]; globals.css already takes the kit's labels back to the text
 * face, and these rules lift the tick size over the utility class.
 * Apply it on an element INSIDE a ChartBox: it fills that box (height 100%)
 * so the kit root, which is h-full, gets a real height. Wrapped around an
 * auto-height parent instead, set `height: auto` on the wrapper.
 */
export const chartFrame = css`
  position: relative;
  width: 100%;
  height: 100%;

  svg text {
    fill: var(--service-text-muted);
    font-size: 12px;
  }

  svg .stroke-border {
    stroke: var(--service-border);
  }

  /* The kit paints its labels in its monospace utility; globals.css already
     takes them back to the text face, so only the size is lifted here. */
  .dk-tooltip-card {
    border-radius: 2px;
    font-size: 13px;

    /* The tooltip's rows carry their own text-[11px] utility class. */
    div {
      font-size: 13px;
    }
  }
`;

/** App-screen chart defaults: still, honest, no decorative motion. */
export const CHART_PROPS = {
  animate: false,
  sparkles: false,
  hoverLift: false,
  bloom: "off",
  crosshair: true,
} as const;

/** Compact tick formatter for counts (1.2k, 34k, 1.1M, 20B, 1.4T). */
export function compactCount(value: number): string {
  const scaled = (divisor: number, suffix: string) => `${(value / divisor).toFixed(1).replace(/\.0$/, "")}${suffix}`;
  if (value >= 1_000_000_000_000) return scaled(1_000_000_000_000, "T");
  if (value >= 1_000_000_000) return scaled(1_000_000_000, "B");
  if (value >= 1_000_000) return scaled(1_000_000, "M");
  if (value >= 1_000) return scaled(1_000, "k");
  return String(value);
}

