"use client";

import styled, { css } from "styled-components";
import type { ViewMode, ColorPaletteName, ClientType, GraphColorPalette } from "@/lib/types";
import { getPaletteNames, colorPalettes } from "@/lib/themes";
import { getDarkGraphColor } from "@/lib/darkGraphColors";
import { SOURCE_DISPLAY_NAMES, SOURCE_LOGOS } from "@/lib/constants";
import { formatTokenCount, toggleClientFilter } from "@/lib/utils";
import { Select } from "@/components/ui/Select";
import { SegmentButton, SegmentedGroup } from "@/components/leaderboard/RankingUI";

interface GraphControlsProps {
  /** Id for the heading, so the graph card can be labelled by it. */
  titleId?: string;
  view: ViewMode;
  onViewChange: (view: ViewMode) => void;
  selectedYear: string;
  availableYears: string[];
  onYearChange: (year: string) => void;
  clientFilter: ClientType[];
  availableClients: ClientType[];
  onClientFilterChange: (clients: ClientType[]) => void;
  totalTokens: number;
}

const INTENSITY_LEVELS = [0, 1, 2, 3, 4] as const;

export function GraphControls({
  titleId,
  view,
  onViewChange,
  selectedYear,
  availableYears,
  onYearChange,
  clientFilter,
  availableClients,
  onClientFilterChange,
  totalTokens,
}: GraphControlsProps) {
  const handleClientToggle = (client: ClientType) => {
    onClientFilterChange(toggleClientFilter(client, clientFilter, availableClients));
  };

  return (
    <Container>
      <TopRow>
        <Title id={titleId}>
          <TotalTokens>{formatTokenCount(totalTokens)}</TotalTokens> tokens used
          {selectedYear && (
            <>
              {" "}in{" "}
              {availableYears.length > 1 ? (
                <YearSelect
                  variant="inline"
                  aria-label="Select year"
                  value={selectedYear}
                  options={availableYears.map((year) => ({ value: year, label: year }))}
                  onChange={onYearChange}
                />
              ) : (
                <YearText>{selectedYear}</YearText>
              )}
            </>
          )}
        </Title>

        <SegmentedGroup role="group" aria-label="View mode">
          {(["2d", "3d"] as const).map((mode) => (
            <ViewButton
              key={mode}
              type="button"
              $active={view === mode}
              onClick={() => onViewChange(mode)}
              aria-pressed={view === mode}
              aria-label={`${mode.toUpperCase()} view`}
            >
              {mode.toUpperCase()}
            </ViewButton>
          ))}
        </SegmentedGroup>
      </TopRow>

      {availableClients.length > 1 && (
        <FilterRow role="group" aria-label="Client filters">
          <FilterLabel>Clients</FilterLabel>
          {availableClients.map((client) => {
            const isSelected = clientFilter.length === 0 || clientFilter.includes(client);
            const name = SOURCE_DISPLAY_NAMES[client] || client;
            return (
              <FilterChip
                key={client}
                type="button"
                $selected={isSelected}
                onClick={() => handleClientToggle(client)}
                aria-pressed={isSelected}
                aria-label={`Filter by ${name}`}
              >
                {SOURCE_LOGOS[client] && <ChipLogo src={SOURCE_LOGOS[client]} alt="" />}
                {name}
              </FilterChip>
            );
          })}
          {clientFilter.length > 0 && clientFilter.length < availableClients.length && (
            <TextButton type="button" onClick={() => onClientFilterChange([...availableClients])} aria-label="Show all clients">
              Show all
            </TextButton>
          )}
          {clientFilter.length === availableClients.length && (
            <TextButton type="button" onClick={() => onClientFilterChange([])} aria-label="Clear all client filters">
              Clear
            </TextButton>
          )}
        </FilterRow>
      )}
    </Container>
  );
}

interface GraphPaletteLegendProps {
  paletteName: ColorPaletteName;
  onPaletteChange: (palette: ColorPaletteName) => void;
  palette: GraphColorPalette;
}

/** Palette picker and the low-to-high key, under the graph. */
export function GraphPaletteLegend({ paletteName, onPaletteChange, palette }: GraphPaletteLegendProps) {
  return (
    <Footer>
      <PaletteControl>
        <span aria-hidden="true">Color</span>
        <PaletteSelect
          aria-label="Color palette"
          value={paletteName}
          options={getPaletteNames().map((name) => ({
            value: name,
            textValue: colorPalettes[name].name,
            label: (
              <PaletteOption>
                <Swatches aria-hidden="true">
                  {([1, 2, 3, 4] as const).map((level) => (
                    <Swatch key={level} $color={getDarkGraphColor(colorPalettes[name], level)} />
                  ))}
                </Swatches>
                {colorPalettes[name].name}
              </PaletteOption>
            ),
          }))}
          onChange={(next) => onPaletteChange(next as ColorPaletteName)}
        />
      </PaletteControl>

      <Legend role="img" aria-label="Usage intensity, low to high">
        <span aria-hidden="true">Low</span>
        <Swatches aria-hidden="true">
          {INTENSITY_LEVELS.map((level) => (
            <Swatch key={level} $color={getDarkGraphColor(palette, level)} $outlined={level === 0} />
          ))}
        </Swatches>
        <span aria-hidden="true">High</span>
      </Legend>
    </Footer>
  );
}

const focusRing = css`
  &:focus-visible {
    outline: 2px solid var(--service-focus);
    outline-offset: 2px;
  }
`;

