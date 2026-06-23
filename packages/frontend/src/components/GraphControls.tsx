"use client";

import styled from "styled-components";
import type { ViewMode, ColorPaletteName, ClientType, GraphColorPalette } from "@/lib/types";
import { getPaletteNames, colorPalettes } from "@/lib/themes";
import { SOURCE_DISPLAY_NAMES, SOURCE_LOGOS } from "@/lib/constants";
import { formatTokenCount } from "@/lib/utils";

interface GraphControlsProps {
  view: ViewMode;
  onViewChange: (view: ViewMode) => void;
  paletteName: ColorPaletteName;
  onPaletteChange: (palette: ColorPaletteName) => void;
  selectedYear: string;
  availableYears: string[];
  onYearChange: (year: string) => void;
  clientFilter: ClientType[];
  availableClients: ClientType[];
  onClientFilterChange: (clients: ClientType[]) => void;
  palette: GraphColorPalette;
  totalTokens: number;
}

const Container = styled.div`
  position: relative;
  margin-bottom: 16px;
`;

const ViewModeGroup = styled.div`
  float: right;
  margin-top: 4px;
  margin-left: 16px;
  position: relative;
  top: 0;
  display: flex;
`;

const ViewModeButton = styled.button<{ $isActive: boolean; $position: 'left' | 'right' }>`
  padding: 6px 12px;
  font-size: 12px;
  font-weight: 600;
  border: 1px solid;
  transition: all 200ms;
  
  ${({ $position }) => $position === 'left' 
    ? 'border-radius: 9999px 0 0 9999px;' 
    : 'border-radius: 0 9999px 9999px 0; border-left: none;'
  }
  
  &:focus-visible {
    outline: none;
    box-shadow: 0 0 0 2px #3b82f6;
    position: relative;
    z-index: 1;
  }
`;

const PaletteSelectContainer = styled.div`
  float: right;
  margin-top: 4px;
  margin-left: 12px;
`;

const PaletteSelect = styled.select`
  font-size: 12px;
  padding: 6px 8px;
  border-radius: 8px;
  border: 1px solid;
  cursor: pointer;
  font-weight: 500;
  transition: all 200ms;
  
  &:hover {
    border-color: #a3a3a3;
  }
  
  &:focus-visible {
    outline: none;
    box-shadow: 0 0 0 2px #3b82f6;
  }
`;

const Title = styled.h2`
  font-size: 18px;
  font-weight: 500;
  margin-bottom: 12px;
`;

const TitleBold = styled.span`
  font-weight: 700;
`;

const YearSelect = styled.select`
  font-weight: 700;
  border: none;
  cursor: pointer;
  text-decoration: underline;
  text-decoration-style: dotted;
  text-decoration-thickness: 2px;
  text-underline-offset: 4px;
  background-color: transparent;
  
  &:focus-visible {
    outline: none;
    box-shadow: 0 0 0 2px #3b82f6;
  }
`;

const YearOption = styled.option``;

const ClearBoth = styled.div`
  clear: both;
`;

const FiltersWrapper = styled.div`
  display: flex;
  flex-wrap: wrap;
  align-items: center;
  justify-content: space-between;
  gap: 12px;
  margin-top: 12px;

  @media (max-width: 560px) {
    flex-direction: column;
    align-items: stretch;
    justify-content: flex-start;
    gap: 8px;
  }
`;

const SourceFilterGroup = styled.div`
  display: flex;
  align-items: center;
  gap: 8px;
  flex-wrap: wrap;
  max-width: 100%;
  overflow-x: auto;
  overflow-y: hidden;
  -webkit-overflow-scrolling: touch;
  scrollbar-width: none;
  padding-bottom: 4px;

  &::-webkit-scrollbar {
    display: none;
  }

  @media (max-width: 560px) {
    flex-wrap: nowrap;
  }
`;

const FilterLabel = styled.span`
  font-size: 12px;
  font-weight: 600;
  flex-shrink: 0;
  white-space: nowrap;
`;

const SourceFilterButton = styled.button<{ $isSelected: boolean }>`
  display: flex;
  align-items: center;
  gap: 6px;
  padding: 4px 12px 4px 6px;
  font-size: 12px;
  border-radius: 9999px;
  transition: all 200ms;
  font-weight: ${({ $isSelected }) => $isSelected ? '600' : '400'};
  opacity: ${({ $isSelected }) => $isSelected ? '1' : '0.5'};
  border: 1.5px solid;
  flex: 0 0 auto;
  touch-action: manipulation;
  
  &:hover {
    transform: scale(1.05);
  }
  
  &:focus-visible {
    outline: none;
    box-shadow: 0 0 0 2px #3b82f6;
  }

  @media (max-width: 400px) {
    padding: 4px 10px 4px 4px;
  }
`;

const SourceLogo = styled.img`
  width: 20px;
  height: 20px;
  border-radius: 50%;
  object-fit: cover;
  flex-shrink: 0;
`;

const ActionButton = styled.button`
  padding: 4px 12px;
  font-size: 12px;
  font-weight: 500;
  border-radius: 9999px;
  transition: colors 200ms;
  border: none;
  background: transparent;
  
  &:hover {
    background-color: rgba(0, 0, 0, 0.05);
  }
  
  @media (prefers-color-scheme: dark) {
    &:hover {
      background-color: rgba(255, 255, 255, 0.1);
    }
  }
  
  &:focus-visible {
    outline: none;
    box-shadow: 0 0 0 2px #3b82f6;
  }
`;

