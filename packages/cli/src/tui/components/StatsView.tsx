import { For, Show } from "solid-js";
import type { TUIData } from "../hooks/useData.js";
import type { ColorPaletteName } from "../config/themes.js";
import { getPalette, getGradeColor } from "../config/themes.js";
import { getModelColor } from "../utils/colors.js";
import { formatTokens } from "../utils/format.js";
import { isNarrow } from "../utils/responsive.js";

interface StatsViewProps {
  data: TUIData;
  height: number;
  colorPalette: ColorPaletteName;
  width?: number;
}

const MONTHS = ["Jan", "Feb", "Mar", "Apr", "May", "Jun", "Jul", "Aug", "Sep", "Oct", "Nov", "Dec"];
const MONTHS_SHORT = ["J", "F", "M", "A", "M", "J", "J", "A", "S", "O", "N", "D"];
const DAYS = ["", "Mon", "", "Wed", "", "Fri", ""];

export function StatsView(props: StatsViewProps) {
  const palette = () => getPalette(props.colorPalette);
  const isNarrowTerminal = () => isNarrow(props.width);

  const grid = () => props.data.contributionGrid;
  const monthLabels = () => isNarrowTerminal() ? MONTHS_SHORT : MONTHS;

  return (
    <box flexDirection="column" gap={1}>
      <box flexDirection="column">
        <box flexDirection="row" gap={isNarrowTerminal() ? 0 : 1} marginLeft={4}>
          <For each={monthLabels()}>
            {(month) => <text dim>{isNarrowTerminal() ? month.padEnd(3) : month.padEnd(4)}</text>}
          </For>
        </box>

        <For each={DAYS}>
          {(day, dayIndex) => (
            <box flexDirection="row">
              <text dim>{isNarrowTerminal() ? " " : day.padStart(3) + " "}</text>
              <For each={grid()[dayIndex()] || []}>
                {(cell) => (
                  <text
                    fg={cell.level === 0 ? "gray" : getGradeColor(palette(), cell.level as 0 | 1 | 2 | 3 | 4)}
                  >
                    {cell.level === 0 ? "·" : "█"}
                  </text>
                )}
              </For>
            </box>
          )}
        </For>
      </box>

      <box flexDirection="row" gap={2} marginTop={1}>
        <text dim>Less</text>
        <box flexDirection="row" gap={0}>
          <For each={[0, 1, 2, 3, 4]}>
            {(level) => (
              <text
                fg={level === 0 ? "gray" : getGradeColor(palette(), level as 0 | 1 | 2 | 3 | 4)}
              >
                {level === 0 ? "·" : "█"}
              </text>
            )}
          </For>
        </box>
        <text dim>More</text>
      </box>

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
    </box>
  );
}


