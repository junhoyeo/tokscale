"use client";

import { useState, useMemo, useCallback, useEffect, useRef } from "react";
import styled from "styled-components";
import dynamic from "next/dynamic";
import type { TokenContributionData, DailyContribution, ViewMode, ClientType, TooltipPosition } from "@/lib/types";
import { getPalette } from "@/lib/themes";
import { useSettings } from "@/lib/useSettings";
import { filterByClient, filterByYear, recalculateIntensity, findBestDay, calculateCurrentStreak, calculateLongestStreak } from "@/lib/utils";
import { TokenGraph2D } from "./TokenGraph2D";

// Lazy load 3D graph (Three.js) - reduces initial bundle, SSR disabled for WebGL
const TokenGraph3D = dynamic(() => import("./TokenGraph3D").then((mod) => mod.TokenGraph3D), {
  ssr: false,
  loading: () => <Graph3DPlaceholder>Loading 3D view...</Graph3DPlaceholder>,
});
import { GraphControls } from "./GraphControls";
import { Tooltip } from "./Tooltip";
import { BreakdownPanel } from "./BreakdownPanel";
import { StatsPanel } from "./StatsPanel";

const Container = styled.div`
  display: flex;
  flex-direction: column;
  gap: 24px;
`;

const Graph3DPlaceholder = styled.div`
  display: flex;
  align-items: center;
  justify-content: center;
  height: 400px;
  color: var(--color-fg-muted);
  font-size: 14px;
`;

const GraphCard = styled.div`
  border-radius: 16px;
  border: 1px solid var(--color-border-default);
  padding-top: 16px;
  padding-bottom: 16px;
  overflow: hidden;
  box-shadow: 0 1px 2px 0 rgb(0 0 0 / 0.05);
  transition: box-shadow 200ms;
  background-color: var(--color-graph-canvas);

  &:hover {
    box-shadow: 0 4px 6px -1px rgb(0 0 0 / 0.1), 0 2px 4px -2px rgb(0 0 0 / 0.1);
  }
`;

const ControlsWrapper = styled.div`
  padding-left: 20px;
  padding-right: 20px;
`;

const GraphWrapper = styled.div`
  padding-left: 20px;
  padding-right: 20px;
  padding-bottom: 12px;
`;

interface GraphContainerProps {
  data: TokenContributionData;
  totalActiveTimeMs?: number | null;
  sessionCount?: number | null;
  mcpServers?: string[];
}

