"use client";

/**
 * DitherUnderline — the site's one selection mark.
 *
 * A strip of dither cells (2px each, the kit's CELL) as wide as the label it
 * marks, painted once per width change: density ramps from sparse at the left
 * to solid at the right (the same ramp DitherTabs paints under its active
 * tab), so the underline reads as dither cells up close and as an accent line
 * at a glance. Nothing renders on the server; the label keeps its place.
 */
import { useEffect, useRef } from "react";
import { BAYER4, fillOf } from "dither-kit/pixel";

const CELL = 2;

export function DitherUnderline({ className }: { className?: string }) {
  const ref = useRef<HTMLSpanElement>(null);
  const canvasRef = useRef<HTMLCanvasElement>(null);

  useEffect(() => {
    const span = ref.current;
    const canvas = canvasRef.current;
    if (!span || !canvas) return;
    const ctx = canvas.getContext("2d", { willReadFrequently: true });
    if (!ctx) return;

    const paint = () => {
      const length = span.offsetWidth;
      if (length <= 0) return;
      const cells = Math.max(4, Math.round(length / CELL));
      canvas.width = cells;
      canvas.height = 1;
      const accent = getComputedStyle(span).getPropertyValue("--v2-accent").trim() || "#0073ff";
      const fill = fillOf(accent.startsWith("#") ? accent : "#0073ff");
      ctx.clearRect(0, 0, cells, 1);
      for (let i = 0; i < cells; i++) {
        const density = (i + 0.5) / cells;
        const lit = density > BAYER4[0][i & 3];
        const alpha = lit ? 0.35 + 0.65 * density : 0.12 * density;
        if (alpha <= 0.004) continue;
        ctx.fillStyle = `rgba(${fill[0]},${fill[1]},${fill[2]},${alpha})`;
        ctx.fillRect(i, 0, 1, 1);
      }
      canvas.style.width = `${cells * CELL}px`;
      canvas.style.height = `${CELL}px`;
    };

    paint();
    const observer = typeof ResizeObserver !== "undefined" ? new ResizeObserver(paint) : null;
    observer?.observe(span);
    return () => observer?.disconnect();
  }, []);

  return (
    <span ref={ref} aria-hidden="true" className={className}>
      <canvas ref={canvasRef} style={{ display: "block", imageRendering: "pixelated" }} />
    </span>
  );
}
