"use client";

import {
  useCallback,
  useEffect,
  useMemo,
  useRef,
  useState,
  type ReactNode,
} from "react";
import Link from "next/link";
import styled from "styled-components";
import { BAYER4 } from "dither-kit/pixel";
import { hexToRgb, rgb, type Rgb } from "dither-kit/palette";

/**
 * Shared in-app controls built on the dither-kit engine exports.
 *
 * The kit ships its own selection pieces (`DitherTabs`, `DitherSegmented`,
 * `DitherToggleGroup`), but each draws its active mark as a wash or a chip,
 * and the dealt data-terminal style marks every selection with the same thin
 * accent line. The pieces here reuse the kit's exact underline recipe (from
 * `DitherTabs`' `paintUnderline`) and its pixel primitives (`BAYER4`, `rgb`)
 * so the mark reads as the kit's own texture.
 */

/** Matches `--service-accent` in globals.css; keep the two in sync. */
const ACCENT = "#2f8fff";
/** css px per dither cell, as in the kit (`CELL` in DitherTabs/DitherMeter). */
const DITHER_CELL = 2;

/**
 * jsdom has no 2D canvas implementation and logs loudly for every
 * getContext call. The context class is only defined where a real canvas
 * backend exists, so it doubles as the feature check.
 */
function canvasContext(
  canvas: HTMLCanvasElement,
): CanvasRenderingContext2D | null {
  if (typeof CanvasRenderingContext2D === "undefined") return null;
  return canvas.getContext("2d", { willReadFrequently: true });
}

/** The kit's tab-underline ramp: sparse at the start, dense at the end. */
function paintDitherRule(
  canvas: HTMLCanvasElement,
  length: number,
  color: string,
): void {
  const ctx = canvasContext(canvas);
  if (!ctx || length <= 0) return;
  const cells = Math.max(4, Math.round(length / DITHER_CELL));
  canvas.width = cells;
  canvas.height = 1;
  const fill = hexToRgb(color);
  ctx.clearRect(0, 0, cells, 1);
  for (let i = 0; i < cells; i += 1) {
    const density = (i + 0.5) / cells;
    const lit = density > BAYER4[0][i & 3];
    const alpha = lit ? 0.35 + 0.65 * density : 0.12 * density;
    if (alpha <= 0.004) continue;
    ctx.fillStyle = rgb(fill, 1, alpha);
    ctx.fillRect(i, 0, 1, 1);
  }
}

/** A dithered rule canvas that repositions under its active sibling. */
function SelectionRule({
  containerRef,
  activeRef,
  color,
}: {
  containerRef: React.RefObject<HTMLElement | null>;
  activeRef: React.RefObject<HTMLElement | null>;
  color: string;
}) {
  const canvasRef = useRef<HTMLCanvasElement>(null);
  const [box, setBox] = useState({ left: 0, top: 0, width: 0 });

  const measure = useCallback(() => {
    const active = activeRef.current;
    const canvas = canvasRef.current;
    if (!active || !canvas) return;
    setBox({
      left: active.offsetLeft,
      top: active.offsetTop + active.offsetHeight - 2,
      width: active.offsetWidth,
    });
    paintDitherRule(canvas, active.offsetWidth, color);
  }, [activeRef, color]);

  useEffect(() => {
    const raf = requestAnimationFrame(measure);
    let observer: ResizeObserver | null = null;
    if (typeof ResizeObserver !== "undefined" && containerRef.current) {
      observer = new ResizeObserver(measure);
      observer.observe(containerRef.current);
    }
    return () => {
      cancelAnimationFrame(raf);
      observer?.disconnect();
    };
  }, [measure, containerRef]);

  useEffect(() => {
    measure();
  }, [measure]);

  return (
    <RuleCanvas
      ref={canvasRef}
      aria-hidden="true"
      style={{
        left: `${box.left}px`,
        top: `${box.top}px`,
        width: `${box.width}px`,
      }}
    />
  );
}

const RuleCanvas = styled.canvas`
  position: absolute;
  height: 2px;
  image-rendering: pixelated;
  pointer-events: none;
  transition:
    left 160ms ease,
    width 160ms ease;

  @media (prefers-reduced-motion: reduce) {
    transition: none;
  }
`;

export interface DitherSelectionOption {
  value: string;
  label: string;
  title?: string;
}

