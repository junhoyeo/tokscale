"use client";

import { useState, useMemo, useCallback, useEffect, useRef } from "react";
import type { TokenContributionData, DailyContribution, ViewMode, SourceType, TooltipPosition } from "@/lib/types";
import { getPalette } from "@/lib/themes";
import { useSettings } from "@/lib/useSettings";
import { filterBySource, filterByYear, recalculateIntensity, findBestDay, calculateCurrentStreak, calculateLongestStreak } from "@/lib/utils";
import { TokenGraph2D } from "./TokenGraph2D";
import { TokenGraph3D } from "./TokenGraph3D";
import { GraphControls } from "./GraphControls";
import { Tooltip } from "./Tooltip";
import { BreakdownPanel } from "./BreakdownPanel";
import { StatsPanel } from "./StatsPanel";

interface GraphContainerProps {
  data: TokenContributionData;
}

export function GraphContainer({ data }: GraphContainerProps) {
  const { paletteName, setPalette } = useSettings();

  const [view, setView] = useState<ViewMode>("2d");
  const [selectedYear, setSelectedYear] = useState<string>(() => data.years.length > 0 ? data.years[data.years.length - 1].year : "");
  const [hoveredDay, setHoveredDay] = useState<DailyContribution | null>(null);
  const [tooltipPosition, setTooltipPosition] = useState<TooltipPosition | null>(null);
  const [selectedDay, setSelectedDay] = useState<DailyContribution | null>(null);
  const [sourceFilter, setSourceFilter] = useState<SourceType[]>([]);
  const initializedRef = useRef(false);

  const palette = useMemo(() => getPalette(paletteName), [paletteName]);
  const availableYears = useMemo(() => data.years.map((y) => y.year), [data.years]);
  const availableSources = useMemo(() => data.summary.sources, [data.summary.sources]);

  const filteredBySource = useMemo(() => {
    if (sourceFilter.length === 0) return data;
    return filterBySource(data, sourceFilter);
  }, [data, sourceFilter]);

  const yearContributions = useMemo(() => {
    const filtered = filterByYear(filteredBySource.contributions, selectedYear);
    return recalculateIntensity(filtered);
  }, [filteredBySource.contributions, selectedYear]);

  const maxCost = useMemo(() => Math.max(...yearContributions.map((c) => c.totals.cost), 0), [yearContributions]);
  const totalCost = useMemo(() => yearContributions.reduce((sum, c) => sum + c.totals.cost, 0), [yearContributions]);
  const totalTokens = useMemo(() => yearContributions.reduce((sum, c) => sum + c.totals.tokens, 0), [yearContributions]);
  const activeDays = useMemo(() => yearContributions.filter((c) => c.totals.cost > 0).length, [yearContributions]);
  const bestDay = useMemo(() => findBestDay(yearContributions), [yearContributions]);
  const currentStreak = useMemo(() => calculateCurrentStreak(yearContributions), [yearContributions]);
  const longestStreak = useMemo(() => calculateLongestStreak(yearContributions), [yearContributions]);

  const dateRange = useMemo(() => {
    if (yearContributions.length === 0) return { start: "", end: "" };
    const dates = yearContributions.filter((c) => c.totals.cost > 0).map((c) => c.date).sort();
    return {
      start: dates[0]?.split("-").slice(1).join("/") || "",
      end: dates[dates.length - 1]?.split("-").slice(1).join("/") || "",
    };
  }, [yearContributions]);

  const totalContributions = useMemo(() => yearContributions.reduce((sum, c) => sum + c.totals.messages, 0), [yearContributions]);

  useEffect(() => {
    if (!initializedRef.current && yearContributions.length > 0) {
      const activeDaysWithCost = yearContributions.filter((c) => c.totals.cost > 0);
      if (activeDaysWithCost.length > 0) {
        const latestDay = activeDaysWithCost[activeDaysWithCost.length - 1];
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
    <div className="space-y-6">
      <div
        className="rounded-2xl border py-4 overflow-hidden shadow-sm transition-shadow hover:shadow-md"
        style={{ backgroundColor: "#141415", borderColor: "#262627" }}
      >
        <div className="px-5">
          <GraphControls
            view={view}
            onViewChange={setView}
            paletteName={paletteName}
            onPaletteChange={setPalette}
            selectedYear={selectedYear}
            availableYears={availableYears}
            onYearChange={setSelectedYear}
            sourceFilter={sourceFilter}
            availableSources={availableSources}
            onSourceFilterChange={setSourceFilter}
            palette={palette}
            totalContributions={totalContributions}
          />
        </div>

        <div className="px-5 pb-3">
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
              maxCost={maxCost}
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
        </div>
      </div>

      {selectedDay && <BreakdownPanel day={selectedDay} onClose={() => setSelectedDay(null)} palette={palette} />}
      {view === "2d" && <StatsPanel data={filteredBySource} palette={palette} />}
      <Tooltip day={hoveredDay} position={tooltipPosition} visible={hoveredDay !== null} palette={palette} />
    </div>
  );
}
