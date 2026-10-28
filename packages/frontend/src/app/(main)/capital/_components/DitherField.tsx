"use client";

/**
 * Hero-sized level field for the year's equity figure.
 *
 * The data is a height, not a texture: the cells fill from the bottom up in full
 * ink to `share` of the block, the rest stays empty ground, and only a three-cell
 * band at the boundary is ordered-dithered — so "the field fills 79% of $300B"
 * and the picture are the same statement. The dither comes from dither-kit's own
 * 4×4 Bayer matrix and palette (`BAYER4`, `fillOf` from `dither-kit/pixel`, the
 * engine under `DitherViz`), and the crawl slides that matrix along the boundary
 * so the dissolving edge moves without moving the level.
 *
 * It pauses off screen and in a hidden tab; a reader who asked for less motion
 * gets the still field.
 */
import { useEffect, useRef } from "react";
import { BAYER4, fillOf } from "dither-kit/pixel";
import { rgb } from "dither-kit/palette";

/** css px per dither cell. */
const CELL = 7;
/** Cells of dissolve at the level: full fill below, empty ground above. */
const BAND = 3;
const STEP_MS = 2_000;

export interface DitherFieldProps {
  /** Filled share of the block, 0–1. The figure and the ceiling are real text beside it. */
  share: number;
}

const PAINT = rgb(fillOf("blue"), 1);

export function DitherField({ share }: DitherFieldProps) {
  const rootRef = useRef<HTMLDivElement | null>(null);
  const canvasRef = useRef<HTMLCanvasElement | null>(null);

  useEffect(() => {
    const root = rootRef.current;
    const canvas = canvasRef.current;
    if (!root || !canvas) return;

    const level = Number.isFinite(share) ? Math.max(0, Math.min(1, share)) : 0;
    const reduce = typeof matchMedia === "function" && matchMedia("(prefers-reduced-motion: reduce)").matches;
    let cols = 0;
    let rows = 0;
    let step = 0;
    let timer = 0;
    let visible = true;

    const paint = (offset: number) => {
      const ctx = canvas.getContext("2d", { willReadFrequently: true });
      if (!ctx || cols <= 0 || rows <= 0) return;
      const shift = offset & 3;
      const filled = rows * level; // cells of fill, counted up from the baseline
      ctx.clearRect(0, 0, cols, rows);
      ctx.fillStyle = PAINT;
      for (let y = 0; y < rows; y++) {
        const above = rows - y; // 1 at the bottom row
        const fromLevel = filled - above; // ≥ BAND is full, ≤ −BAND is empty
        if (fromLevel <= -BAND) continue;
        const coverage = fromLevel >= BAND ? 1 : (fromLevel + BAND) / (2 * BAND);
        if (coverage >= 1) {
          ctx.fillRect(0, y, cols, 1);
          continue;
        }
        if (coverage <= 0) continue;
        for (let x = 0; x < cols; x++) {
          if (coverage > BAYER4[y & 3][(x + shift) & 3]) ctx.fillRect(x, y, 1, 1);
        }
      }
    };

    const size = () => {
      const box = root.getBoundingClientRect();
      if (box.width < 8 || box.height < 8) return;
      cols = Math.max(8, Math.round(box.width / CELL));
      rows = Math.max(4, Math.round(box.height / CELL));
      canvas.width = cols;
      canvas.height = rows;
      paint(step);
    };

    const stop = () => {
      if (timer) window.clearInterval(timer);
      timer = 0;
    };

    const start = () => {
      if (timer || reduce) return;
      timer = window.setInterval(() => {
        step = (step + 1) % 4;
        paint(step);
      }, STEP_MS);
    };

    size();
    const resize = typeof ResizeObserver !== "undefined" ? new ResizeObserver(size) : null;
    resize?.observe(root);
    if (!reduce) {
      start();
      const io =
        typeof IntersectionObserver !== "undefined"
          ? new IntersectionObserver((entries) => {
              visible = entries.some((entry) => entry.isIntersecting);
              if (visible && !document.hidden) start();
              else stop();
            })
          : null;
      io?.observe(root);
      const onVisibility = () => (document.hidden ? stop() : visible && start());
      document.addEventListener("visibilitychange", onVisibility);
      return () => {
        stop();
        io?.disconnect();
        resize?.disconnect();
        document.removeEventListener("visibilitychange", onVisibility);
      };
    }
    return () => resize?.disconnect();
  }, [share]);

  return (
    <div ref={rootRef} aria-hidden="true" role="presentation" style={{ position: "relative", width: "100%", height: "100%" }}>
      <canvas ref={canvasRef} aria-hidden="true" />
    </div>
  );
}
