import { For, Show, createMemo, createSignal } from "solid-js";
import type { TUIData } from "../hooks/useData.js";
import type { ColorPaletteName } from "../config/themes.js";
import { getPalette, getGradeColor } from "../config/themes.js";
import { getModelColor } from "../utils/colors.js";
import { formatTokens } from "../utils/format.js";
import { isNarrow } from "../utils/responsive.js";
import { DateBreakdownPanel } from "./DateBreakdownPanel.js";

interface StatsViewProps {
  data: TUIData;
  height: number;
  colorPalette: ColorPaletteName;
  width?: number;
  selectedDate?: string | null;
}

const MONTHS = ["Jan", "Feb", "Mar", "Apr", "May", "Jun", "Jul", "Aug", "Sep", "Oct", "Nov", "Dec"];
const MONTHS_SHORT = ["J", "F", "M", "A", "M", "J", "J", "A", "S", "O", "N", "D"];
const DAYS = ["", "Mon", "", "Wed", "", "Fri", ""];

interface MonthLabel {
  month: string;
  weekIndex: number;
}

export function StatsView(props: StatsViewProps) {
  const palette = () => getPalette(props.colorPalette);
  const isNarrowTerminal = () => isNarrow(props.width);
  const grid = () => props.data.contributionGrid;
  const cellWidth = 2;
  
  const [clickedCell, setClickedCell] = createSignal<string | null>(null);
  
  const selectedBreakdown = createMemo(() => {
    const date = clickedCell();
    if (!date) return null;
    if (!props.data.dailyBreakdowns) return null;
    if (!(props.data.dailyBreakdowns instanceof Map)) return null;
    return props.data.dailyBreakdowns.get(date) || null;
  });
  
  const monthPositions = createMemo(() => {
    const sundayRow = grid()[0] || [];
    if (sundayRow.length === 0) return [];
    
    const positions: MonthLabel[] = [];
    let lastMonth = -1;
    const monthNames = isNarrowTerminal() ? MONTHS_SHORT : MONTHS;
    
    for (let weekIdx = 0; weekIdx < sundayRow.length; weekIdx++) {
      const cell = sundayRow[weekIdx];
      if (!cell.date) continue;
      const month = new Date(cell.date + "T00:00:00").getMonth();
      if (month !== lastMonth) {
        positions.push({ month: monthNames[month], weekIndex: weekIdx });
        lastMonth = month;
      }
    }
    return positions;
  });

  const totalWeeks = createMemo(() => (grid()[0] || []).length);

  const monthLabelRow = createMemo(() => {
    const weeks = totalWeeks();
    const positions = monthPositions();
    const chars: string[] = new Array(weeks * cellWidth).fill(" ");
    
    for (const pos of positions) {
      const startIdx = pos.weekIndex * cellWidth;
      const monthChars = pos.month.split("");
      for (let i = 0; i < monthChars.length && startIdx + i < chars.length; i++) {
        chars[startIdx + i] = monthChars[i];
      }
    }
    
    return chars.join("");
  });

  const dayLabelWidth = () => isNarrowTerminal() ? 2 : 4;

  const isSelected = (cellDate: string | null) => 
    cellDate && (clickedCell() === cellDate || props.selectedDate === cellDate);
  
  const getCellColor = (level: number) => 
    level === 0 ? "#666666" : getGradeColor(palette(), level as 0 | 1 | 2 | 3 | 4);



  return (
    <box flexDirection="column" gap={1}>
      <box flexDirection="column">
        <box flexDirection="row">
          <text dim>{" ".repeat(dayLabelWidth())}</text>
          <text dim>{monthLabelRow()}</text>
        </box>

        <box onMouseDown={(e: { x: number; y: number }) => {
          const labelW = dayLabelWidth();
          const col = Math.floor((e.x - labelW) / cellWidth);
          const row = e.y - 2;
          const gridRows = grid().length;
          
          if (row < 0 || row >= gridRows) {
            return;
          }
          if (col < 0) {
            return;
          }
          
          const rowData = grid()[row];
          if (!rowData || col >= rowData.length) {
            return;
          }
          
          const cell = rowData[col];
          if (!cell?.date) {
            return;
          }
          
          const newDate = clickedCell() === cell.date ? null : cell.date;
          setClickedCell(newDate);
        }}>
          <For each={DAYS}>
            {(day, dayIndex) => (
              <box flexDirection="row">
                <text dim>{isNarrowTerminal() ? "  " : day.padStart(3) + " "}</text>
                <For each={grid()[dayIndex()] || []}>
                  {(cell) => (
                    <text 
                      fg={isSelected(cell.date) ? "#ffffff" : getCellColor(cell.level)} 
                      bg={isSelected(cell.date) ? getCellColor(cell.level) : undefined}
                    >
                      {isSelected(cell.date) ? "▓▓" : (cell.level === 0 ? "· " : "██")}
                    </text>
                  )}
                </For>
              </box>
            )}
          </For>
        </box>
      </box>

      <box flexDirection="row" gap={2} marginTop={1}>
        <text dim>Less</text>
        <box flexDirection="row" gap={0}>
          <For each={[0, 1, 2, 3, 4]}>
            {(level) => (
              <text
                fg={level === 0 ? "#666666" : getGradeColor(palette(), level as 0 | 1 | 2 | 3 | 4)}
              >
                {level === 0 ? "· " : "██"}
              </text>
            )}
          </For>
        </box>
        <text dim>More</text>
        <Show when={!isNarrowTerminal()}>
          <text dim>|</text>
          <text dim>Click on a day to see breakdown</text>
        </Show>
      </box>

      <Show when={selectedBreakdown()}>
        <DateBreakdownPanel breakdown={selectedBreakdown()!} isNarrow={isNarrowTerminal()} />
      </Show>

      <Show when={!selectedBreakdown()}>
        <box flexDirection="column" marginTop={1}>
          <box flexDirection={isNarrowTerminal() ? "column" : "row"} gap={isNarrowTerminal() ? 0 : 4}>
            <box flexDirection="column">
              <box flexDirection="row" gap={1}>
                <text dim>{isNarrowTerminal() ? "Model:" : "Favorite model:"}</text>
                <text fg={getModelColor(props.data.stats.favoriteModel)}>{props.data.stats.favoriteModel}</text>
              </box>
              <box flexDirection="row" gap={1}>
                <text dim>Sessions:</text>
                <text fg="cyan">{props.data.stats.sessions.toLocaleString()}</text>
              </box>
              <box flexDirection="row" gap={1}>
                <text dim>{isNarrowTerminal() ? "Streak:" : "Current streak:"}</text>
                <text fg="cyan">{`${props.data.stats.currentStreak} days`}</text>
              </box>
              <box flexDirection="row" gap={1}>
                <text dim>{isNarrowTerminal() ? "Active:" : "Active days:"}</text>
                <text fg="cyan">{`${props.data.stats.activeDays}/${props.data.stats.totalDays}`}</text>
              </box>
            </box>

            <box flexDirection="column">
              <box flexDirection="row" gap={1}>
                <text dim>{isNarrowTerminal() ? "Tokens:" : "Total tokens:"}</text>
                <text fg="cyan">{formatTokens(props.data.stats.totalTokens)}</text>
              </box>
              <box flexDirection="row" gap={1}>
                <text dim>{isNarrowTerminal() ? "Session:" : "Longest session:"}</text>
                <text fg="cyan">{props.data.stats.longestSession}</text>
              </box>
              <box flexDirection="row" gap={1}>
                <text dim>{isNarrowTerminal() ? "Max streak:" : "Longest streak:"}</text>
                <text fg="cyan">{`${props.data.stats.longestStreak} days`}</text>
              </box>
              <box flexDirection="row" gap={1}>
                <text dim>{isNarrowTerminal() ? "Peak:" : "Peak hour:"}</text>
                <text fg="cyan">{props.data.stats.peakHour}</text>
              </box>
            </box>
          </box>
        </box>

        <Show when={!isNarrowTerminal()}>
          <box marginTop={1}>
            <text fg="yellow" italic>{`Your total spending is $${props.data.totalCost.toFixed(2)} on AI coding assistants!`}</text>
          </box>
        </Show>
        <Show when={isNarrowTerminal()}>
          <box marginTop={1}>
            <text fg="yellow" italic>{`Total: $${props.data.totalCost.toFixed(2)}`}</text>
          </box>
        </Show>
      </Show>
    </box>
  );
}
