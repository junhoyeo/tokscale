import { For } from "solid-js";
import type { TUIData } from "../hooks/useData.js";
import type { ColorPaletteName } from "../config/themes.js";
import { getPalette, getGradeColor } from "../config/themes.js";
import { getModelColor } from "../utils/colors.js";
import { formatTokens } from "../utils/format.js";

interface StatsViewProps {
  data: TUIData;
  height: number;
  colorPalette: ColorPaletteName;
}

const MONTHS = ["Jan", "Feb", "Mar", "Apr", "May", "Jun", "Jul", "Aug", "Sep", "Oct", "Nov", "Dec"];
const DAYS = ["", "Mon", "", "Wed", "", "Fri", ""];

export function StatsView(props: StatsViewProps) {
  const palette = () => getPalette(props.colorPalette);

  const grid = () => props.data.contributionGrid;

  return (
    <box flexDirection="column" gap={1}>
      <box flexDirection="column">
        <box flexDirection="row" gap={1} marginLeft={4}>
          <For each={MONTHS}>
            {(month) => <text dim>{month.padEnd(4)}</text>}
          </For>
        </box>

        <For each={DAYS}>
          {(day, dayIndex) => (
            <box flexDirection="row">
              <text dim>{day.padStart(3)} </text>
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
        <box flexDirection="row" gap={4}>
          <box flexDirection="column">
            <box flexDirection="row" gap={1}>
              <text dim>Favorite model:</text>
              <text fg={getModelColor(props.data.stats.favoriteModel)}>{props.data.stats.favoriteModel}</text>
            </box>
            <box flexDirection="row" gap={1}>
              <text dim>Sessions:</text>
              <text fg="cyan">{props.data.stats.sessions.toLocaleString()}</text>
            </box>
            <box flexDirection="row" gap={1}>
              <text dim>Current streak:</text>
              <text fg="cyan">{`${props.data.stats.currentStreak} days`}</text>
            </box>
            <box flexDirection="row" gap={1}>
              <text dim>Active days:</text>
              <text fg="cyan">{`${props.data.stats.activeDays}/${props.data.stats.totalDays}`}</text>
            </box>
          </box>

          <box flexDirection="column">
            <box flexDirection="row" gap={1}>
              <text dim>Total tokens:</text>
              <text fg="cyan">{formatTokens(props.data.stats.totalTokens)}</text>
            </box>
            <box flexDirection="row" gap={1}>
              <text dim>Longest session:</text>
              <text fg="cyan">{props.data.stats.longestSession}</text>
            </box>
            <box flexDirection="row" gap={1}>
              <text dim>Longest streak:</text>
              <text fg="cyan">{`${props.data.stats.longestStreak} days`}</text>
            </box>
            <box flexDirection="row" gap={1}>
              <text dim>Peak hour:</text>
              <text fg="cyan">{props.data.stats.peakHour}</text>
            </box>
          </box>
        </box>
      </box>

      <box marginTop={1}>
        <text fg="yellow" italic>{`Your total spending is $${props.data.totalCost.toFixed(2)} on AI coding assistants!`}</text>
      </box>
    </box>
  );
}


