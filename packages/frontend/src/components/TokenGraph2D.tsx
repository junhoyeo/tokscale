"use client";

import { useRef, useEffect, useCallback, useId, useMemo, useState } from "react";
import styled from "styled-components";
import type { DailyContribution, GraphColorPalette, TooltipPosition } from "@/lib/types";
import { getDarkGraphColor } from "@/lib/darkGraphColors";
import { dayHasActivity, formatCurrency, formatTokenCount, groupByWeek } from "@/lib/utils";
import { CELL_SIZE, FONT_FAMILY, DAY_LABELS_SHORT, MONTH_LABELS_SHORT } from "@/lib/constants";
import { addDays, format, getMonth, parseISO, startOfWeek, startOfYear } from "date-fns";

/* Canvas layout, in CSS pixels. Cells grow with the card up to MAX_CELL_SIZE and never shrink below
   the original 12px pitch; narrower cards scroll sideways instead. */
const MAX_CELL_SIZE = 24;
const LABEL_FONT_SIZE = 11;
const DAY_LABEL_WIDTH = 32;
const MONTH_LABEL_HEIGHT = 22;
const RIGHT_PADDING = 24;
const BOTTOM_PADDING = 2;

export function getGraphCellSize(availableWidth: number, weeks: number): number {
  if (weeks <= 0 || !Number.isFinite(availableWidth) || availableWidth <= 0) return CELL_SIZE;
  const fit = Math.floor((availableWidth - DAY_LABEL_WIDTH - RIGHT_PADDING) / weeks);
  return Math.max(CELL_SIZE, Math.min(MAX_CELL_SIZE, fit));
}

type Cell = readonly [weekIndex: number, dayIndex: number];

/**
 * The cell an arrow key moves to. The grid is laid out like the profile graph's: weeks are columns and
 * weekdays are rows, so Left and Right step a week and Up and Down step a day. Home and End jump to the
 * first and last week. Moves stop at the edges.
 */
export function moveGraphCell(cell: Cell, key: string, weeks: number): Cell | null {
  const [weekIndex, dayIndex] = cell;
  const clampWeek = (value: number) => Math.max(0, Math.min(weeks - 1, value));
  const clampDay = (value: number) => Math.max(0, Math.min(6, value));
  switch (key) {
    case "ArrowLeft":
      return [clampWeek(weekIndex - 1), dayIndex];
    case "ArrowRight":
      return [clampWeek(weekIndex + 1), dayIndex];
    case "ArrowUp":
      return [weekIndex, clampDay(dayIndex - 1)];
    case "ArrowDown":
      return [weekIndex, clampDay(dayIndex + 1)];
    case "Home":
      return [0, dayIndex];
    case "End":
      return [Math.max(0, weeks - 1), dayIndex];
    default:
      return null;
  }
}

const Container = styled.div`
  overflow-x: auto;
  overscroll-behavior-inline: contain;
  /* Room for the focus ring, which the scroll box would otherwise clip. */
  padding: 4px;
  margin: -4px;
  scrollbar-color: rgba(255, 255, 255, 0.22) transparent;
  scrollbar-width: thin;
`;

const GraphCanvas = styled.canvas`
  display: block;
  max-width: none;
  border-radius: 4px;
  cursor: pointer;

  &:focus {
    outline: none;
  }

  &:focus-visible {
    outline: 2px solid var(--service-focus);
    outline-offset: 2px;
  }
`;

const VisuallyHidden = styled.span`
  position: absolute;
  width: 1px;
  height: 1px;
  margin: -1px;
  padding: 0;
  overflow: hidden;
  clip: rect(0 0 0 0);
  white-space: nowrap;
  border: 0;
`;

interface TokenGraph2DProps {
  contributions: DailyContribution[];
  palette: GraphColorPalette;
  year: string;
  /** Day under the pointer; drawn with a thin outline. */
  hoveredDate?: string | null;
  /** Day shown in the breakdown panel; drawn with the focus colour. */
  selectedDate?: string | null;
  onDayHover: (day: DailyContribution | null, position: TooltipPosition | null) => void;
  onDayClick: (day: DailyContribution | null) => void;
}

