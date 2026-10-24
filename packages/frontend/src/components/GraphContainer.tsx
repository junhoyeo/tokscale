"use client";

import { useState, useMemo, useCallback, useId } from "react";
import styled from "styled-components";
import dynamic from "next/dynamic";
import type { TokenContributionData, DailyContribution, ViewMode, ClientType, TooltipPosition } from "@/lib/types";
import { getPalette } from "@/lib/themes";
import { useSettings } from "@/lib/useSettings";
import { filterByClient, filterByYear, recalculateIntensity, findBestDay, calculateCurrentStreak, calculateLongestStreak, dayHasActivity, resolveSelectedDay } from "@/lib/utils";
import { TokenGraph2D } from "./TokenGraph2D";

// Lazy load 3D graph (Three.js) - reduces initial bundle, SSR disabled for WebGL
const TokenGraph3D = dynamic(() => import("./TokenGraph3D").then((mod) => mod.TokenGraph3D), {
  ssr: false,
  loading: () => <Graph3DPlaceholder>Loading 3D view...</Graph3DPlaceholder>,
});
import { GraphControls, GraphPaletteLegend } from "./GraphControls";
import { Tooltip } from "./Tooltip";
import { BreakdownPanel } from "./BreakdownPanel";
import { StatsPanel } from "./StatsPanel";

const Container = styled.div`
  display: flex;
  min-width: 0;
  flex-direction: column;
  gap: 16px;
`;

const Graph3DPlaceholder = styled.div`
  display: flex;
  align-items: center;
  justify-content: center;
  width: 100%;
  aspect-ratio: 5 / 3;
  color: var(--service-text-muted);
  font-size: 14px;
`;

const GraphCard = styled.section`
  min-width: 0;
  overflow: hidden;
  border: 1px solid var(--service-border);
  border-radius: 12px;
  background: var(--service-surface);
`;

const ControlsWrapper = styled.div`
  padding: 16px 20px 0;

  @media (max-width: 520px) {
    padding: 14px 14px 0;
  }
`;

const GraphWrapper = styled.div`
  min-width: 0;
  padding: 16px 20px;

  @media (max-width: 520px) {
    padding: 14px;
  }
`;

const FooterWrapper = styled.div`
  padding: 12px 20px;
  border-top: 1px solid var(--service-border);

  @media (max-width: 520px) {
    padding: 12px 14px;
  }
`;

interface GraphContainerProps {
  data: TokenContributionData;
  totalActiveTimeMs?: number | null;
  sessionCount?: number | null;
  mcpServers?: string[];
}

export function GraphContainer({ data, totalActiveTimeMs, sessionCount, mcpServers }: GraphContainerProps) {
  const { paletteName, setPalette } = useSettings();
  const titleId = useId();

  const [view, setView] = useState<ViewMode>("2d");
  const [selectedYear, setSelectedYear] = useState<string>(() =>
    data.years.length > 0 ? data.years[data.years.length - 1].year : String(new Date().getFullYear())
  );
  const [hoveredDay, setHoveredDay] = useState<DailyContribution | null>(null);
  const [tooltipPosition, setTooltipPosition] = useState<TooltipPosition | null>(null);
  // LocalClient mounts this component only after loading an export. Initialize the selection
  // with that data, in the same render as the graph/summary, regardless of input array order.
  const [selectedDate, setSelectedDate] = useState<string | null>(() => {
    let latest: string | null = null;
    for (const day of data.contributions) {
      if (day.date.startsWith(selectedYear) && dayHasActivity(day) && (!latest || day.date > latest)) latest = day.date;
    }
    return latest;
  });
  const [clientFilter, setClientFilter] = useState<ClientType[]>([]);

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

  // Derive the breakdown panel's day from the live contributions so it re-resolves
  // (and closes when the date drops out of the filtered data) whenever the client
  // filter or selected year changes, instead of showing a stale pre-filter snapshot.
  const selectedDay = useMemo(
    () => resolveSelectedDay(selectedDate, yearContributions),
    [selectedDate, yearContributions]
  );

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


  const handleDayHover = useCallback((day: DailyContribution | null, position: TooltipPosition | null) => {
    setHoveredDay(day);
    setTooltipPosition(position);
  }, []);

  const handleDayClick = useCallback((day: DailyContribution | null) => {
    setSelectedDate((prev) => (prev === day?.date ? null : day?.date ?? null));
  }, []);

  return (
    <Container>
      <GraphCard aria-labelledby={titleId}>
        <ControlsWrapper>
          <GraphControls
            titleId={titleId}
            view={view}
            onViewChange={setView}
            selectedYear={selectedYear}
            availableYears={availableYears}
            onYearChange={setSelectedYear}
            clientFilter={clientFilter}
            availableClients={availableClients}
            onClientFilterChange={setClientFilter}
            totalTokens={totalTokens}
          />
        </ControlsWrapper>

        <GraphWrapper>
          {view === "2d" ? (
            <TokenGraph2D
              contributions={yearContributions}
              palette={palette}
              year={selectedYear}
              hoveredDate={hoveredDay?.date ?? null}
              selectedDate={selectedDay?.date ?? null}
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

        <FooterWrapper>
          <GraphPaletteLegend paletteName={paletteName} onPaletteChange={setPalette} palette={palette} />
        </FooterWrapper>
      </GraphCard>

      {selectedDay && <BreakdownPanel day={selectedDay} onClose={() => setSelectedDate(null)} />}
      {view === "2d" && <StatsPanel data={filteredByClient} totalActiveTimeMs={clientFilter.length === 0 ? totalActiveTimeMs : null} sessionCount={clientFilter.length === 0 ? sessionCount : null} mcpServers={mcpServers} />}
      <Tooltip day={hoveredDay} position={tooltipPosition} visible={hoveredDay !== null} />
    </Container>
  );
}
