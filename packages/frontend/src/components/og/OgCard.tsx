/**
 * Shared building blocks for the Open Graph cards rendered by
 * `opengraph-image.tsx` routes.
 *
 * These render through Satori, not the browser, which constrains them:
 * - flexbox only, no grid
 * - no CSS variables, so the brand tokens are duplicated here as literals
 * - every element with more than one child needs an explicit `display`
 *
 * That last rule is the one that bites: `@{name}` compiles to two text
 * children and throws. Interpolate into a single template literal instead.
 */

import { OG_FONT_FAMILY } from "@/lib/og/fonts";
import {
  TOKSCALE_WORDMARK_ASPECT,
  TOKSCALE_WORDMARK_DATA_URI,
} from "@/lib/og/logo";

export const OG_SIZE = { width: 1200, height: 630 } as const;

export const OG_CANVAS = "#0d1018";
export const OG_SURFACE = "#131822";
export const OG_BORDER = "rgba(255, 255, 255, 0.09)";
export const OG_TEXT = "#f4f7fb";
export const OG_TEXT_MUTED = "#a8b3c5";

export function OgStat({ label, value }: { label: string; value: string }) {
  return (
    <div
      style={{
        display: "flex",
        flexDirection: "column",
        flex: 1,
        padding: "28px 32px",
        border: `1px solid ${OG_BORDER}`,
        borderRadius: 20,
        background: OG_SURFACE,
      }}
    >
      <div style={{ fontSize: 56, fontWeight: 700, color: OG_TEXT, lineHeight: 1.1 }}>
        {value}
      </div>
      <div
        style={{
          marginTop: 10,
          fontSize: 22,
          color: OG_TEXT_MUTED,
          letterSpacing: 2,
        }}
      >
        {label}
      </div>
    </div>
  );
}

/**
 * Canvas, brand rule, and the tokscale.ai watermark. `children` fills the
 * space between them; pass a flex-grow spacer to bottom-align a stats row.
 */
const WORDMARK_WIDTH = 220;

export function OgCardShell({ children }: { children: React.ReactNode }) {
  return (
    <div
      style={{
        width: "100%",
        height: "100%",
        display: "flex",
        flexDirection: "column",
        background: OG_CANVAS,
        padding: 72,
        // Satori resolves this stack per glyph, so Latin renders in Manrope and
        // any CJK falls through to Pretendard. Both are registered by
        // loadOgFonts(); if neither loaded, Satori uses its built-in face.
        fontFamily: OG_FONT_FAMILY,
      }}
    >
      {/* eslint-disable-next-line @next/next/no-img-element --
          this tree is rendered by Satori into a PNG, never by the browser;
          next/image has no meaning here and would not render. */}
      <img
        src={TOKSCALE_WORDMARK_DATA_URI}
        width={WORDMARK_WIDTH}
        height={Math.round(WORDMARK_WIDTH / TOKSCALE_WORDMARK_ASPECT)}
        alt=""
      />

      {children}

      <div
        style={{
          display: "flex",
          justifyContent: "flex-end",
          marginTop: 32,
          fontSize: 26,
          color: OG_TEXT_MUTED,
        }}
      >
        tokscale.ai
      </div>
    </div>
  );
}
