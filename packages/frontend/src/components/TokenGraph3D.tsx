"use client";

import { useRef, useEffect, useCallback, useState } from "react";
import styled, { keyframes } from "styled-components";
import type { DailyContribution, GraphColorPalette, TooltipPosition } from "@/lib/types";
import { getGradeColor } from "@/lib/themes";
import { useSystemDarkMode } from "@/lib/useMediaQuery";
import { groupByWeek, hexToNumber, formatCurrency, formatDate, formatTokenCount } from "@/lib/utils";
import { CUBE_SIZE, MAX_CUBE_HEIGHT, MIN_CUBE_HEIGHT, ISO_CANVAS_WIDTH, ISO_CANVAS_HEIGHT } from "@/lib/constants";

const pulse = keyframes`
  0%, 100% { opacity: 1; }
  50% { opacity: .5; }
`;

const LoadingContainer = styled.div`
  display: flex;
  align-items: center;
  justify-content: center;
  width: 100%;
`;

const LoadingText = styled.div`
  animation: ${pulse} 2s cubic-bezier(0.4, 0, 0.6, 1) infinite;
`;

const Container = styled.div.attrs({ className: "ic-contributions-wrapper" })`
  position: relative;
  width: 100%;
`;

const StyledCanvas = styled.canvas`
  cursor: pointer;
  width: 100%;
  height: auto;
`;

const TopRightStats = styled.div`
  position: absolute;
  top: 12px;
  right: 20px;
`;

const BottomLeftStats = styled.div`
  position: absolute;
  bottom: 24px;
  left: 20px;
`;

const StatsTitle = styled.h5`
  margin-bottom: 4px;
  font-size: 14px;
  font-weight: 600;
`;

const StatsBox = styled.div`
  display: flex;
  justify-content: space-between;
  border-radius: 6px;
  border-width: 1px;
  border-style: solid;
  padding-left: 4px;
  padding-right: 4px;
  
  @media (min-width: 768px) {
    padding-left: 8px;
    padding-right: 8px;
  }
`;

const StatItem = styled.div`
  padding: 8px;
`;

const HiddenStatItem = styled(StatItem)`
  display: none;
  @media (min-width: 1280px) {
    display: block;
  }
`;

const StatValue = styled.span`
  display: block;
  font-size: 24px;
  font-weight: 700;
  line-height: 1.25;
`;

const StatLabel = styled.span`
  display: block;
  font-size: 12px;
  font-weight: 700;
`;

const StatSubtext = styled.span`
  display: none;
  font-size: 12px;
  
  @media (min-width: 640px) {
    display: block;
  }
`;

const StatsFooter = styled.p`
  margin-top: 4px;
  text-align: right;
  font-size: 12px;
`;

const SpanBase = styled.span`
  font-size: 16px;
`;

const SpanBold = styled.span`
  font-weight: 700;
`;

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
          ? (isDark ? "#1A212A" : "#EBEDF0") 
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
      <LoadingContainer
        ref={containerRef}
        style={{ aspectRatio: `${ISO_CANVAS_WIDTH} / ${ISO_CANVAS_HEIGHT}`, backgroundColor: isDark ? "#10121C" : "#FFFFFF" }}
      >
        <LoadingText style={{ color: isDark ? "#4B6486" : "#656D76" }}>Loading 3D view...</LoadingText>
      </LoadingContainer>
    );
  }

  return (
    <Container ref={containerRef}>
      <StyledCanvas
        ref={canvasRef}
        onMouseMove={handleMouseMove}
        onMouseLeave={() => onDayHover(null, null)}
        onClick={handleClick}
        style={{ aspectRatio: `${ISO_CANVAS_WIDTH} / ${ISO_CANVAS_HEIGHT}` }}
      />

      <TopRightStats>
        <StatsTitle style={{ color: isDark ? "#FFFFFF" : "#1F2328" }}>Token Usage</StatsTitle>
        <StatsBox
          style={{ borderColor: isDark ? "#1E2733" : "#D0D7DE", backgroundColor: isDark ? "#1A212A" : "#FFFFFF" }}
        >
          <StatItem>
            <StatValue style={{ color: palette.grade1 }}>{formatCurrency(totalCost)}</StatValue>
            <StatLabel style={{ color: isDark ? "#FFFFFF" : "#1F2328" }}>Total</StatLabel>
            <StatSubtext style={{ color: isDark ? "#4B6486" : "#656D76" }}>{dateRange.start} → {dateRange.end}</StatSubtext>
          </StatItem>
          <HiddenStatItem>
            <StatValue style={{ color: palette.grade1 }}>{formatTokenCount(totalTokens)}</StatValue>
            <StatLabel style={{ color: isDark ? "#FFFFFF" : "#1F2328" }}>Tokens</StatLabel>
            <StatSubtext style={{ color: isDark ? "#4B6486" : "#656D76" }}>{activeDays} active days</StatSubtext>
          </HiddenStatItem>
          {bestDay && (
            <StatItem>
              <StatValue style={{ color: palette.grade1 }}>{formatCurrency(bestDay.totals.cost)}</StatValue>
              <StatLabel style={{ color: isDark ? "#FFFFFF" : "#1F2328" }}>Best day</StatLabel>
              <StatSubtext style={{ color: isDark ? "#4B6486" : "#656D76" }}>{formatDate(bestDay.date).split(",")[0]}</StatSubtext>
            </StatItem>
          )}
        </StatsBox>
        <StatsFooter style={{ color: isDark ? "#4B6486" : "#656D76" }}>
          Average: <SpanBold style={{ color: palette.grade1 }}>{formatCurrency(activeDays > 0 ? totalCost / activeDays : 0)}</SpanBold> / day
        </StatsFooter>
      </TopRightStats>

      <BottomLeftStats>
        <StatsTitle style={{ color: isDark ? "#FFFFFF" : "#1F2328" }}>Streaks</StatsTitle>
        <StatsBox
          style={{ borderColor: isDark ? "#1E2733" : "#D0D7DE", backgroundColor: isDark ? "#1A212A" : "#FFFFFF" }}
        >
          <StatItem>
            <StatValue style={{ color: palette.grade1 }}>{longestStreak} <SpanBase>days</SpanBase></StatValue>
            <StatLabel style={{ color: isDark ? "#FFFFFF" : "#1F2328" }}>Longest</StatLabel>
          </StatItem>
          <StatItem>
            <StatValue style={{ color: palette.grade1 }}>{currentStreak} <SpanBase>days</SpanBase></StatValue>
            <StatLabel style={{ color: isDark ? "#FFFFFF" : "#1F2328" }}>Current</StatLabel>
          </StatItem>
        </StatsBox>
      </BottomLeftStats>
    </Container>
  );
}