export interface DitherSelectionGroupProps {
  options: readonly DitherSelectionOption[];
  value: string;
  onChange: (value: string) => void;
  /** Accessible name for the radio group. */
  label: string;
  className?: string;
}

/**
 * Mutually exclusive options marked by the data-terminal accent line: one
 * dithered rule exactly as wide as the active label. Radiogroup semantics
 * with a roving tabindex; Arrow/Home/End keys move the selection, matching
 * the kit's own controls.
 */
export function DitherSelectionGroup({
  options,
  value,
  onChange,
  label,
  className,
}: DitherSelectionGroupProps) {
  const containerRef = useRef<HTMLDivElement>(null);
  const itemRefs = useRef<Array<HTMLButtonElement | null>>([]);

  const enabled = useMemo(
    () => options.map((option, index) => ({ option, index })),
    [options],
  );
  const activeIndex = Math.max(
    0,
    options.findIndex((option) => option.value === value),
  );
  const activeRef = {
    get current() {
      return itemRefs.current[activeIndex] ?? null;
    },
  };

  const select = (index: number) => {
    const next = options[index];
    if (!next) return;
    onChange(next.value);
  };

  const handleKeyDown = (
    event: React.KeyboardEvent<HTMLButtonElement>,
    currentIndex: number,
  ) => {
    let nextIndex: number | null = null;
    switch (event.key) {
      case "ArrowRight":
      case "ArrowDown":
        nextIndex = (currentIndex + 1) % options.length;
        break;
      case "ArrowLeft":
      case "ArrowUp":
        nextIndex = (currentIndex - 1 + options.length) % options.length;
        break;
      case "Home":
        nextIndex = 0;
        break;
      case "End":
        nextIndex = options.length - 1;
        break;
      default:
        return;
    }
    event.preventDefault();
    select(nextIndex);
    requestAnimationFrame(() => itemRefs.current[nextIndex]?.focus());
  };

  return (
    <SelectionRow
      ref={containerRef}
      role="radiogroup"
      aria-label={label}
      className={className}
    >
      {enabled.map(({ option, index }) => (
        <SelectionButton
          key={option.value}
          ref={(node) => {
            itemRefs.current[index] = node;
          }}
          type="button"
          role="radio"
          title={option.title}
          aria-checked={option.value === value}
          tabIndex={option.value === value ? 0 : -1}
          $active={option.value === value}
          onClick={() => onChange(option.value)}
          onKeyDown={(event) => handleKeyDown(event, index)}
        >
          {option.label}
        </SelectionButton>
      ))}
      <SelectionRule
        containerRef={containerRef}
        activeRef={activeRef}
        color={ACCENT}
      />
    </SelectionRow>
  );
}

export interface DitherSelectionLinkOption extends DitherSelectionOption {
  href: string;
}

/**
 * Navigation links (period ranges) carrying the same accent-line selection:
 * real links keep URLs, history and server rendering authoritative.
 */
export function DitherSelectionNav({
  options,
  value,
  label,
  className,
}: {
  options: readonly DitherSelectionLinkOption[];
  value: string;
  label: string;
  className?: string;
}) {
  const containerRef = useRef<HTMLDivElement>(null);
  const itemRefs = useRef<Array<HTMLAnchorElement | null>>([]);
  const activeIndex = Math.max(
    0,
    options.findIndex((option) => option.value === value),
  );
  const activeRef = {
    get current() {
      return itemRefs.current[activeIndex] ?? null;
    },
  };

  return (
    <SelectionRow
      ref={containerRef}
      role="navigation"
      aria-label={label}
      className={className}
    >
      {options.map((option, index) => (
        <SelectionLink
          key={option.value}
          ref={(node) => {
            itemRefs.current[index] = node;
          }}
          href={option.href}
          title={option.title}
          aria-current={option.value === value ? "page" : undefined}
          $active={option.value === value}
        >
          {option.label}
        </SelectionLink>
      ))}
      <SelectionRule
        containerRef={containerRef}
        activeRef={activeRef}
        color={ACCENT}
      />
    </SelectionRow>
  );
}

const SelectionRow = styled.div`
  position: relative;
  display: inline-flex;
  align-items: center;
  gap: 18px;
  max-width: 100%;
  min-width: 0;
`;

