"use client";

import { useRef, useEffect, useCallback } from "react";
import type { DailyContribution, GraphColorPalette, TooltipPosition } from "@/lib/types";
import { getGradeColor } from "@/lib/themes";
import { groupByWeek } from "@/lib/utils";
import { useSystemDarkMode } from "@/lib/useMediaQuery";
import { BOX_WIDTH, CELL_SIZE, CANVAS_MARGIN, HEADER_HEIGHT, TEXT_HEIGHT, FONT_SIZE, FONT_FAMILY, DAY_LABELS_SHORT, MONTH_LABELS_SHORT } from "@/lib/constants";
import { parseISO, getMonth } from "date-fns";

interface TokenGraph2DProps {
  contributions: DailyContribution[];
  palette: GraphColorPalette;
  year: string;
  onDayHover: (day: DailyContribution | null, position: TooltipPosition | null) => void;
  onDayClick: (day: DailyContribution | null) => void;
}

export function TokenGraph2D({ contributions, palette, year, onDayHover, onDayClick }: TokenGraph2DProps) {
  const canvasRef = useRef<HTMLCanvasElement>(null);
  const weeksData = groupByWeek(contributions, year);
  const isDark = useSystemDarkMode();

  const getCSSVar = (varName: string): string => {
    if (typeof window === "undefined") return "";
    return getComputedStyle(document.documentElement).getPropertyValue(varName).trim();
  };

  const canvasWidth = CANVAS_MARGIN * 2 + TEXT_HEIGHT + weeksData.length * CELL_SIZE;
  const canvasHeight = HEADER_HEIGHT + 7 * CELL_SIZE + CANVAS_MARGIN;

  useEffect(() => {
    const canvas = canvasRef.current;
    if (!canvas) return;

    const ctx = canvas.getContext("2d");
    if (!ctx) return;

    const dpr = window.devicePixelRatio || 1;
    canvas.width = canvasWidth * dpr;
    canvas.height = canvasHeight * dpr;
    canvas.style.width = `${canvasWidth}px`;
    canvas.style.height = `${canvasHeight}px`;
    ctx.scale(dpr, dpr);

    const bgColor = getCSSVar("--color-graph-canvas") || (isDark ? "#0d1117" : "#ffffff");
    ctx.fillStyle = bgColor;
    ctx.fillRect(0, 0, canvasWidth, canvasHeight);

    ctx.font = `${FONT_SIZE}px ${FONT_FAMILY}`;
    const metaColor = getCSSVar("--color-fg-muted") || (isDark ? "#7d8590" : "#656d76");
    ctx.fillStyle = metaColor;
    ctx.textAlign = "left";

    let lastMonth = -1;
    for (let weekIndex = 0; weekIndex < weeksData.length; weekIndex++) {
      const week = weeksData[weekIndex];
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

    ctx.textAlign = "right";
    for (const dayIndex of [1, 3, 5]) {
      const y = HEADER_HEIGHT + dayIndex * CELL_SIZE + BOX_WIDTH / 2 + FONT_SIZE / 3;
      ctx.fillText(DAY_LABELS_SHORT[dayIndex], CANVAS_MARGIN + TEXT_HEIGHT - 4, y);
    }

    for (let weekIndex = 0; weekIndex < weeksData.length; weekIndex++) {
      const week = weeksData[weekIndex];
      for (let dayIndex = 0; dayIndex < 7; dayIndex++) {
        const day = week.days[dayIndex];
        const x = CANVAS_MARGIN + TEXT_HEIGHT + weekIndex * CELL_SIZE;
        const y = HEADER_HEIGHT + dayIndex * CELL_SIZE;

        const intensity = day?.intensity ?? 0;
        const colorHex = getGradeColor(palette, intensity);
        const resolvedColor = colorHex.startsWith("var(")
          ? getCSSVar("--color-graph-empty") || (isDark ? "#161b22" : "#ebedf0")
          : colorHex;
        ctx.fillStyle = resolvedColor;

        roundRect(ctx, x, y, BOX_WIDTH, BOX_WIDTH, 2);
        ctx.fill();
      }
    }
  }, [contributions, palette, year, weeksData, canvasWidth, canvasHeight, isDark]);

  const getDayAtPosition = useCallback(
    (clientX: number, clientY: number): { day: DailyContribution | null; position: TooltipPosition } | null => {
      const canvas = canvasRef.current;
      if (!canvas) return null;

      const rect = canvas.getBoundingClientRect();
      const x = clientX - rect.left;
      const y = clientY - rect.top;

      const gridX = x - CANVAS_MARGIN - TEXT_HEIGHT;
      const gridY = y - HEADER_HEIGHT;

      if (gridX < 0 || gridY < 0) return null;

      const weekIndex = Math.floor(gridX / CELL_SIZE);
      const dayIndex = Math.floor(gridY / CELL_SIZE);

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

  return (
    <div className="overflow-x-auto">
      <canvas
        ref={canvasRef}
        onMouseMove={handleMouseMove}
        onMouseLeave={() => onDayHover(null, null)}
        onClick={handleClick}
        className="cursor-pointer"
        style={{ minWidth: canvasWidth }}
      />
    </div>
  );
}

function roundRect(ctx: CanvasRenderingContext2D, x: number, y: number, width: number, height: number, radius: number) {
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
