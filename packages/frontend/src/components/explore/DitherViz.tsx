"use client";

/**
 * Horizontal magnitude bar painted with dither-kit's ordered-dither engine
 * (`dither-paint`, `pixel`): the same Bayer scatter, texture presets and
 * palette as the kit's charts, so every bar on the site reads as the same
 * pixel. The fill is solid at its base and dissolves toward the value edge,
 * exactly like a kit chart column laid on its side. The value itself always
 * lives in real text beside the bar; the canvas is decorative.
 */
import { useEffect, useRef } from "react";
import styled from "styled-components";
import { resolveMatrix, resolveTexture, type VariantInput } from "dither-kit/dither-paint";
import { fillOf, type PixelColor } from "dither-kit/pixel";

/** css px per dither cell — the kit's chunky pixel. */
const CELL = 2;

export interface DitherBarProps {
  /** Fill ratio of the track, 0–1. A zero draws no stub. */
  value: number;
  /** Kit colour name, hue or hex. Blue is the site's one data hue. */
  color?: PixelColor;
  /** Kit texture preset ("gradient" | "dotted" | "hatched" | "solid") or config. */
  variant?: VariantInput;
  /** Draw the faint grey dithered track behind the fill (the 100% baseline). */
  track?: boolean;
  /** Track height in css px (a multiple of 2 keeps the cells square). */
  height?: number;
  className?: string;
}

const Root = styled.span<{ $height: number }>`
  position: relative;
  display: block;
  width: 100%;
  height: ${({ $height }) => $height}px;
  overflow: hidden;
  border-radius: 2px;
`;

function paintBar(
  ctx: CanvasRenderingContext2D,
  cols: number,
  rows: number,
  fill: [number, number, number],
  muted: [number, number, number],
  ratio: number,
  variant: VariantInput,
  track: boolean,
): void {
  const tex = resolveTexture(variant);
  const mat = resolveMatrix(variant);
  const filled = ratio >= 1 ? cols : Math.round(cols * Math.max(0, Math.min(1, ratio)));
  const rgba = (c: [number, number, number], a: number) =>
    `rgba(${c[0]},${c[1]},${c[2]},${a < 0 ? 0 : a > 1 ? 1 : a})`;
  ctx.clearRect(0, 0, cols, rows);
  for (let y = 0; y < rows; y++) {
    for (let x = 0; x < cols; x++) {
      if (x < filled) {
        // Thin at the base, dense at the value edge: more tokens, more dither.
        const raw = filled > 1 ? (x + 0.5) / filled : 1;
        const density = 1 - tex.ramp * (1 - raw);
        const lit = density + tex.density > mat[y & 3][x & 3];
        const k = tex.alphaFloor + density * tex.alphaRange;
        const alpha = lit ? k : k * tex.offTier;
        ctx.fillStyle = rgba(fill, alpha);
        ctx.fillRect(x, y, 1, 1);
      } else if (track) {
        const lit = 0.25 > mat[y & 3][x & 3];
        ctx.fillStyle = rgba(muted, lit ? 0.2 : 0.06);
        ctx.fillRect(x, y, 1, 1);
      }
    }
  }
  // The value edge: a soft crisp line in the fill colour, like a chart's top border.
  if (filled > 0 && filled < cols) {
    ctx.fillStyle = rgba(fill, tex.edge);
    ctx.fillRect(filled, 0, 1, rows);
  }
}

/**
 * One dithered horizontal bar. Sized by its container's width (ResizeObserver)
 * and its `height` prop; the backing canvas is one pixel per dither cell and
 * scaled up `pixelated`. Nothing animates, so reduced motion needs no case.
 */
export function DitherBar({
  value,
  color = "blue",
  variant = "gradient",
  track = true,
  height = 8,
  className,
}: DitherBarProps) {
  const rootRef = useRef<HTMLSpanElement | null>(null);
  const canvasRef = useRef<HTMLCanvasElement | null>(null);
  const colsRef = useRef(0);

  useEffect(() => {
    const root = rootRef.current;
    const canvas = canvasRef.current;
    if (!root || !canvas) return;
    const rows = Math.max(1, Math.round(height / CELL));
    const ratio = Number.isFinite(value) ? Math.max(0, Math.min(1, value)) : 0;

    const paint = () => {
      const ctx = canvas.getContext("2d", { willReadFrequently: true });
      if (!ctx || colsRef.current <= 0) return;
      paintBar(ctx, colsRef.current, rows, fillOf(color), fillOf("grey"), ratio, variant, track);
    };

    const resize = () => {
      colsRef.current = Math.max(4, Math.round(root.getBoundingClientRect().width / CELL));
      canvas.width = colsRef.current;
      canvas.height = rows;
      paint();
    };

    let ro: ResizeObserver | null = null;
    const raf = requestAnimationFrame(() => {
      resize();
      if (typeof ResizeObserver !== "undefined") {
        ro = new ResizeObserver(resize);
        ro.observe(root);
      }
    });
    return () => {
      cancelAnimationFrame(raf);
      ro?.disconnect();
    };
  }, [value, color, variant, track, height]);

  return (
    <Root ref={rootRef} $height={height} className={className} aria-hidden="true">
      <canvas
        ref={canvasRef}
        aria-hidden="true"
        style={{ position: "absolute", inset: 0, width: "100%", height: "100%", imageRendering: "pixelated" }}
      />
    </Root>
  );
}