const selectionText = `
  padding: 0 0 6px;
  color: var(--service-text-muted);
  font: inherit;
  font-size: 0.875rem;
  line-height: 1.2;
  white-space: nowrap;
  cursor: pointer;

  &:focus-visible {
    outline: 2px solid var(--service-focus);
    outline-offset: 2px;
  }
`;

const SelectionButton = styled.button<{ $active: boolean }>`
  ${selectionText}
  position: relative;
  z-index: 1;
  border: 0;
  background: transparent;
  color: ${(props) =>
    props.$active ? "var(--service-text)" : "var(--service-text-muted)"};

  &:hover {
    color: var(--service-text);
  }

  @media (pointer: coarse) {
    min-height: 44px;
    padding-top: 22px;
    margin-top: -22px;
  }
`;

const SelectionLink = styled(Link)<{ $active: boolean }>`
  ${selectionText}
  position: relative;
  z-index: 1;
  text-decoration: none;
  color: ${(props) =>
    props.$active ? "var(--service-text)" : "var(--service-text-muted)"};

  &:hover {
    color: var(--service-text);
  }

  @media (pointer: coarse) {
    min-height: 44px;
    padding-top: 22px;
    margin-top: -22px;
  }
`;

/** Paint a small square swatch whose dither density encodes `level` (0..max). */
export function paintDitherSwatch(
  canvas: HTMLCanvasElement,
  level: number,
  max: number,
  color: string,
  emptyColor: string,
): void {
  const ctx = canvasContext(canvas);
  if (!ctx) return;
  canvas.width = 4;
  canvas.height = 4;
  ctx.clearRect(0, 0, 4, 4);
  const density = level <= 0 ? 0 : level / max;
  const fill = hexToRgb(color);
  const empty = hexToRgb(emptyColor);
  for (let y = 0; y < 4; y += 1) {
    for (let x = 0; x < 4; x += 1) {
      const lit = density > BAYER4[y & 3][x & 3];
      const alpha = lit
        ? 0.45 + 0.55 * density
        : level <= 0
          ? 0.14
          : 0.12 * density;
      if (alpha <= 0.004) continue;
      ctx.fillStyle = rgb(lit ? fill : empty, 1, alpha);
      ctx.fillRect(x, y, 1, 1);
    }
  }
}

/** One dithered swatch tile; paints on mount and when the level changes. */
export function DitherSwatch({
  level,
  max,
  color,
  emptyColor,
  size = 10,
  title,
}: {
  level: number;
  max: number;
  color: string;
  emptyColor: string;
  size?: number;
  title?: string;
}) {
  const canvasRef = useRef<HTMLCanvasElement>(null);

  useEffect(() => {
    const canvas = canvasRef.current;
    if (canvas) paintDitherSwatch(canvas, level, max, color, emptyColor);
  }, [level, max, color, emptyColor]);

  return (
    <canvas
      ref={canvasRef}
      aria-hidden="true"
      title={title}
      style={{
        width: `${size}px`,
        height: `${size}px`,
        imageRendering: "pixelated",
      }}
    />
  );
}

/** Paint a horizontal bar whose filled span carries the kit's progress ramp. */
function paintDitherBar(
  canvas: HTMLCanvasElement,
  cols: number,
  rows: number,
  segments: readonly { fill: Rgb; from: number; to: number }[],
  emptyFill: Rgb,
  paintTrack = true,
  flatDensity?: number,
): void {
  const ctx = canvasContext(canvas);
  if (!ctx || cols <= 0) return;
  canvas.width = cols;
  canvas.height = rows;
  ctx.clearRect(0, 0, cols, rows);
  for (let y = 0; y < rows; y += 1) {
    for (let x = 0; x < cols; x += 1) {
      const position = (x + 0.5) / cols;
      const segment = segments.find(
        ({ from, to }) => position >= from && position < to,
      );
      if (!segment) {
        if (!paintTrack) continue;
        const lit = 0.25 > BAYER4[y & 3][x & 3];
        ctx.fillStyle = rgb(emptyFill, 1, lit ? 0.2 : 0.06);
        ctx.fillRect(x, y, 1, 1);
        continue;
      }
      const span = Math.max(1e-6, segment.to - segment.from);
      const density =
        flatDensity ?? 0.35 + 0.65 * ((position - segment.from) / span);
      const lit = density > BAYER4[y & 3][x & 3];
      const k = 0.3 + density * 0.7;
      ctx.fillStyle = rgb(segment.fill, 1, lit ? k : k * 0.4);
      ctx.fillRect(x, y, 1, 1);
    }
  }
}

