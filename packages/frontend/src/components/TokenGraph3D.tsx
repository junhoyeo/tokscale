"use client";

import { useRef, useEffect, useCallback, useState } from "react";
import type { DailyContribution, GraphColorPalette, TooltipPosition } from "@/lib/types";
import { getGradeColor } from "@/lib/themes";
import { useSystemDarkMode } from "@/lib/useMediaQuery";
import { groupByWeek, hexToNumber, formatCurrency, formatDate, formatTokenCount } from "@/lib/utils";
import { CUBE_SIZE, MAX_CUBE_HEIGHT, MIN_CUBE_HEIGHT, ISO_CANVAS_WIDTH, ISO_CANVAS_HEIGHT } from "@/lib/constants";

interface TokenGraph3DProps {
  contributions: DailyContribution[];
  palette: GraphColorPalette;
  year: string;
  maxCost: number;
  totalCost: number;
  totalTokens: number;
  activeDays: number;
  bestDay: DailyContribution | null;
  currentStreak: number;
  longestStreak: number;
  dateRange: { start: string; end: string };
  onDayHover: (day: DailyContribution | null, position: TooltipPosition | null) => void;
  onDayClick: (day: DailyContribution | null) => void;
}

export function TokenGraph3D({
  contributions,
  palette,
  year,
  maxCost,
  totalCost,
  totalTokens,
  activeDays,
  bestDay,
  currentStreak,
  longestStreak,
  dateRange,
  onDayHover,
  onDayClick,
}: TokenGraph3DProps) {
  const containerRef = useRef<HTMLDivElement>(null);
  const canvasRef = useRef<HTMLCanvasElement>(null);
  const [obeliskLoaded, setObeliskLoaded] = useState(false);
  // eslint-disable-next-line @typescript-eslint/no-explicit-any
  const obeliskRef = useRef<any>(null);
  const weeksData = groupByWeek(contributions, year);
  const isDark = useSystemDarkMode();

  useEffect(() => {
    async function loadObelisk() {
      try {
        const obeliskModule = await import("obelisk.js");
        obeliskRef.current = obeliskModule.default || obeliskModule;
        setObeliskLoaded(true);
      } catch (err) {
        console.error("Failed to load obelisk.js:", err);
      }
    }
    loadObelisk();
  }, []);

  useEffect(() => {
    if (!obeliskLoaded || !obeliskRef.current) return;

    const canvas = canvasRef.current;
    if (!canvas) return;

    const obelisk = obeliskRef.current;

    canvas.width = ISO_CANVAS_WIDTH;
    canvas.height = ISO_CANVAS_HEIGHT;
    canvas.style.width = `${ISO_CANVAS_WIDTH}px`;
    canvas.style.height = `${ISO_CANVAS_HEIGHT}px`;

    const ctx = canvas.getContext("2d");
    if (!ctx) return;

    ctx.fillStyle = isDark ? "#10121C" : "#FFFFFF";
    ctx.fillRect(0, 0, ISO_CANVAS_WIDTH, ISO_CANVAS_HEIGHT);

    const point = new obelisk.Point(130, 90);
    const pixelView = new obelisk.PixelView(canvas, point);

    const GH_OFFSET = 14;
    let transform = GH_OFFSET;

    for (let weekIndex = 0; weekIndex < weeksData.length; weekIndex++) {
      const week = weeksData[weekIndex];
      const x = transform / (GH_OFFSET + 1);
      transform += GH_OFFSET;

      let offsetY = 0;
      for (let dayIndex = 0; dayIndex < 7; dayIndex++) {
        const day = week.days[dayIndex];
        const y = offsetY / GH_OFFSET;
        offsetY += 13;

        let cubeHeight = MIN_CUBE_HEIGHT;
        if (day && maxCost > 0) {
          cubeHeight = MIN_CUBE_HEIGHT + Math.floor((MAX_CUBE_HEIGHT / maxCost) * day.totals.cost);
        }

        const intensity = day?.intensity ?? 0;
        const colorHex = getGradeColor(palette, intensity);
        const resolvedColor = colorHex.startsWith("var(") 
          ? (isDark ? "#1F1F20" : "#EBEDF0") 
          : colorHex;
        const colorNum = hexToNumber(resolvedColor);

        const dimension = new obelisk.CubeDimension(CUBE_SIZE, CUBE_SIZE, Math.max(cubeHeight, MIN_CUBE_HEIGHT));
        const color = new obelisk.CubeColor().getByHorizontalColor(colorNum);
        const cube = new obelisk.Cube(dimension, color, false);
        const p3d = new obelisk.Point3D(CUBE_SIZE * x, CUBE_SIZE * y, 0);

        pixelView.renderObject(cube, p3d);
      }
    }
  }, [obeliskLoaded, contributions, palette, year, maxCost, weeksData]);

  const getDayAtPosition = useCallback(
    (clientX: number, clientY: number): { day: DailyContribution | null; position: TooltipPosition } | null => {
      const canvas = canvasRef.current;
      if (!canvas) return null;

      const rect = canvas.getBoundingClientRect();
      const scaleX = ISO_CANVAS_WIDTH / rect.width;
      const scaleY = ISO_CANVAS_HEIGHT / rect.height;
      const x = (clientX - rect.left) * scaleX;
      const y = (clientY - rect.top) * scaleY;

      const isoX = (x - 130) / (CUBE_SIZE * 0.7);
      const isoY = (y - 90) / (CUBE_SIZE * 0.35) - isoX;

      const weekIndex = Math.floor(isoX);
      const dayIndex = Math.floor(isoY);

      if (weekIndex < 0 || weekIndex >= weeksData.length || dayIndex < 0 || dayIndex >= 7) return null;

      const day = weeksData[weekIndex]?.days[dayIndex] ?? null;
      return { day, position: { x: clientX, y: clientY } };
    },
    [weeksData]
  );

  const handleMouseMove = useCallback(
    (e: React.MouseEvent<HTMLCanvasElement>) => {
      const result = getDayAtPosition(e.clientX, e.clientY);
      if (result) {
        onDayHover(result.day, result.position);
      } else {
        onDayHover(null, null);
      }
    },
    [getDayAtPosition, onDayHover]
  );

  const handleClick = useCallback(
    (e: React.MouseEvent<HTMLCanvasElement>) => {
      const result = getDayAtPosition(e.clientX, e.clientY);
      if (result?.day) onDayClick(result.day);
    },
    [getDayAtPosition, onDayClick]
  );

  if (!obeliskLoaded) {
    return (
      <div
        ref={containerRef}
        className="flex items-center justify-center w-full"
        style={{ aspectRatio: `${ISO_CANVAS_WIDTH} / ${ISO_CANVAS_HEIGHT}`, maxWidth: ISO_CANVAS_WIDTH, backgroundColor: isDark ? "#10121C" : "#FFFFFF" }}
      >
        <div className="animate-pulse" style={{ color: isDark ? "#696969" : "#656D76" }}>Loading 3D view...</div>
      </div>
    );
  }

  return (
    <div ref={containerRef} className="ic-contributions-wrapper relative w-full" style={{ maxWidth: ISO_CANVAS_WIDTH }}>
      <canvas
        ref={canvasRef}
        onMouseMove={handleMouseMove}
        onMouseLeave={() => onDayHover(null, null)}
        onClick={handleClick}
        className="cursor-pointer w-full h-auto"
        style={{ aspectRatio: `${ISO_CANVAS_WIDTH} / ${ISO_CANVAS_HEIGHT}` }}
      />

      <div className="absolute top-3 right-5">
        <h5 className="mb-1 text-sm font-semibold" style={{ color: isDark ? "#FFFFFF" : "#1F2328" }}>Token Usage</h5>
        <div
          className="flex justify-between rounded-md border px-1 md:px-2"
          style={{ borderColor: isDark ? "#262627" : "#D0D7DE", backgroundColor: isDark ? "#1F1F20" : "#FFFFFF" }}
        >
          <div className="p-2">
            <span className="block text-2xl font-bold leading-tight" style={{ color: palette.grade1 }}>{formatCurrency(totalCost)}</span>
            <span className="block text-xs font-bold" style={{ color: isDark ? "#FFFFFF" : "#1F2328" }}>Total</span>
            <span className="hidden sm:block text-xs" style={{ color: isDark ? "#696969" : "#656D76" }}>{dateRange.start} → {dateRange.end}</span>
          </div>
          <div className="p-2 hidden xl:block">
            <span className="block text-2xl font-bold leading-tight" style={{ color: palette.grade1 }}>{formatTokenCount(totalTokens)}</span>
            <span className="block text-xs font-bold" style={{ color: isDark ? "#FFFFFF" : "#1F2328" }}>Tokens</span>
            <span className="hidden sm:block text-xs" style={{ color: isDark ? "#696969" : "#656D76" }}>{activeDays} active days</span>
          </div>
          {bestDay && (
            <div className="p-2">
              <span className="block text-2xl font-bold leading-tight" style={{ color: palette.grade1 }}>{formatCurrency(bestDay.totals.cost)}</span>
              <span className="block text-xs font-bold" style={{ color: isDark ? "#FFFFFF" : "#1F2328" }}>Best day</span>
              <span className="hidden sm:block text-xs" style={{ color: isDark ? "#696969" : "#656D76" }}>{formatDate(bestDay.date).split(",")[0]}</span>
            </div>
          )}
        </div>
        <p className="mt-1 text-right text-xs" style={{ color: isDark ? "#696969" : "#656D76" }}>
          Average: <span className="font-bold" style={{ color: palette.grade1 }}>{formatCurrency(activeDays > 0 ? totalCost / activeDays : 0)}</span> / day
        </p>
      </div>

      <div className="absolute bottom-6 left-5">
        <h5 className="mb-1 text-sm font-semibold" style={{ color: isDark ? "#FFFFFF" : "#1F2328" }}>Streaks</h5>
        <div
          className="flex justify-between rounded-md border px-1 md:px-2"
          style={{ borderColor: isDark ? "#262627" : "#D0D7DE", backgroundColor: isDark ? "#1F1F20" : "#FFFFFF" }}
        >
          <div className="p-2">
            <span className="block text-2xl font-bold leading-tight" style={{ color: palette.grade1 }}>{longestStreak} <span className="text-base">days</span></span>
            <span className="block text-xs font-bold" style={{ color: isDark ? "#FFFFFF" : "#1F2328" }}>Longest</span>
          </div>
          <div className="p-2">
            <span className="block text-2xl font-bold leading-tight" style={{ color: palette.grade1 }}>{currentStreak} <span className="text-base">days</span></span>
            <span className="block text-xs font-bold" style={{ color: isDark ? "#FFFFFF" : "#1F2328" }}>Current</span>
          </div>
        </div>
      </div>
    </div>
  );
}