const Container = styled.div`
  display: grid;
  gap: 14px;
  min-width: 0;
`;

const TopRow = styled.div`
  display: flex;
  flex-wrap: wrap;
  align-items: center;
  justify-content: space-between;
  gap: 12px 16px;
  min-width: 0;
`;

const Title = styled.h2`
  display: flex;
  flex-wrap: wrap;
  align-items: baseline;
  gap: 0 0.3em;
  min-width: 0;
  margin: 0;
  color: var(--service-text);
  font-size: 1.125rem;
  font-weight: 500;
  letter-spacing: -0.01em;
  line-height: 1.4;
`;

const TotalTokens = styled.span`
  color: var(--v2-numeric);
  font-weight: 600;
  font-variant-numeric: tabular-nums;
`;

const YearText = styled.span`
  font-weight: 600;
  font-variant-numeric: tabular-nums;
`;

/* Inline select that reads as part of the heading: same size, dotted underline, chevron from Select. */
const YearSelect = styled(Select)`
  --select-font-size: 1em;
  --select-height: 1.5em;

  && {
    color: var(--service-text);
    font-weight: 600;
    font-variant-numeric: tabular-nums;
  }
`;

const ViewButton = styled(SegmentButton)`
  min-width: 40px;
  font-weight: 600;

  @media (pointer: coarse) {
    min-height: 44px;
    min-width: 48px;
  }
`;

const FilterRow = styled.div`
  display: flex;
  flex-wrap: wrap;
  align-items: center;
  gap: 8px;
  min-width: 0;

  @media (max-width: 560px) {
    flex-wrap: nowrap;
    overflow-x: auto;
    overscroll-behavior-inline: contain;
    padding-bottom: 4px;
    scrollbar-width: none;
    -webkit-overflow-scrolling: touch;

    &::-webkit-scrollbar {
      display: none;
    }
  }
`;

const FilterLabel = styled.span`
  flex: 0 0 auto;
  margin-right: 4px;
  color: var(--service-text-muted);
  font-size: 0.8125rem;
  font-weight: 500;
  white-space: nowrap;
`;

const FilterChip = styled.button<{ $selected: boolean }>`
  display: inline-flex;
  flex: 0 0 auto;
  align-items: center;
  gap: 6px;
  min-height: 30px;
  padding: 0 12px 0 6px;
  border: 1px solid ${({ $selected }) => ($selected ? "rgba(0, 115, 255, 0.55)" : "var(--service-border)")};
  border-radius: 999px;
  background: ${({ $selected }) => ($selected ? "var(--v2-accent-soft)" : "transparent")};
  color: ${({ $selected }) => ($selected ? "var(--service-text)" : "var(--service-text-muted)")};
  font-size: 0.8125rem;
  font-weight: 500;
  white-space: nowrap;
  touch-action: manipulation;
  transition:
    background-color 120ms ease,
    border-color 120ms ease,
    color 120ms ease;

  img {
    opacity: ${({ $selected }) => ($selected ? 1 : 0.5)};
  }

  &:hover {
    border-color: ${({ $selected }) => ($selected ? "rgba(0, 115, 255, 0.8)" : "var(--service-border-strong)")};
    color: var(--service-text);
  }

  ${focusRing}

  @media (pointer: coarse) {
    min-height: 40px;
  }

  @media (prefers-reduced-motion: reduce) {
    transition: none;
  }
`;

const ChipLogo = styled.img`
  width: 18px;
  height: 18px;
  flex-shrink: 0;
  border-radius: 50%;
  object-fit: cover;
`;

const TextButton = styled.button`
  flex: 0 0 auto;
  min-height: 30px;
  padding: 0 8px;
  border-radius: 6px;
  color: var(--service-text-muted);
  font-size: 0.8125rem;
  font-weight: 500;
  text-decoration: underline;
  text-decoration-color: var(--service-border-strong);
  text-underline-offset: 3px;
  white-space: nowrap;

  &:hover {
    color: var(--service-text);
  }

  ${focusRing}

  @media (pointer: coarse) {
    min-height: 40px;
  }
`;

const Footer = styled.div`
  display: flex;
  flex-wrap: wrap;
  align-items: center;
  justify-content: space-between;
  gap: 10px 16px;
`;

const PaletteControl = styled.div`
  display: inline-flex;
  min-width: 0;
  align-items: center;
  gap: 8px;
  color: var(--service-text-muted);
  font-size: 0.8125rem;
`;

const PaletteSelect = styled(Select)`
  --select-height: 32px;
  --select-font-size: 0.8125rem;
  --select-padding-x: 10px;
  --select-background: var(--service-surface-muted);
  --select-border-color: var(--service-border);
`;

const PaletteOption = styled.span`
  display: inline-flex;
  align-items: center;
  gap: 8px;
`;

const Swatches = styled.span`
  display: inline-flex;
  flex: 0 0 auto;
  gap: 3px;
`;

const Swatch = styled.span<{ $color: string; $outlined?: boolean }>`
  width: 10px;
  height: 10px;
  border-radius: 2px;
  background: ${({ $color }) => $color};
  box-shadow: ${({ $outlined }) => ($outlined ? "inset 0 0 0 1px rgba(255, 255, 255, 0.08)" : "none")};
`;

const Legend = styled.div`
  display: inline-flex;
  align-items: center;
  gap: 6px;
  color: var(--service-text-muted);
  font-size: 0.75rem;
`;
