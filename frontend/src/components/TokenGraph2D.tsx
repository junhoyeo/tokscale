"use client";

import { useRef, useEffect, useCallback, useState } from "react";
import type { DailyContribution, GraphColorPalette, TooltipPosition } from "@/lib/types";
import { getGradeColor } from "@/lib/themes";
import { groupByWeek } from "@/lib/utils";
import {
  BOX_WIDTH,
  CELL_SIZE,
  CANVAS_MARGIN,
  HEADER_HEIGHT,
  TEXT_HEIGHT,
  FONT_SIZE,
  FONT_FAMILY,
  DAY_LABELS_SHORT,
  MONTH_LABELS_SHORT,
} from "@/lib/constants";
import { parseISO, getMonth } from "date-fns";

interface TokenGraph2DProps {
  contributions: DailyContribution[];
  palette: GraphColorPalette;
  year: string;
  onDayHover: (day: DailyContribution | null, position: TooltipPosition | null) => void;
  onDayClick: (day: DailyContribution | null) => void;
}

// Hook to detect system dark mode
function useSystemDarkMode() {
  const [isDark, setIsDark] = useState(false);

  useEffect(() => {
    const mq = window.matchMedia("(prefers-color-scheme: dark)");
    setIsDark(mq.matches);

    const handler = (e: MediaQueryListEvent) => setIsDark(e.matches);
    mq.addEventListener("change", handler);
    return () => mq.removeEventListener("change", handler);
  }, []);

  return isDark;
}

