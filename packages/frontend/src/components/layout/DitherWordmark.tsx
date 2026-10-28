"use client";

/**
 * DitherWordmark — the footer's "mark, built large".
 *
 * The shipped logo (131×25 viewBox: the three-blade mark in the brand blue,
 * the Tokscale wordmark in ink) is rasterized to a grid of square cells that
 * spans the full content width. Each cell is lit when its coverage, dithered
 * against the kit's Bayer matrix under a density that firms toward the
 * baseline, clears the threshold: sparse cells at the top, solid at the foot,
 * so the site's idea (a count becoming a field of dithered pixels) is carried
 * by the name itself. Painted once per width change; nothing moves.
 *
 * The box holds the logo's aspect ratio before the first paint, so the footer
 * never shifts; the canvas is drawn at the grid's own resolution and scaled
 * up with `pixelated` rendering, which keeps every cell square.
 */
import { useEffect, useRef } from "react";
import { BAYER4 } from "dither-kit/pixel";
import { TOKSCALE_WORDMARK_PATH } from "@/components/layout/TokscaleMark";

const LOGO_WIDTH = 131;
const LOGO_HEIGHT = 25;
const MARK_PATH =
  "M10.6965 7.31864V24.7708L14.6373 11.2594 10.6965 7.31864ZM10.6965 7.31864H28.1486L14.6373 11.2594 10.6965 7.31864ZM10.6965 3.37783V7.31864L3.37783 6.1927 0 0 10.6965 3.37783Z";
const MIN_COLS = 96;
const MAX_COLS = 240;
/** css px per cell at the widest size, before the fit is clamped. */
const TARGET_CELL = 7;

type Rgb = [number, number, number];

function tokenColor(variable: string, fallback: Rgb): Rgb {
  if (typeof window === "undefined") return fallback;
  const value = getComputedStyle(document.body).getPropertyValue(variable).trim();
  if (!/^#([0-9a-f]{3}|[0-9a-f]{6})$/i.test(value)) return fallback;
  const hex =
    value.length === 4 ? `#${value[1]}${value[1]}${value[2]}${value[2]}${value[3]}${value[3]}` : value;
  return [
    Number.parseInt(hex.slice(1, 3), 16),
    Number.parseInt(hex.slice(3, 5), 16),
    Number.parseInt(hex.slice(5, 7), 16),
  ];
}

export function DitherWordmark({ className }: { className?: string }) {
  const boxRef = useRef<HTMLDivElement>(null);
  const canvasRef = useRef<HTMLCanvasElement>(null);

  useEffect(() => {
    const box = boxRef.current;
    const canvas = canvasRef.current;
    if (!box || !canvas) return;
    const ctx = canvas.getContext("2d", { willReadFrequently: true });
    if (!ctx) return;

    let lastCols = 0;

    const paint = () => {
      const width = box.clientWidth;
      if (width <= 0) return;
      const cols = Math.min(MAX_COLS, Math.max(MIN_COLS, Math.round(width / TARGET_CELL)));
      if (cols === lastCols) return;
      lastCols = cols;
      const rows = Math.max(2, Math.round((cols * LOGO_HEIGHT) / LOGO_WIDTH));

      // Rasterize the logo at grid resolution × 6 supersampling and read the
      // coverage and the colour of each cell back.
      const ss = 6;
      const off = document.createElement("canvas");
      off.width = cols * ss;
      off.height = rows * ss;
      const octx = off.getContext("2d", { willReadFrequently: true });
      if (!octx) return;
      octx.fillStyle = "#000";
      octx.fillRect(0, 0, off.width, off.height);
      octx.scale(off.width / LOGO_WIDTH, off.height / LOGO_HEIGHT);
      // Each part owns one channel, so a cell's coverage of the mark and of
      // the wordmark can be told apart from the raster alone.
      octx.fillStyle = "rgb(255,0,0)";
      octx.fill(new Path2D(MARK_PATH));
      octx.fillStyle = "rgb(0,255,0)";
      octx.fill(new Path2D(TOKSCALE_WORDMARK_PATH));
      const data = octx.getImageData(0, 0, off.width, off.height).data;

      const accent = tokenColor("--v2-accent", [0, 115, 255]);
      const ink = tokenColor("--v2-text", [244, 247, 251]);

      canvas.width = cols;
      canvas.height = rows;
      ctx.clearRect(0, 0, cols, rows);
      // On a phone the cells are only ~4px, so the ramp keeps a high floor
      // and the letters stay readable; the wide version shows the dither.
      const floor = width < 700 ? 0.9 : 0.62;
      for (let row = 0; row < rows; row++) {
        // Density firms toward the baseline: the wordmark fills as it falls.
        const density = floor + (1 - floor) * (row / Math.max(1, rows - 1));
        for (let col = 0; col < cols; col++) {
          let hits = 0;
          let markHits = 0;
          let inkHits = 0;
          const base = (row * ss * off.width + col * ss) * 4;
          for (let y = 0; y < ss; y++) {
            for (let x = 0; x < ss; x++) {
              const i = base + (y * off.width + x) * 4;
              const mark = data[i] > 64;
              const ink = data[i + 1] > 64;
              if (mark || ink) hits++;
              if (mark) markHits++;
              if (ink) inkHits++;
            }
          }
          const coverage = hits / (ss * ss);
          if (coverage <= 0) continue;
          if (coverage * density <= BAYER4[row & 3][col & 3]) continue;
          const fill = inkHits > markHits ? ink : accent;
          ctx.fillStyle = `rgb(${fill[0]},${fill[1]},${fill[2]})`;
          ctx.fillRect(col, row, 1, 1);
        }
      }
    };

    paint();
    const observer = typeof ResizeObserver !== "undefined" ? new ResizeObserver(paint) : null;
    observer?.observe(box);
    return () => observer?.disconnect();
  }, []);

  return (
    <div ref={boxRef} className={className} aria-hidden="true">
      <canvas ref={canvasRef} style={{ display: "block", width: "100%", height: "100%", imageRendering: "pixelated" }} />
    </div>
  );
}
