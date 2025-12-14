"use client";

import { useRef, useEffect, useCallback, useState } from "react";
import type { DailyContribution, GraphColorPalette, TooltipPosition } from "@/lib/types";
import { getGradeColor } from "@/lib/themes";
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
  const canvasRef = useRef<HTMLCanvasElement>(null);
  const [obeliskLoaded, setObeliskLoaded] = useState(false);
  // eslint-disable-next-line @typescript-eslint/no-explicit-any
  const obeliskRef = useRef<any>(null);
  const weeksData = groupByWeek(contributions, year);

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

    ctx.fillStyle = "#141415";
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
        const resolvedColor = colorHex.startsWith("var(") ? "#1F1F20" : colorHex;
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
      const x = clientX - rect.left;
      const y = clientY - rect.top;

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
        className="flex items-center justify-center"
        style={{ width: ISO_CANVAS_WIDTH, height: ISO_CANVAS_HEIGHT, backgroundColor: "#141415" }}
      >
        <div className="animate-pulse" style={{ color: "#696969" }}>Loading 3D view...</div>
      </div>
    );
  }

  return (
    <div className="ic-contributions-wrapper relative overflow-x-auto">
      <canvas
        ref={canvasRef}
        onMouseMove={handleMouseMove}
        onMouseLeave={() => onDayHover(null, null)}
        onClick={handleClick}
        className="cursor-pointer"
        style={{ width: "100%", minWidth: ISO_CANVAS_WIDTH }}
      />

      <div className="absolute top-3 right-5">
        <h5 className="mb-1 text-sm font-semibold" style={{ color: "#FFFFFF" }}>Token Usage</h5>
        <div
          className="flex justify-between rounded-md border px-1 md:px-2"
          style={{ borderColor: "#262627", backgroundColor: "#1F1F20" }}
        >
          <div className="p-2">
            <span className="block text-2xl font-bold leading-tight" style={{ color: palette.grade4 }}>{formatCurrency(totalCost)}</span>
            <span className="block text-xs font-bold" style={{ color: "#FFFFFF" }}>Total</span>
            <span className="hidden sm:block text-xs" style={{ color: "#696969" }}>{dateRange.start} → {dateRange.end}</span>
          </div>
          <div className="p-2 hidden xl:block">
            <span className="block text-2xl font-bold leading-tight" style={{ color: palette.grade4 }}>{formatTokenCount(totalTokens)}</span>
            <span className="block text-xs font-bold" style={{ color: "#FFFFFF" }}>Tokens</span>
            <span className="hidden sm:block text-xs" style={{ color: "#696969" }}>{activeDays} active days</span>
          </div>
          {bestDay && (
            <div className="p-2">
              <span className="block text-2xl font-bold leading-tight" style={{ color: palette.grade4 }}>{formatCurrency(bestDay.totals.cost)}</span>
              <span className="block text-xs font-bold" style={{ color: "#FFFFFF" }}>Best day</span>
              <span className="hidden sm:block text-xs" style={{ color: "#696969" }}>{formatDate(bestDay.date).split(",")[0]}</span>
            </div>
          )}
        </div>
        <p className="mt-1 text-right text-xs" style={{ color: "#696969" }}>
          Average: <span className="font-bold" style={{ color: palette.grade4 }}>{formatCurrency(activeDays > 0 ? totalCost / activeDays : 0)}</span> / day
        </p>
      </div>

      <div className="absolute bottom-6 left-5">
        <h5 className="mb-1 text-sm font-semibold" style={{ color: "#FFFFFF" }}>Streaks</h5>
        <div
          className="flex justify-between rounded-md border px-1 md:px-2"
          style={{ borderColor: "#262627", backgroundColor: "#1F1F20" }}
        >
          <div className="p-2">
            <span className="block text-2xl font-bold leading-tight" style={{ color: palette.grade4 }}>{longestStreak} <span className="text-base">days</span></span>
            <span className="block text-xs font-bold" style={{ color: "#FFFFFF" }}>Longest</span>
          </div>
          <div className="p-2">
            <span className="block text-2xl font-bold leading-tight" style={{ color: palette.grade4 }}>{currentStreak} <span className="text-base">days</span></span>
            <span className="block text-xs font-bold" style={{ color: "#FFFFFF" }}>Current</span>
          </div>
        </div>
      </div>
    </div>
  );
}