export function TokenGraph2D({
  contributions,
  palette,
  year,
  onDayHover,
  onDayClick,
}: TokenGraph2DProps) {
  const canvasRef = useRef<HTMLCanvasElement>(null);
  const weeksData = groupByWeek(contributions, year);
  const isDark = useSystemDarkMode();

  // Get CSS variable value at runtime
  const getCSSVar = (varName: string): string => {
    if (typeof window === "undefined") return "";
    return getComputedStyle(document.documentElement).getPropertyValue(varName).trim();
  };

  // Calculate canvas dimensions
  const canvasWidth = CANVAS_MARGIN * 2 + TEXT_HEIGHT + weeksData.length * CELL_SIZE;
  const canvasHeight = HEADER_HEIGHT + 7 * CELL_SIZE + CANVAS_MARGIN;

  // Render the graph
  useEffect(() => {
    const canvas = canvasRef.current;
    if (!canvas) return;

    const ctx = canvas.getContext("2d");
    if (!ctx) return;

    // Handle retina displays
    const dpr = window.devicePixelRatio || 1;
    canvas.width = canvasWidth * dpr;
    canvas.height = canvasHeight * dpr;
    canvas.style.width = `${canvasWidth}px`;
    canvas.style.height = `${canvasHeight}px`;
    ctx.scale(dpr, dpr);

    // Clear and set background
    const bgColor = getCSSVar("--color-graph-canvas") || (isDark ? "#0d1117" : "#ffffff");
    ctx.fillStyle = bgColor;
    ctx.fillRect(0, 0, canvasWidth, canvasHeight);

    // Draw month labels
    ctx.font = `${FONT_SIZE}px ${FONT_FAMILY}`;
    const metaColor = getCSSVar("--color-fg-muted") || (isDark ? "#7d8590" : "#656d76");
    ctx.fillStyle = metaColor;
    ctx.textAlign = "left";

    let lastMonth = -1;
    for (let weekIndex = 0; weekIndex < weeksData.length; weekIndex++) {
      const week = weeksData[weekIndex];
      // Find first non-null day in the week
      const firstDay = week.days.find((d) => d !== null);
      if (firstDay) {
        const month = getMonth(parseISO(firstDay.date));
        if (month !== lastMonth) {
          const x = CANVAS_MARGIN + TEXT_HEIGHT + weekIndex * CELL_SIZE;
          ctx.fillText(MONTH_LABELS_SHORT[month], x, CANVAS_MARGIN + FONT_SIZE);
          lastMonth = month;
        }
      }
    }

    // Draw day labels (Mon, Wed, Fri)
    ctx.textAlign = "right";
    const dayIndicesToShow = [1, 3, 5]; // Mon, Wed, Fri
    for (const dayIndex of dayIndicesToShow) {
      const y = HEADER_HEIGHT + dayIndex * CELL_SIZE + BOX_WIDTH / 2 + FONT_SIZE / 3;
      ctx.fillText(DAY_LABELS_SHORT[dayIndex], CANVAS_MARGIN + TEXT_HEIGHT - 4, y);
    }

    // Draw contribution cells
    for (let weekIndex = 0; weekIndex < weeksData.length; weekIndex++) {
      const week = weeksData[weekIndex];
      for (let dayIndex = 0; dayIndex < 7; dayIndex++) {
        const day = week.days[dayIndex];
        const x = CANVAS_MARGIN + TEXT_HEIGHT + weekIndex * CELL_SIZE;
        const y = HEADER_HEIGHT + dayIndex * CELL_SIZE;

        // Get color based on intensity
        const intensity = day?.intensity ?? 0;
        const colorHex = getGradeColor(palette, intensity);
        // Handle CSS variable for grade0
        const resolvedColor = colorHex.startsWith("var(")
          ? (getCSSVar("--color-graph-empty") || (isDark ? "#161b22" : "#ebedf0"))
          : colorHex;
        ctx.fillStyle = resolvedColor;

        // Draw rounded rectangle
        roundRect(ctx, x, y, BOX_WIDTH, BOX_WIDTH, 2);
        ctx.fill();
      }
    }
  }, [contributions, palette, year, weeksData, canvasWidth, canvasHeight, isDark]);

  // Hit testing for mouse position
  const getDayAtPosition = useCallback(
    (clientX: number, clientY: number): { day: DailyContribution | null; position: TooltipPosition } | null => {
      const canvas = canvasRef.current;
      if (!canvas) return null;

      const rect = canvas.getBoundingClientRect();
      const x = clientX - rect.left;
      const y = clientY - rect.top;

      // Check if within grid area
      const gridX = x - CANVAS_MARGIN - TEXT_HEIGHT;
      const gridY = y - HEADER_HEIGHT;

      if (gridX < 0 || gridY < 0) return null;

      const weekIndex = Math.floor(gridX / CELL_SIZE);
      const dayIndex = Math.floor(gridY / CELL_SIZE);

      if (weekIndex < 0 || weekIndex >= weeksData.length || dayIndex < 0 || dayIndex >= 7) {
        return null;
      }

      const day = weeksData[weekIndex]?.days[dayIndex] ?? null;
      return { day, position: { x: clientX, y: clientY } };
    },
    [weeksData]
  );

  // Mouse event handlers
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

  const handleMouseLeave = useCallback(() => {
    onDayHover(null, null);
  }, [onDayHover]);

  const handleClick = useCallback(
    (e: React.MouseEvent<HTMLCanvasElement>) => {
      const result = getDayAtPosition(e.clientX, e.clientY);
      if (result && result.day) {
        onDayClick(result.day);
      }
    },
    [getDayAtPosition, onDayClick]
  );

  return (
    <div className="overflow-x-auto">
      <canvas
        ref={canvasRef}
        onMouseMove={handleMouseMove}
        onMouseLeave={handleMouseLeave}
        onClick={handleClick}
        className="cursor-pointer"
        style={{
          minWidth: canvasWidth,
        }}
      />
    </div>
  );
}

/**
 * Draw a rounded rectangle path
 */
function roundRect(
  ctx: CanvasRenderingContext2D,
  x: number,
  y: number,
  width: number,
  height: number,
  radius: number
) {
  ctx.beginPath();
  ctx.moveTo(x + radius, y);
  ctx.lineTo(x + width - radius, y);
  ctx.quadraticCurveTo(x + width, y, x + width, y + radius);
  ctx.lineTo(x + width, y + height - radius);
  ctx.quadraticCurveTo(x + width, y + height, x + width - radius, y + height);
  ctx.lineTo(x + radius, y + height);
  ctx.quadraticCurveTo(x, y + height, x, y + height - radius);
  ctx.lineTo(x, y + radius);
  ctx.quadraticCurveTo(x, y, x + radius, y);
  ctx.closePath();
}
