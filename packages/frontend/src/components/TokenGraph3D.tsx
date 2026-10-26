"use client";

import { useRef, useEffect, useCallback, useState, useMemo } from "react";
import styled, { keyframes } from "styled-components";
import type { DailyContribution, GraphColorPalette, TooltipPosition } from "@/lib/types";
import { DARK_GRAPH_EMPTY_CUBE, getDarkGraphColor } from "@/lib/darkGraphColors";
import { groupByWeek, hexToNumber, formatCurrency, formatTokenCount } from "@/lib/utils";
import { formatContributionDate } from "@/lib/date-utils";
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
  color: var(--service-text-muted);
  font-size: 14px;
`;

const LoadingText = styled.div`
  animation: ${pulse} 2s cubic-bezier(0.4, 0, 0.6, 1) infinite;

  @media (prefers-reduced-motion: reduce) {
    animation: none;
  }
`;

/* The stats float over the empty corners of the isometric view. When the card is too narrow for
   that, they move below the canvas instead of covering the cubes. */
const Container = styled.div.attrs({ className: "ic-contributions-wrapper" })`
  width: 100%;
  container-type: inline-size;
`;

const Stage = styled.div`
  position: relative;
  display: flex;
  flex-wrap: wrap;
  gap: 12px;
`;

const StyledCanvas = styled.canvas`
  flex: 1 1 100%;
  width: 100%;
  max-width: 100%;
  height: auto;
  cursor: pointer;
`;

const StatsGroup = styled.div`
  position: absolute;
  min-width: 0;

  @container (max-width: 760px) {
    position: static;
    flex: 1 1 260px;
  }
`;

const TopRightStats = styled(StatsGroup)`
  top: 0;
  right: 0;
`;

const BottomLeftStats = styled(StatsGroup)`
  bottom: 0;
  left: 0;
`;

const StatsTitle = styled.h3`
  margin: 0 0 6px;
  color: var(--service-text);
  font-size: 13px;
  font-weight: 600;
`;

const StatsBox = styled.div`
  display: flex;
  flex-wrap: wrap;
  overflow: hidden;
  border: 1px solid var(--service-border);
  border-radius: 10px;
  background: rgba(13, 16, 22, 0.88);
  backdrop-filter: blur(6px);
  -webkit-backdrop-filter: blur(6px);
`;

const StatItem = styled.div`
  flex: 1 1 auto;
  min-width: 0;
  padding: 8px 12px;

  & + & {
    border-left: 1px solid var(--service-border);
  }
`;

const TokensStatItem = styled(StatItem)`
  display: none;

  @container (min-width: 1000px) {
    display: block;
  }

  @container (max-width: 760px) {
    display: block;
  }
`;

const StatValue = styled.span<{ $emphasis?: boolean }>`
  display: block;
  color: ${({ $emphasis }) => ($emphasis ? "var(--v2-numeric)" : "var(--service-text)")};
  font-size: 22px;
  font-weight: 600;
  font-variant-numeric: tabular-nums;
  letter-spacing: -0.02em;
  line-height: 1.25;
  white-space: nowrap;

  @container (max-width: 760px) {
    font-size: 18px;
  }
`;

const StatLabel = styled.span`
  display: block;
  margin-top: 2px;
  color: var(--service-text);
  font-size: 12px;
  font-weight: 500;
`;

const StatSubtext = styled.span`
  display: block;
  color: var(--service-text-muted);
  font-size: 12px;
  white-space: nowrap;
`;

const StatsFooter = styled.p`
  margin: 6px 0 0;
  color: var(--service-text-muted);
  font-size: 12px;
  text-align: right;

  @container (max-width: 760px) {
    text-align: left;
  }
`;

const SpanBase = styled.span`
  font-size: 14px;
  font-weight: 500;
`;

const SpanBold = styled.span`
  color: var(--service-text);
  font-weight: 600;
  font-variant-numeric: tabular-nums;