const LegendContainer = styled.div`
  display: flex;
  align-items: center;
  gap: 8px;
  margin-left: auto;

  @media (max-width: 560px) {
    margin-left: 0;
    width: 100%;
    justify-content: flex-end;
    min-width: 0;
  }
`;

const LegendText = styled.span`
  font-size: 12px;
  font-weight: 500;
`;

const LegendBox = styled.div`
  width: 12px;
  height: 12px;
  border-radius: 6px;
  transition: transform 200ms;
  
  &:hover {
    transform: scale(1.1);
  }
`;

export function GraphControls({
  view,
  onViewChange,
  paletteName,
  onPaletteChange,
  selectedYear,
  availableYears,
  onYearChange,
  clientFilter,
  availableClients,
  onClientFilterChange,
  palette,
  totalTokens,
}: GraphControlsProps) {
  const paletteNames = getPaletteNames();

  const handleClientToggle = (client: ClientType) => {
    if (clientFilter.includes(client)) {
      onClientFilterChange(clientFilter.filter((c) => c !== client));
    } else {
      onClientFilterChange([...clientFilter, client]);
    }
  };

  return (
    <Container>
      <ViewModeGroup role="group" aria-label="View mode">
        <ViewModeButton
          $isActive={view === "2d"}
          $position="left"
          onClick={() => onViewChange("2d")}
          aria-pressed={view === "2d"}
          aria-label="2D view"
          style={{
            backgroundColor: view === "2d" ? palette.grade3 : "var(--color-bg-button)",
            color: view === "2d" ? "#fff" : "var(--color-fg-default)",
            borderColor: view === "2d" ? palette.grade3 : "var(--color-border-default)",
          }}
        >
          2D
        </ViewModeButton>
        <ViewModeButton
          $isActive={view === "3d"}
          $position="right"
          onClick={() => onViewChange("3d")}
          aria-pressed={view === "3d"}
          aria-label="3D view"
          style={{
            backgroundColor: view === "3d" ? palette.grade3 : "var(--color-bg-button)",
            color: view === "3d" ? "#fff" : "var(--color-fg-default)",
            borderColor: view === "3d" ? palette.grade3 : "var(--color-border-default)",
          }}
        >
          3D
        </ViewModeButton>
      </ViewModeGroup>

      <PaletteSelectContainer>
        <PaletteSelect
          value={paletteName}
          onChange={(e) => onPaletteChange(e.target.value as ColorPaletteName)}
          aria-label="Color palette"
          style={{
            borderColor: "var(--color-border-default)",
            color: "var(--color-fg-default)",
            backgroundColor: "var(--color-bg-button)",
          }}
        >
          {paletteNames.map((name) => (
            <option key={name} value={name}>{colorPalettes[name].name}</option>
          ))}
        </PaletteSelect>
      </PaletteSelectContainer>

      <Title style={{ color: "var(--color-fg-default)" }}>
        <TitleBold style={{ color: palette.grade1 }}>{formatTokenCount(totalTokens)}</TitleBold>
        {" "}tokens used
        {selectedYear && (
          <>
            {" "}in{" "}
            {availableYears.length > 1 ? (
              <YearSelect
                value={selectedYear}
                onChange={(e) => onYearChange(e.target.value)}
                aria-label="Select year"
                style={{ color: "var(--color-fg-default)" }}
              >
                {availableYears.map((year) => (
                  <YearOption key={year} value={year} style={{ backgroundColor: "var(--color-canvas-default)" }}>{year}</YearOption>
                ))}
              </YearSelect>
            ) : (
              <TitleBold>{selectedYear}</TitleBold>
            )}
          </>
        )}
      </Title>

      <ClearBoth />

      <FiltersWrapper>
        {availableClients.length > 1 && (
          <SourceFilterGroup role="group" aria-label="Client filters">
            <FilterLabel id="client-filter-label" style={{ color: "var(--color-fg-muted)" }}>Filter:</FilterLabel>
            {availableClients.map((client) => {
              const isSelected = clientFilter.length === 0 || clientFilter.includes(client);
              return (
                <SourceFilterButton
                  key={client}
                  $isSelected={isSelected}
                  onClick={() => handleClientToggle(client)}
                  aria-pressed={isSelected}
                  aria-label={`Filter by ${SOURCE_DISPLAY_NAMES[client] || client}`}
                  style={{
                    backgroundColor: isSelected ? `${palette.grade3}30` : "transparent",
                    color: "var(--color-fg-default)",
                    borderColor: isSelected ? palette.grade3 : "var(--color-border-default)",
                  }}
                >
                  {SOURCE_LOGOS[client] && (
                    <SourceLogo
                      src={SOURCE_LOGOS[client]}
                      alt={`${SOURCE_DISPLAY_NAMES[client] || client} logo`}
                    />
                  )}
                  {SOURCE_DISPLAY_NAMES[client] || client}
                </SourceFilterButton>
              );
            })}
            {clientFilter.length > 0 && clientFilter.length < availableClients.length && (
              <ActionButton
                onClick={() => onClientFilterChange([...availableClients])}
                aria-label="Show all clients"
                style={{ color: "var(--color-fg-muted)" }}
              >
                Show all
              </ActionButton>
            )}
            {clientFilter.length === availableClients.length && (
              <ActionButton
                onClick={() => onClientFilterChange([])}
                aria-label="Clear all client filters"
                style={{ color: "var(--color-fg-muted)" }}
              >
                Clear
              </ActionButton>
            )}
          </SourceFilterGroup>
        )}
      </FiltersWrapper>
    </Container>
  );
}
