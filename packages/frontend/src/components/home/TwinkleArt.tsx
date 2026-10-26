"use client";

import { useEffect, useRef } from "react";
import styled from "styled-components";
import { DOT_CELL, DOT_OFFSET, DOT_SIZE, findDots, pickWeighted, prefixSums, twinkleAlpha } from "./dotTwinkle";

interface TwinkleArtProps {
  src: string;
  width: number;
  height: number;
  /** Smaller rendition for screens up to 720px wide. */
  mobileSrc: string;
  mobileWidth: number;
  mobileHeight: number;
  loading?: "lazy" | "eager";
  className?: string;
}

/** Concurrent twinkles: a small share of the dots, capped. */
const SHARE = 0.026;
const MAX_ACTIVE = 160;
const FRAME_MS = 1000 / 30;

interface Twinkle {
  index: number;
  start: number;
  duration: number;
}

/**
 * The blue dot-matrix art with a canvas on top that makes a few dots twinkle at a
 * time. Dot positions are read from the image itself, so the overlay always lines
 * up with the art. Visitors who prefer reduced motion get the still image; the
 * animation also pauses while the art is off screen.
 */
export function TwinkleArt({ src, width, height, mobileSrc, mobileWidth, mobileHeight, loading = "eager", className }: TwinkleArtProps) {
  const imgRef = useRef<HTMLImageElement>(null);
  const canvasRef = useRef<HTMLCanvasElement>(null);

  useEffect(() => {
    const img = imgRef.current;
    const canvas = canvasRef.current;
    if (!img || !canvas) return;
    if (window.matchMedia("(prefers-reduced-motion: reduce)").matches) return;

    let frame = 0;
    let visible = false;
    let disposed = false;
    let lastDraw = 0;
    let dots: ReturnType<typeof findDots> = [];
    let prefix: number[] = [];
    let sourceWidth = 0;
    let active: Twinkle[] = [];

    const context = canvas.getContext("2d");
    if (!context) return;

    const resize = () => {
      const ratio = Math.min(window.devicePixelRatio || 1, 2);
      const rect = canvas.getBoundingClientRect();
      canvas.width = Math.max(1, Math.round(rect.width * ratio));
      canvas.height = Math.max(1, Math.round(rect.height * ratio));
    };

    const sample = () => {
      if (!img.naturalWidth) return;
      sourceWidth = img.naturalWidth;
      const cols = Math.floor(img.naturalWidth / DOT_CELL);
      const rows = Math.floor(img.naturalHeight / DOT_CELL);
      const probe = document.createElement("canvas");
      probe.width = cols;
      probe.height = rows;
      const probeContext = probe.getContext("2d", { willReadFrequently: true });
      if (!probeContext) return;
      probeContext.imageSmoothingEnabled = false;
      probeContext.drawImage(img, 0, 0, cols * DOT_CELL, rows * DOT_CELL, 0, 0, cols, rows);
      dots = findDots(probeContext.getImageData(0, 0, cols, rows).data, cols, rows);
      prefix = prefixSums(dots);
      active = [];
    };

    const draw = (now: number) => {
      frame = 0;
      if (disposed || !visible) return;
      frame = requestAnimationFrame(draw);
      if (now - lastDraw < FRAME_MS || dots.length === 0) return;
      lastDraw = now;

      const target = Math.min(MAX_ACTIVE, Math.round(dots.length * SHARE));
      active = active.filter((t) => now - t.start < t.duration);
      while (active.length < target) {
        const index = pickWeighted(prefix, Math.random());
        if (index < 0) break;
        active.push({ index, start: now - Math.random() * 400, duration: 1400 + Math.random() * 2200 });
      }

      const scale = canvas.width / sourceWidth;
      const size = DOT_SIZE * scale;
      context.clearRect(0, 0, canvas.width, canvas.height);
      for (const t of active) {
        const alpha = twinkleAlpha((now - t.start) / t.duration);
        if (alpha <= 0) continue;
        const dot = dots[t.index];
        const x = (dot.col * DOT_CELL + DOT_OFFSET) * scale;
        const y = (dot.row * DOT_CELL + DOT_OFFSET) * scale;
        context.fillStyle = `rgba(92, 145, 240, ${(alpha * 0.42).toFixed(3)})`;
        context.fillRect(x - size * 0.8, y - size * 0.8, size * 2.6, size * 2.6);
        context.fillStyle = `rgba(214, 230, 255, ${(alpha * 0.95).toFixed(3)})`;
        context.fillRect(x, y, size, size);
      }
    };

    const start = () => {
      if (!frame && visible && !disposed) frame = requestAnimationFrame(draw);
    };

    const onLoad = () => {
      sample();
      resize();
      start();
    };

    const resizeObserver = new ResizeObserver(() => {
      resize();
      if (img.naturalWidth && img.naturalWidth !== sourceWidth) sample();
    });
    resizeObserver.observe(canvas);

    const intersection = new IntersectionObserver(
      (entries) => {
        visible = entries.some((entry) => entry.isIntersecting);
        if (visible) start();
        else context.clearRect(0, 0, canvas.width, canvas.height);
      },
      { rootMargin: "100px" },
    );
    intersection.observe(canvas);

    if (img.complete && img.naturalWidth) onLoad();
    else img.addEventListener("load", onLoad);

    return () => {
      disposed = true;
      if (frame) cancelAnimationFrame(frame);
      img.removeEventListener("load", onLoad);
      resizeObserver.disconnect();
      intersection.disconnect();
    };
  }, []);

  return (
    <Wrap className={className}>
      <picture>
        <source media="(max-width: 720px)" srcSet={mobileSrc} width={mobileWidth} height={mobileHeight} />
        <img ref={imgRef} src={src} alt="" width={width} height={height} loading={loading} decoding="async" />
      </picture>
      <Canvas ref={canvasRef} aria-hidden="true" />
    </Wrap>
  );
}

const Wrap = styled.span`
  position: relative;
  display: block;

  img {
    display: block;
    width: 100%;
    height: auto;
  }
`;

const Canvas = styled.canvas`
  position: absolute;
  inset: 0;
  width: 100%;
  height: 100%;
  pointer-events: none;
`;