export function TokenGraph2D({ contributions, palette, year, hoveredDate = null, selectedDate = null, onDayHover, onDayClick }: TokenGraph2DProps) {
  const containerRef = useRef<HTMLDivElement>(null);
  const canvasRef = useRef<HTMLCanvasElement>(null);
  const weeksData = useMemo(() => groupByWeek(contributions, year), [contributions, year]);
  const [availableWidth, setAvailableWidth] = useState(0);
  const [fontsReady, setFontsReady] = useState(false);
  const [keyboardCell, setKeyboardCell] = useState<Cell | null>(null);
  const instructionsId = useId();
  const scrollAnchor = useRef<{ key: string; overflowing: boolean } | null>(null);

  const cellSize = getGraphCellSize(availableWidth, weeksData.length);
  const gap = cellSize < 16 ? 2 : 3;
  const boxSize = cellSize - gap;
  const radius = cellSize < 16 ? 2 : 3;
  const canvasWidth = DAY_LABEL_WIDTH + weeksData.length * cellSize + RIGHT_PADDING;
  const canvasHeight = MONTH_LABEL_HEIGHT + 7 * cellSize + BOTTOM_PADDING;

  const cellIndexByDate = useMemo(() => {
    const index = new Map<string, Cell>();
    weeksData.forEach((week, weekIndex) => {
      week.days.forEach((day, dayIndex) => {
        if (day) index.set(day.date, [weekIndex, dayIndex]);
      });
    });
    return index;
  }, [weeksData]);

  // Empty calendar entries (including future days) are not a useful starting point.
  const anchorDate = useMemo(() => {
    if (selectedDate && cellIndexByDate.has(selectedDate)) return selectedDate;
    let latest: string | null = null;
    for (const day of contributions) {
      if (cellIndexByDate.has(day.date) && dayHasActivity(day) && (!latest || day.date > latest)) latest = day.date;
    }
    return latest;
  }, [selectedDate, cellIndexByDate, contributions]);
  const anchorWeek = anchorDate ? cellIndexByDate.get(anchorDate)?.[0] ?? 0 : 0;

  // Same calendar as groupByWeek: week 0 starts on the Sunday on or before January 1.
  const firstSunday = useMemo(() => startOfWeek(startOfYear(parseISO(`${year}-01-01`)), { weekStartsOn: 0 }), [year]);

  // Size the cells to the card: the observer reports the width before the first paint.
  useEffect(() => {
    const container = containerRef.current;
    if (!container || typeof ResizeObserver === "undefined") return;
    const observer = new ResizeObserver((entries) => {
      const width = entries[0]?.contentRect.width ?? 0;
      setAvailableWidth((previous) => (Math.abs(previous - width) < 0.5 ? previous : width));
    });
    observer.observe(container);
    return () => observer.disconnect();
  }, []);

  // Month and day labels use the page font; draw again once it has loaded.
  useEffect(() => {
    let cancelled = false;
    document.fonts?.ready.then(() => {
      if (!cancelled) setFontsReady(true);
    });
    return () => {
      cancelled = true;
    };
  }, []);

  useEffect(() => {
    const canvas = canvasRef.current;
    if (!canvas) return;

    const ctx = canvas.getContext("2d");
    if (!ctx) return;

    const dpr = window.devicePixelRatio || 1;
    const pixelWidth = Math.round(canvasWidth * dpr);
    const pixelHeight = Math.round(canvasHeight * dpr);
    if (canvas.width !== pixelWidth) canvas.width = pixelWidth;
    if (canvas.height !== pixelHeight) canvas.height = pixelHeight;
    canvas.style.width = `${canvasWidth}px`;
    canvas.style.height = `${canvasHeight}px`;
    ctx.setTransform(dpr, 0, 0, dpr, 0, 0);
    ctx.clearRect(0, 0, canvasWidth, canvasHeight);

    const styles = getComputedStyle(canvas);
    const labelColor = styles.getPropertyValue("--service-text-muted").trim() || "#a8b3c5";
    const focusColor = styles.getPropertyValue("--service-focus").trim() || "#75b6ff";
    const fontFamily = styles.fontFamily || FONT_FAMILY;

    ctx.font = `500 ${LABEL_FONT_SIZE}px ${fontFamily}`;
    ctx.fillStyle = labelColor;
    ctx.textBaseline = "alphabetic";
    ctx.textAlign = "left";

    let lastMonth = -1;
    for (let weekIndex = 0; weekIndex < weeksData.length; weekIndex++) {
      const week = weeksData[weekIndex];
      const firstDay = week.days.find((d) => d !== null);
      if (firstDay) {
        const month = getMonth(parseISO(firstDay.date));
        if (month !== lastMonth) {
          const x = DAY_LABEL_WIDTH + weekIndex * cellSize;
          ctx.fillText(MONTH_LABELS_SHORT[month], x, MONTH_LABEL_HEIGHT - 8);
          lastMonth = month;
        }
      }
    }

    ctx.textAlign = "right";
    ctx.textBaseline = "middle";
    for (const dayIndex of [1, 3, 5]) {
      const y = MONTH_LABEL_HEIGHT + dayIndex * cellSize + boxSize / 2;
      ctx.fillText(DAY_LABELS_SHORT[dayIndex], DAY_LABEL_WIDTH - 8, y);
    }

    for (let weekIndex = 0; weekIndex < weeksData.length; weekIndex++) {
      const week = weeksData[weekIndex];
      for (let dayIndex = 0; dayIndex < 7; dayIndex++) {
        const day = week.days[dayIndex];
        const x = DAY_LABEL_WIDTH + weekIndex * cellSize;
        const y = MONTH_LABEL_HEIGHT + dayIndex * cellSize;

        ctx.fillStyle = getDarkGraphColor(palette, day?.intensity ?? 0);
        roundRect(ctx, x, y, boxSize, boxSize, radius);
        ctx.fill();
      }
    }

    const outline = (cell: Cell | undefined, color: string, width: number) => {
      if (!cell) return;
      const [weekIndex, dayIndex] = cell;
      const x = DAY_LABEL_WIDTH + weekIndex * cellSize;
      const y = MONTH_LABEL_HEIGHT + dayIndex * cellSize;
      const inset = width / 2 + 0.5;
      ctx.strokeStyle = color;
      ctx.lineWidth = width;
      roundRect(ctx, x - inset, y - inset, boxSize + inset * 2, boxSize + inset * 2, radius + 1);
      ctx.stroke();
    };
    if (hoveredDate && hoveredDate !== selectedDate) outline(cellIndexByDate.get(hoveredDate), "rgba(244, 247, 251, 0.85)", 1);
    if (selectedDate) outline(cellIndexByDate.get(selectedDate), focusColor, 1.5);
    if (keyboardCell) outline(keyboardCell, "#f4f7fb", 2);
  }, [palette, weeksData, cellIndexByDate, cellSize, boxSize, radius, canvasWidth, canvasHeight, hoveredDate, selectedDate, keyboardCell, fontsReady]);

  // Run after sizing the canvas. Reveal the selected/latest active week, not empty December.
  // A width change within the scrolling layout must not undo deliberate horizontal scrolling.
  useEffect(() => {
    const container = containerRef.current;
    if (!container) return;
    const key = `${year}:${anchorDate ?? "empty"}`;
    const overflowing = container.scrollWidth > container.clientWidth;
    const previous = scrollAnchor.current;
    scrollAnchor.current = { key, overflowing };
    if (previous?.key === key && (previous.overflowing || !overflowing)) return;
    if (!anchorDate) {
      container.scrollLeft = 0;
    } else if (overflowing) {
      const left = DAY_LABEL_WIDTH + anchorWeek * cellSize;
      if (left < container.scrollLeft + DAY_LABEL_WIDTH) {
        container.scrollLeft = Math.max(0, left - DAY_LABEL_WIDTH);
      } else if (left + cellSize + RIGHT_PADDING > container.scrollLeft + container.clientWidth) {
        container.scrollLeft = left + cellSize + RIGHT_PADDING - container.clientWidth;
      }
    }
  }, [year, anchorDate, anchorWeek, cellSize, canvasWidth, availableWidth]);

  const getDayAtPosition = useCallback(
    (clientX: number, clientY: number): { day: DailyContribution | null; position: TooltipPosition } | null => {
      const canvas = canvasRef.current;
      if (!canvas) return null;

      const rect = canvas.getBoundingClientRect();
      const x = clientX - rect.left;
      const y = clientY - rect.top;

      const gridX = x - DAY_LABEL_WIDTH;
      const gridY = y - MONTH_LABEL_HEIGHT;

      if (gridX < 0 || gridY < 0) return null;

      const weekIndex = Math.floor(gridX / cellSize);
      const dayIndex = Math.floor(gridY / cellSize);

      if (weekIndex < 0 || weekIndex >= weeksData.length || dayIndex < 0 || dayIndex >= 7) return null;

      const day = weeksData[weekIndex]?.days[dayIndex] ?? null;
      return { day, position: { x: clientX, y: clientY } };
    },
    [weeksData, cellSize]
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

  /* Keyboard: the canvas takes focus, the arrow keys move a ring between days, the tooltip follows the
     ring, and Enter does what a click does. A visually hidden live region reads each day out. */
  const startCell = (): Cell => (anchorDate ? cellIndexByDate.get(anchorDate) : undefined) ?? [0, 0];

  const showCell = (cell: Cell) => {
    setKeyboardCell(cell);
    const [weekIndex, dayIndex] = cell;
    const container = containerRef.current;
    const canvas = canvasRef.current;
    const left = DAY_LABEL_WIDTH + weekIndex * cellSize;
    if (container) {
      if (left < container.scrollLeft + DAY_LABEL_WIDTH) {
        container.scrollLeft = Math.max(0, left - DAY_LABEL_WIDTH);
      } else if (left + boxSize > container.scrollLeft + container.clientWidth) {
        container.scrollLeft = left + boxSize - container.clientWidth + 8;
      }
    }
    const day = weeksData[weekIndex]?.days[dayIndex] ?? null;
    if (!day || !canvas) {
      onDayHover(null, null);
      return;
    }
    const rect = canvas.getBoundingClientRect();
    onDayHover(day, { x: rect.left + left + boxSize, y: rect.top + MONTH_LABEL_HEIGHT + dayIndex * cellSize + boxSize });
  };

  const handleFocus = (e: React.FocusEvent<HTMLCanvasElement>) => {
    let fromKeyboard = true;
    try {
      fromKeyboard = e.currentTarget.matches(":focus-visible");
    } catch {
      // Older engines without :focus-visible: treat focus as keyboard focus.
    }
    if (fromKeyboard) showCell(startCell());
  };

  const handleBlur = () => {
    setKeyboardCell(null);
    onDayHover(null, null);
  };

  const handleKeyDown = (e: React.KeyboardEvent<HTMLCanvasElement>) => {
    if (e.key === "Escape") {
      if (keyboardCell) {
        e.preventDefault();
        onDayHover(null, null);
      }
      return;
    }
    if (e.key === "Enter" || e.key === " ") {
      e.preventDefault();
      if (!keyboardCell) {
        showCell(startCell());
        return;
      }
      const day = weeksData[keyboardCell[0]]?.days[keyboardCell[1]] ?? null;
      if (day) onDayClick(day);
      return;
    }
    const next = moveGraphCell(keyboardCell ?? startCell(), e.key, weeksData.length);
    if (!next) return;
    e.preventDefault();
    showCell(keyboardCell ? next : startCell());
  };

  let announcement = "";
  if (keyboardCell) {
    const [weekIndex, dayIndex] = keyboardCell;
    const day = weeksData[weekIndex]?.days[dayIndex] ?? null;
    const date = format(addDays(firstSunday, weekIndex * 7 + dayIndex), "EEEE, MMMM d, yyyy");
    announcement = day
      ? `${date}: ${formatTokenCount(day.totals.tokens)} tokens, ${formatCurrency(day.totals.cost)}, ${day.totals.messages.toLocaleString("en-US")} messages${day.date === selectedDate ? ", shown in the breakdown" : ""}`
      : `${date}: no usage`;
  }

  return (
    <Container ref={containerRef}>
      <GraphCanvas
        ref={canvasRef}
        tabIndex={0}
        role="application"
        aria-label={`Daily token usage in ${year}`}
        aria-describedby={instructionsId}
        onMouseMove={handleMouseMove}
        onMouseLeave={() => onDayHover(null, null)}
        onClick={handleClick}
        onFocus={handleFocus}
        onBlur={handleBlur}
        onKeyDown={handleKeyDown}
        style={{ minWidth: canvasWidth }}
      />
      <VisuallyHidden id={instructionsId}>
        Use the arrow keys to move between days, Home and End to jump to the first and last week, Enter to show
        the day in the breakdown, and Escape to hide the tooltip.
      </VisuallyHidden>
      <VisuallyHidden aria-live="polite">{announcement}</VisuallyHidden>
    </Container>
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
