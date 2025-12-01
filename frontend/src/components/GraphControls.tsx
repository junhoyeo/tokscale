"use client";

import type { ViewMode, ColorPaletteName, SourceType, GraphColorPalette } from "@/lib/types";
import { getPaletteNames, colorPalettes } from "@/lib/themes";
import { SOURCE_DISPLAY_NAMES } from "@/lib/constants";

interface GraphControlsProps {
  view: ViewMode;
  onViewChange: (view: ViewMode) => void;
  paletteName: ColorPaletteName;
  onPaletteChange: (palette: ColorPaletteName) => void;
  selectedYear: string;
  availableYears: string[];
  onYearChange: (year: string) => void;
  sourceFilter: SourceType[];
  availableSources: SourceType[];
  onSourceFilterChange: (sources: SourceType[]) => void;
  palette: GraphColorPalette;
  totalContributions: number;
}

export function GraphControls({
  view,
  onViewChange,
  paletteName,
  onPaletteChange,
  selectedYear,
  availableYears,
  onYearChange,
  sourceFilter,
  availableSources,
  onSourceFilterChange,
  palette,
  totalContributions,
}: GraphControlsProps) {
  const paletteNames = getPaletteNames();

  const handleSourceToggle = (source: SourceType) => {
    if (sourceFilter.includes(source)) {
      const newFilter = sourceFilter.filter((s) => s !== source);
      onSourceFilterChange(newFilter);
    } else {
      onSourceFilterChange([...sourceFilter, source]);
    }
  };

  const handleSelectAllSources = () => {
    onSourceFilterChange([...availableSources]);
  };

  const handleClearSources = () => {
    onSourceFilterChange([]);
  };

  return (
    <div className="relative mb-2">
      {/* GitHub-style 2D/3D Toggle - Top Right */}
      <div className="float-right mt-1 ml-3 relative top-0 flex">
        <button
          onClick={() => onViewChange("2d")}
          className="px-2 py-0.5 text-xs font-medium rounded-l border transition-colors"
          style={{
            backgroundColor: view === "2d" ? palette.grade3 : "var(--color-btn-bg)",
            color: view === "2d" ? "#fff" : "var(--color-fg-default)",
            borderColor: "var(--color-border-default)",
          }}
        >
          2D
        </button>
        <button
          onClick={() => onViewChange("3d")}
          className="px-2 py-0.5 text-xs font-medium rounded-r border-t border-b border-r transition-colors"
          style={{
            backgroundColor: view === "3d" ? palette.grade3 : "var(--color-btn-bg)",
            color: view === "3d" ? "#fff" : "var(--color-fg-default)",
            borderColor: "var(--color-border-default)",
          }}
        >
          3D
        </button>
      </div>

      {/* Palette dropdown - floating right */}
      <div className="float-right mt-1 ml-2">
        <select
          value={paletteName}
          onChange={(e) => onPaletteChange(e.target.value as ColorPaletteName)}
          className="text-xs py-0.5 px-1 rounded border cursor-pointer"
          style={{
            borderColor: "var(--color-border-default)",
            color: "var(--color-fg-default)",
            backgroundColor: "var(--color-btn-bg)",
          }}
        >
          {paletteNames.map((name) => (
            <option key={name} value={name}>
              {colorPalettes[name].name}
            </option>
          ))}
        </select>
      </div>

      {/* Main Title - GitHub style */}
      <h2 className="text-base font-normal mb-2" style={{ color: "var(--color-fg-default)" }}>
        <span className="font-semibold" style={{ color: palette.grade4 }}>
          {totalContributions.toLocaleString()}
        </span>
        {" "}token usage entries
        {selectedYear && (
          <>
            {" "}in{" "}
            {availableYears.length > 1 ? (
              <select
                value={selectedYear}
                onChange={(e) => onYearChange(e.target.value)}
                className="font-semibold border-none cursor-pointer underline decoration-dotted"
                style={{ 
                  color: "var(--color-fg-default)",
                  backgroundColor: "transparent",
                }}
              >
                {availableYears.map((year) => (
                  <option key={year} value={year} style={{ backgroundColor: "var(--color-canvas-default)" }}>
                    {year}
                  </option>
                ))}
              </select>
            ) : (
              <span className="font-semibold">{selectedYear}</span>
            )}
          </>
        )}
      </h2>

      <div className="clear-both" />

      {/* Source Filter Pills & Legend */}
      <div className="flex flex-wrap items-center justify-between gap-2 mt-2">
        {/* Source Filter */}
        {availableSources.length > 1 && (
          <div className="flex items-center gap-1.5 flex-wrap">
            <span className="text-xs" style={{ color: "var(--color-fg-muted)" }}>
              Filter:
            </span>
            {availableSources.map((source) => {
              const isSelected = sourceFilter.length === 0 || sourceFilter.includes(source);
              return (
                <button
                  key={source}
                  onClick={() => handleSourceToggle(source)}
                  className={`px-2 py-0.5 text-xs rounded-full transition-colors ${
                    isSelected ? "font-medium" : "opacity-50"
                  }`}
                  style={{
                    backgroundColor: isSelected ? `${palette.grade3}30` : "transparent",
                    color: "var(--color-fg-default)",
                    border: `1px solid ${isSelected ? palette.grade3 : "var(--color-border-default)"}`,
                  }}
                >
                  {SOURCE_DISPLAY_NAMES[source] || source}
                </button>
              );
            })}
            {sourceFilter.length > 0 && sourceFilter.length < availableSources.length && (
              <button
                onClick={handleSelectAllSources}
                className="px-2 py-0.5 text-xs"
                style={{ color: "var(--color-fg-muted)", textDecoration: "underline" }}
              >
                Show all
              </button>
            )}
            {sourceFilter.length === availableSources.length && (
              <button
                onClick={handleClearSources}
                className="px-2 py-0.5 text-xs"
                style={{ color: "var(--color-fg-muted)", textDecoration: "underline" }}
              >
                Clear
              </button>
            )}
          </div>
        )}

        {/* Legend */}
        <div className="flex items-center gap-1.5 ml-auto">
          <span className="text-xs" style={{ color: "var(--color-fg-muted)" }}>
            Less
          </span>
          {[0, 1, 2, 3, 4].map((level) => (
            <div
              key={level}
              className="w-2.5 h-2.5 rounded-sm"
              style={{
                backgroundColor: palette[`grade${level}` as keyof GraphColorPalette] as string,
              }}
            />
          ))}
          <span className="text-xs" style={{ color: "var(--color-fg-muted)" }}>
            More
          </span>
        </div>
      </div>
    </div>
  );
}