`;

interface TokenGraph3DProps {
  contributions: DailyContribution[];
  palette: GraphColorPalette;
  year: string;
  maxTokens: number;
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
  maxTokens,
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
  const weeksData = useMemo(() => groupByWeek(contributions, year), [contributions, year]);

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

    // Transparent: the card's surface shows through, as in the 2D view.
    ctx.clearRect(0, 0, ISO_CANVAS_WIDTH, ISO_CANVAS_HEIGHT);

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
        if (day && maxTokens > 0) {
          cubeHeight = MIN_CUBE_HEIGHT + Math.floor((MAX_CUBE_HEIGHT / maxTokens) * day.totals.tokens);
        }

        const intensity = day?.intensity ?? 0;
        const resolvedColor = intensity > 0 ? getDarkGraphColor(palette, intensity) : DARK_GRAPH_EMPTY_CUBE;
        const colorNum = hexToNumber(resolvedColor);

        const dimension = new obelisk.CubeDimension(CUBE_SIZE, CUBE_SIZE, Math.max(cubeHeight, MIN_CUBE_HEIGHT));
        const color = new obelisk.CubeColor().getByHorizontalColor(colorNum);
        const cube = new obelisk.Cube(dimension, color, false);
        const p3d = new obelisk.Point3D(CUBE_SIZE * x, CUBE_SIZE * y, 0);

        pixelView.renderObject(cube, p3d);
      }
    }
  }, [obeliskLoaded, palette, maxTokens, weeksData]);

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
      <LoadingContainer ref={containerRef} style={{ aspectRatio: `${ISO_CANVAS_WIDTH} / ${ISO_CANVAS_HEIGHT}` }}>
        <LoadingText>Loading 3D view...</LoadingText>
      </LoadingContainer>
    );
  }

  return (
    <Container ref={containerRef}>
      <Stage>
        <StyledCanvas
          ref={canvasRef}
          onMouseMove={handleMouseMove}
          onMouseLeave={() => onDayHover(null, null)}
          onClick={handleClick}
          style={{ aspectRatio: `${ISO_CANVAS_WIDTH} / ${ISO_CANVAS_HEIGHT}` }}
        />

        <TopRightStats>
          <StatsTitle>Token usage</StatsTitle>
          <StatsBox>
            <StatItem>
              <StatValue $emphasis>{formatCurrency(totalCost)}</StatValue>
              <StatLabel>Total</StatLabel>
              <StatSubtext>{dateRange.start} → {dateRange.end}</StatSubtext>
            </StatItem>
            <TokensStatItem>
              <StatValue>{formatTokenCount(totalTokens)}</StatValue>
              <StatLabel>Tokens</StatLabel>
              <StatSubtext>{activeDays} active days</StatSubtext>
            </TokensStatItem>
            {bestDay && (
              <StatItem>
                <StatValue>{formatCurrency(bestDay.totals.cost)}</StatValue>
                <StatLabel>Best day</StatLabel>
                <StatSubtext>{formatContributionDate(bestDay).split(",")[0]}</StatSubtext>
              </StatItem>
            )}
          </StatsBox>
          <StatsFooter>
            Average: <SpanBold>{formatCurrency(activeDays > 0 ? totalCost / activeDays : 0)}</SpanBold> / day
          </StatsFooter>
        </TopRightStats>

        <BottomLeftStats>
          <StatsTitle>Streaks</StatsTitle>
          <StatsBox>
            <StatItem>
              <StatValue>{longestStreak} <SpanBase>days</SpanBase></StatValue>
              <StatLabel>Longest</StatLabel>
            </StatItem>
            <StatItem>
              <StatValue>{currentStreak} <SpanBase>days</SpanBase></StatValue>
              <StatLabel>Current</StatLabel>
            </StatItem>
          </StatsBox>
        </BottomLeftStats>
      </Stage>
    </Container>
  );
}
