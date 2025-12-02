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
      onSourceFilterChange(sourceFilter.filter((s) => s !== source));
    } else {
      onSourceFilterChange([...sourceFilter, source]);
    }
  };

  return (
    <div className="relative mb-4">
      <div className="float-right mt-1 ml-4 relative top-0 flex">
        <button
          onClick={() => onViewChange("2d")}
          className="px-3 py-1.5 text-xs font-semibold rounded-l-full border transition-all duration-200"
          style={{
            backgroundColor: view === "2d" ? palette.grade3 : "var(--color-btn-bg)",
            color: view === "2d" ? "#fff" : "var(--color-fg-default)",
            borderColor: view === "2d" ? palette.grade3 : "var(--color-border-default)",
          }}
        >
          2D
        </button>
        <button
          onClick={() => onViewChange("3d")}
          className="px-3 py-1.5 text-xs font-semibold rounded-r-full border-t border-b border-r transition-all duration-200"
          style={{
            backgroundColor: view === "3d" ? palette.grade3 : "var(--color-btn-bg)",
            color: view === "3d" ? "#fff" : "var(--color-fg-default)",
            borderColor: view === "3d" ? palette.grade3 : "var(--color-border-default)",
          }}
        >
          3D
        </button>
      </div>

      <div className="float-right mt-1 ml-3">
        <select
          value={paletteName}
          onChange={(e) => onPaletteChange(e.target.value as ColorPaletteName)}
          className="text-xs py-1.5 px-2 rounded-lg border cursor-pointer font-medium transition-all duration-200 hover:border-gray-400"
          style={{
            borderColor: "var(--color-border-default)",
            color: "var(--color-fg-default)",
            backgroundColor: "var(--color-btn-bg)",
          }}
        >
          {paletteNames.map((name) => (
            <option key={name} value={name}>{colorPalettes[name].name}</option>
          ))}
        </select>
      </div>

      <h2 className="text-lg font-medium mb-3" style={{ color: "var(--color-fg-default)" }}>
        <span className="font-bold" style={{ color: palette.grade4 }}>{totalContributions.toLocaleString()}</span>
        {" "}token usage entries
        {selectedYear && (
          <>
            {" "}in{" "}
            {availableYears.length > 1 ? (
              <select
                value={selectedYear}
                onChange={(e) => onYearChange(e.target.value)}
                className="font-bold border-none cursor-pointer underline decoration-dotted decoration-2 underline-offset-4"
                style={{ color: "var(--color-fg-default)", backgroundColor: "transparent" }}
              >
                {availableYears.map((year) => (
                  <option key={year} value={year} style={{ backgroundColor: "var(--color-canvas-default)" }}>{year}</option>
                ))}
              </select>
            ) : (
              <span className="font-bold">{selectedYear}</span>
            )}
          </>
        )}
      </h2>

      <div className="clear-both" />

      <div className="flex flex-wrap items-center justify-between gap-3 mt-3">
        {availableSources.length > 1 && (
          <div className="flex items-center gap-2 flex-wrap">
            <span className="text-xs font-semibold" style={{ color: "var(--color-fg-muted)" }}>Filter:</span>
            {availableSources.map((source) => {
              const isSelected = sourceFilter.length === 0 || sourceFilter.includes(source);
              return (
                <button
                  key={source}
                  onClick={() => handleSourceToggle(source)}
                  className={`px-3 py-1 text-xs rounded-full transition-all duration-200 hover:scale-105 ${isSelected ? "font-semibold" : "opacity-50"}`}
                  style={{
                    backgroundColor: isSelected ? `${palette.grade3}30` : "transparent",
                    color: "var(--color-fg-default)",
                    border: `1.5px solid ${isSelected ? palette.grade3 : "var(--color-border-default)"}`,
                  }}
                >
                  {SOURCE_DISPLAY_NAMES[source] || source}
                </button>
              );
            })}
            {sourceFilter.length > 0 && sourceFilter.length < availableSources.length && (
              <button
                onClick={() => onSourceFilterChange([...availableSources])}
                className="px-3 py-1 text-xs font-medium rounded-full hover:bg-gray-100 dark:hover:bg-gray-800 transition-colors"
                style={{ color: "var(--color-fg-muted)" }}
              >
                Show all
              </button>
            )}
            {sourceFilter.length === availableSources.length && (
              <button
                onClick={() => onSourceFilterChange([])}
                className="px-3 py-1 text-xs font-medium rounded-full hover:bg-gray-100 dark:hover:bg-gray-800 transition-colors"
                style={{ color: "var(--color-fg-muted)" }}
              >
                Clear
              </button>
            )}
          </div>
        )}

        <div className="flex items-center gap-2 ml-auto">
          <span className="text-xs font-medium" style={{ color: "var(--color-fg-muted)" }}>Less</span>
          {[0, 1, 2, 3, 4].map((level) => (
            <div
              key={level}
              className="w-3 h-3 rounded-md transition-transform hover:scale-110"
              style={{ backgroundColor: palette[`grade${level}` as keyof GraphColorPalette] as string }}
            />
          ))}
          <span className="text-xs font-medium" style={{ color: "var(--color-fg-muted)" }}>More</span>
        </div>
      </div>
    </div>
  );
}