export interface DitherBarSegment {
  /** Segment weight; a weight of zero renders no stub. */
  weight: number;
  color: string;
}

/**
 * A full-width dithered stacked bar (the token mix): each part carries the
 * kit's determinate-progress ramp, split by one-cell seams. Weights map onto
 * the bar's true ratio; a part with no weight paints nothing.
 */
export function DitherSegmentBar({
  segments,
  emptyColor = "#737373",
  height = 12,
  className,
}: {
  segments: readonly DitherBarSegment[];
  emptyColor?: string;
  height?: number;
  className?: string;
}) {
  const containerRef = useRef<HTMLDivElement>(null);
  const canvasRef = useRef<HTMLCanvasElement>(null);
  const total = segments.reduce((sum, item) => sum + Math.max(0, item.weight), 0);

  useEffect(() => {
    const container = containerRef.current;
    const canvas = canvasRef.current;
    if (!container || !canvas) return;
    const paint = () => {
      const cols = Math.max(
        4,
        Math.round(container.getBoundingClientRect().width / DITHER_CELL),
      );
      const rows = Math.max(2, Math.round(height / DITHER_CELL));
      // Build cumulative spans with a one-cell seam between parts.
      let cursor = 0;
      const painted: { fill: Rgb; from: number; to: number }[] = [];
      for (const segment of segments) {
        const weight = Math.max(0, segment.weight);
        if (weight <= 0 || total <= 0) continue;
        const to = cursor + (weight / total) * cols;
        painted.push({
          fill: hexToRgb(segment.color),
          from: cursor / cols,
          to: Math.max(cursor + 1, to - 1) / cols,
        });
        cursor = to;
      }
      paintDitherBar(canvas, cols, rows, painted, hexToRgb(emptyColor), true, 0.8);
    };
    const raf = requestAnimationFrame(paint);
    let observer: ResizeObserver | null = null;
    if (typeof ResizeObserver !== "undefined") {
      observer = new ResizeObserver(paint);
      observer.observe(container);
    }
    return () => {
      cancelAnimationFrame(raf);
      observer?.disconnect();
    };
  }, [segments, total, height, emptyColor]);

  return (
    <div ref={containerRef} className={className} style={{ height: `${height}px` }}>
      <canvas
        ref={canvasRef}
        aria-hidden="true"
        style={{
          display: "block",
          width: "100%",
          height: "100%",
          imageRendering: "pixelated",
        }}
      />
    </div>
  );
}

/**
 * A thin proportional dithered bar (the models list's share column): filled
 * span is the true ratio, no track beyond it, grey with the option of ink.
 */
export function DitherMiniBar({
  ratio,
  color = "#737373",
  width = "100%",
  className,
}: {
  ratio: number;
  color?: string;
  width?: string;
  className?: string;
}) {
  const containerRef = useRef<HTMLSpanElement>(null);
  const canvasRef = useRef<HTMLCanvasElement>(null);
  const clamped = Math.max(0, Math.min(1, Number.isFinite(ratio) ? ratio : 0));

  useEffect(() => {
    const container = containerRef.current;
    const canvas = canvasRef.current;
    if (!container || !canvas) return;
    const paint = () => {
      const cols = Math.max(
        2,
        Math.round((container.getBoundingClientRect().width / DITHER_CELL) || 0),
      );
      paintDitherBar(
        canvas,
        cols,
        1,
        clamped > 0
          ? [{ fill: hexToRgb(color), from: 0, to: clamped }]
          : [],
        hexToRgb(color),
        false,
        0.85,
      );
    };
    const raf = requestAnimationFrame(paint);
    let observer: ResizeObserver | null = null;
    if (typeof ResizeObserver !== "undefined") {
      observer = new ResizeObserver(paint);
      observer.observe(container);
    }
    return () => {
      cancelAnimationFrame(raf);
      observer?.disconnect();
    };
  }, [clamped, color]);

  return (
    <span
      ref={containerRef}
      className={className}
      style={{ display: "block", width, maxWidth: "20rem", height: 3 }}
    >
      <canvas
        ref={canvasRef}
        aria-hidden="true"
        style={{
          display: "block",
          width: "100%",
          height: "100%",
          imageRendering: "pixelated",
        }}
      />
    </span>
  );
}

export { ACCENT };
export type { ReactNode };
