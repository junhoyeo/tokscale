"use client";

import { useState, useMemo, useCallback } from "react";
import type {
  TokenContributionData,
  DailyContribution,
  ViewMode,
  ColorPaletteName,
  SourceType,
  TooltipPosition,
} from "@/lib/types";
import { getPalette, DEFAULT_PALETTE } from "@/lib/themes";
import {
  filterBySource,
  filterByYear,
  recalculateIntensity,
  findBestDay,
  calculateCurrentStreak,
  calculateLongestStreak,
} from "@/lib/utils";
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
  // State
  const [view, setView] = useState<ViewMode>("2d");
  const [paletteName, setPaletteName] = useState<ColorPaletteName>(DEFAULT_PALETTE);
  const [selectedYear, setSelectedYear] = useState<string>(() => {
    // Default to most recent year
    return data.years.length > 0 ? data.years[data.years.length - 1].year : "";
  });
  const [hoveredDay, setHoveredDay] = useState<DailyContribution | null>(null);
  const [tooltipPosition, setTooltipPosition] = useState<TooltipPosition | null>(null);
  const [selectedDay, setSelectedDay] = useState<DailyContribution | null>(null);
  const [sourceFilter, setSourceFilter] = useState<SourceType[]>([]);

  // Get color palette for graph cells
  const palette = useMemo(() => getPalette(paletteName), [paletteName]);

  // Get available years
  const availableYears = useMemo(() => data.years.map((y) => y.year), [data.years]);

  // Get available sources
  const availableSources = useMemo(() => data.summary.sources, [data.summary.sources]);

  // Filter data by source
  const filteredBySource = useMemo(() => {
    if (sourceFilter.length === 0) return data;
    return filterBySource(data, sourceFilter);
  }, [data, sourceFilter]);

  // Filter contributions by year
  const yearContributions = useMemo(() => {
    const filtered = filterByYear(filteredBySource.contributions, selectedYear);
    return recalculateIntensity(filtered);
  }, [filteredBySource.contributions, selectedYear]);

  // Calculate stats
  const maxCost = useMemo(() => {
    return Math.max(...yearContributions.map((c) => c.totals.cost), 0);
  }, [yearContributions]);

  const totalCost = useMemo(() => {
    return yearContributions.reduce((sum, c) => sum + c.totals.cost, 0);
  }, [yearContributions]);

  const totalTokens = useMemo(() => {
    return yearContributions.reduce((sum, c) => sum + c.totals.tokens, 0);
  }, [yearContributions]);

  const activeDays = useMemo(() => {
    return yearContributions.filter((c) => c.totals.cost > 0).length;
  }, [yearContributions]);

  const bestDay = useMemo(() => {
    return findBestDay(yearContributions);
  }, [yearContributions]);

  const currentStreak = useMemo(() => {
    return calculateCurrentStreak(yearContributions);
  }, [yearContributions]);

  const longestStreak = useMemo(() => {
    return calculateLongestStreak(yearContributions);
  }, [yearContributions]);

  const dateRange = useMemo(() => {
    if (yearContributions.length === 0) return { start: "", end: "" };
    const dates = yearContributions.filter(c => c.totals.cost > 0).map((c) => c.date).sort();
    return {
      start: dates[0]?.split("-").slice(1).join("/") || "",
      end: dates[dates.length - 1]?.split("-").slice(1).join("/") || "",
    };
  }, [yearContributions]);

  const totalContributions = useMemo(() => {
    return yearContributions.reduce((sum, c) => sum + c.totals.messages, 0);
  }, [yearContributions]);

  // Handlers
  const handleDayHover = useCallback(
    (day: DailyContribution | null, position: TooltipPosition | null) => {
      setHoveredDay(day);
      setTooltipPosition(position);
    },
    []
  );

  const handleDayClick = useCallback((day: DailyContribution | null) => {
    setSelectedDay((prev) => (prev?.date === day?.date ? null : day));
  }, []);

  const handleCloseBreakdown = useCallback(() => {
    setSelectedDay(null);
  }, []);

  return (
    <div className="space-y-4">
      {/* Graph with Controls */}
      <div
        className="rounded-lg border py-2 overflow-hidden"
        style={{
          backgroundColor: "var(--color-canvas-default)",
          borderColor: "var(--color-border-default)",
        }}
      >
        {/* Controls inside the graph container - GitHub style */}
        <div className="px-4">
          <GraphControls
            view={view}
            onViewChange={setView}
            paletteName={paletteName}
            onPaletteChange={setPaletteName}
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

        {/* Graph */}
        <div className="px-4 pb-2">
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

      {/* Breakdown Panel (shown when day is selected) */}
      {selectedDay && (
        <BreakdownPanel
          day={selectedDay}
          onClose={handleCloseBreakdown}
          palette={palette}
        />
      )}

      {/* Stats Panel - only show in 2D mode since 3D has overlays */}
      {view === "2d" && <StatsPanel data={filteredBySource} palette={palette} />}

      {/* Tooltip (floating) */}
      <Tooltip
        day={hoveredDay}
        position={tooltipPosition}
        visible={hoveredDay !== null}
        palette={palette}
      />
    </div>
  );
}