export function GraphContainer({ data, totalActiveTimeMs, sessionCount, mcpServers }: GraphContainerProps) {
  const { paletteName, setPalette } = useSettings();

  const [view, setView] = useState<ViewMode>("2d");
  const [selectedYear, setSelectedYear] = useState<string>(() => data.years.length > 0 ? data.years[data.years.length - 1].year : "");
  const [hoveredDay, setHoveredDay] = useState<DailyContribution | null>(null);
  const [tooltipPosition, setTooltipPosition] = useState<TooltipPosition | null>(null);
  const [selectedDay, setSelectedDay] = useState<DailyContribution | null>(null);
  const [clientFilter, setClientFilter] = useState<ClientType[]>([]);
  const initializedRef = useRef(false);

  const palette = useMemo(() => getPalette(paletteName), [paletteName]);
  const availableYears = useMemo(() => data.years.map((y) => y.year), [data.years]);
  const availableClients = useMemo(() => data.summary.clients, [data.summary.clients]);

  const filteredByClient = useMemo(() => {
    if (clientFilter.length === 0) return data;
    return filterByClient(data, clientFilter);
  }, [data, clientFilter]);

  const yearContributions = useMemo(() => {
    const filtered = filterByYear(filteredByClient.contributions, selectedYear);
    return recalculateIntensity(filtered);
  }, [filteredByClient.contributions, selectedYear]);

  const maxTokens = useMemo(() => Math.max(...yearContributions.map((c) => c.totals.tokens), 0), [yearContributions]);
  const totalCost = useMemo(() => yearContributions.reduce((sum, c) => sum + c.totals.cost, 0), [yearContributions]);
  const totalTokens = useMemo(() => yearContributions.reduce((sum, c) => sum + c.totals.tokens, 0), [yearContributions]);
  const activeDays = useMemo(() => yearContributions.filter((c) => c.totals.tokens > 0).length, [yearContributions]);
  const bestDay = useMemo(() => findBestDay(yearContributions), [yearContributions]);
  const currentStreak = useMemo(() => calculateCurrentStreak(yearContributions), [yearContributions]);
  const longestStreak = useMemo(() => calculateLongestStreak(yearContributions), [yearContributions]);

  const dateRange = useMemo(() => {
    if (yearContributions.length === 0) return { start: "", end: "" };
    const dates = yearContributions.filter((c) => c.totals.tokens > 0).map((c) => c.date).sort();
    return {
      start: dates[0]?.split("-").slice(1).join("/") || "",
      end: dates[dates.length - 1]?.split("-").slice(1).join("/") || "",
    };
  }, [yearContributions]);


  useEffect(() => {
    if (!initializedRef.current && yearContributions.length > 0) {
      const activeDaysWithTokens = yearContributions.filter((c) => c.totals.tokens > 0);
      if (activeDaysWithTokens.length > 0) {
        const latestDay = activeDaysWithTokens[activeDaysWithTokens.length - 1];
        // Intentional one-time initialization on first data load
        // eslint-disable-next-line react-hooks/set-state-in-effect
        setSelectedDay(latestDay);
        initializedRef.current = true;
      }
    }
  }, [yearContributions]);

  const handleDayHover = useCallback((day: DailyContribution | null, position: TooltipPosition | null) => {
    setHoveredDay(day);
    setTooltipPosition(position);
  }, []);

  const handleDayClick = useCallback((day: DailyContribution | null) => {
    setSelectedDay((prev) => (prev?.date === day?.date ? null : day));
  }, []);

  return (
    <Container>
      <GraphCard>
        <ControlsWrapper>
          <GraphControls
            view={view}
            onViewChange={setView}
            paletteName={paletteName}
            onPaletteChange={setPalette}
            selectedYear={selectedYear}
            availableYears={availableYears}
            onYearChange={setSelectedYear}
            clientFilter={clientFilter}
            availableClients={availableClients}
            onClientFilterChange={setClientFilter}
            palette={palette}
            totalTokens={totalTokens}
          />
        </ControlsWrapper>

        <GraphWrapper>
          {view === "2d" ? (
            <TokenGraph2D
              contributions={yearContributions}
              palette={palette}
              year={selectedYear}
              onDayHover={handleDayHover}
              onDayClick={handleDayClick}
            />
          ) : (
            <TokenGraph3D
              contributions={yearContributions}
              palette={palette}
              year={selectedYear}
              maxTokens={maxTokens}
              totalCost={totalCost}
              totalTokens={totalTokens}
              activeDays={activeDays}
              bestDay={bestDay}
              currentStreak={currentStreak}
              longestStreak={longestStreak}
              dateRange={dateRange}
              onDayHover={handleDayHover}
              onDayClick={handleDayClick}
            />
          )}
        </GraphWrapper>
      </GraphCard>

      {selectedDay && <BreakdownPanel day={selectedDay} onClose={() => setSelectedDay(null)} palette={palette} />}
      {view === "2d" && <StatsPanel data={filteredByClient} palette={palette} totalActiveTimeMs={clientFilter.length === 0 ? totalActiveTimeMs : null} sessionCount={clientFilter.length === 0 ? sessionCount : null} mcpServers={mcpServers} />}
      <Tooltip day={hoveredDay} position={tooltipPosition} visible={hoveredDay !== null} palette={palette} />
    </Container>
  );
}