const SwatchRoot = styled(DitherBar)`
  display: inline-block;
  width: 14px;
  flex: 0 0 auto;
`;

/**
 * Legend swatch that IS the mark it explains: a short slice of the same
 * dithered bar, same colour and texture, scaled down.
 */
export function DitherSwatch({
  color = "blue",
  variant = "gradient",
  height = 14,
}: {
  color?: PixelColor;
  variant?: VariantInput;
  height?: number;
}) {
  return <SwatchRoot value={1} color={color} variant={variant} track={false} height={height} />;
}

export interface DitherPart {
  color?: PixelColor;
  variant?: VariantInput;
  /** Share of the whole, 0–1; parts are laid end to end in order. */
  share: number;
}

/**
 * One 100% stacked bar of dither parts (the token mix): each part is the same
 * ordered-dither fill in its own colour, separated by a one-cell seam so the
 * segments read apart without a rule. Shares are normalised to the parts shown.
 */
export function DitherStack({ parts, height = 14 }: { parts: readonly DitherPart[]; height?: number }) {
  const rootRef = useRef<HTMLSpanElement | null>(null);
  const canvasRef = useRef<HTMLCanvasElement | null>(null);

  useEffect(() => {
    const root = rootRef.current;
    const canvas = canvasRef.current;
    if (!root || !canvas) return;
    const rows = Math.max(1, Math.round(height / CELL));
    const total = parts.reduce((sum, part) => sum + Math.max(0, part.share), 0);

    const paint = () => {
      const ctx = canvas.getContext("2d", { willReadFrequently: true });
      if (!ctx) return;
      const cols = Math.max(8, Math.round(root.getBoundingClientRect().width / CELL));
      canvas.width = cols;
      canvas.height = rows;
      ctx.clearRect(0, 0, cols, rows);
      if (total <= 0) return;
      // Cell spans, largest-remainder so the parts always add up to the full width.
      const wanted = parts.map((part) => (Math.max(0, part.share) / total) * cols);
      const spans = wanted.map((w) => Math.floor(w));
      let left = cols - spans.reduce((sum, w) => sum + w, 0);
      const order = wanted.map((w, i) => ({ i, rest: w - spans[i] })).sort((a, b) => b.rest - a.rest);
      for (const { i } of order) {
        if (left <= 0) break;
        spans[i] += 1;
        left -= 1;
      }
      let pen = 0;
      parts.forEach((part, index) => {
        const tex = resolveTexture(part.variant ?? "gradient");
        const mat = resolveMatrix(part.variant ?? "gradient");
        const fill = fillOf(part.color ?? "blue");
        const width = spans[index];
        const seam = index < parts.length - 1 && width > 2 ? 1 : 0;
        for (let x = 0; x < width - seam; x++) {
          const raw = width > 1 ? (x + 0.5) / width : 1;
          const density = 1 - tex.ramp * (1 - raw);
          for (let y = 0; y < rows; y++) {
            const lit = density + tex.density > mat[y & 3][x & 3];
            const k = tex.alphaFloor + density * tex.alphaRange;
            const alpha = lit ? k : k * tex.offTier;
            ctx.fillStyle = `rgba(${fill[0]},${fill[1]},${fill[2]},${alpha})`;
            ctx.fillRect(pen + x, y, 1, 1);
          }
        }
        pen += width;
      });
    };

    let ro: ResizeObserver | null = null;
    const raf = requestAnimationFrame(() => {
      paint();
      if (typeof ResizeObserver !== "undefined") {
        ro = new ResizeObserver(paint);
        ro.observe(root);
      }
    });
    return () => {
      cancelAnimationFrame(raf);
      ro?.disconnect();
    };
  }, [parts, height]);

  return (
    <Root ref={rootRef} $height={height} aria-hidden="true">
      <canvas
        ref={canvasRef}
        aria-hidden="true"
        style={{ position: "absolute", inset: 0, width: "100%", height: "100%", imageRendering: "pixelated" }}
      />
    </Root>
  );
}
