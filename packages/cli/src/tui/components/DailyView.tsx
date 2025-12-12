import { For } from "solid-js";
import type { TUIData, SortType } from "../hooks/useData.js";

interface DailyViewProps {
  data: TUIData;
  sortBy: SortType;
  sortDesc: boolean;
  selectedIndex: number;
  height: number;
}

export function DailyView(props: DailyViewProps) {
  const sortedEntries = () => {
    return [...props.data.dailyEntries].sort((a, b) => {
      let cmp = 0;
      if (props.sortBy === "cost") cmp = a.cost - b.cost;
      else if (props.sortBy === "tokens") cmp = a.total - b.total;
      else cmp = a.date.localeCompare(b.date);
      return props.sortDesc ? -cmp : cmp;
    });
  };

  const visibleEntries = () => sortedEntries().slice(0, props.height - 3);

  const formatNum = (n: number) => {
    if (n >= 1_000_000_000) return `${(n / 1_000_000_000).toFixed(1)}B`;
    if (n >= 1_000_000) return `${(n / 1_000_000).toFixed(1)}M`;
    if (n >= 1_000) return `${Math.floor(n / 1_000).toLocaleString()},${String(n % 1000).padStart(3, "0")}`;
    return n.toLocaleString();
  };

  const formatCost = (cost: number) => `$${cost.toFixed(2)}`;

  return (
    <box flexDirection="column">
      <box flexDirection="row">
        <text fg="cyan" bold>
          {"  Date".padEnd(14)}
          {"Input".padStart(14)}
          {"Output".padStart(14)}
          {"Cache".padStart(14)}
          {"Total".padStart(16)}
          {"Cost".padStart(12)}
        </text>
      </box>
      <box borderStyle="single" borderTop={false} borderLeft={false} borderRight={false} borderBottom borderColor="gray" />

      <For each={visibleEntries()}>
        {(entry, i) => {
          const isSelected = () => i() === props.selectedIndex;

          return (
            <box flexDirection="row">
              <text
                backgroundColor={isSelected() ? "blue" : undefined}
                fg={isSelected() ? "white" : undefined}
              >
                {entry.date.padEnd(14)}
                {formatNum(entry.input).padStart(14)}
                {formatNum(entry.output).padStart(14)}
                {formatNum(entry.cache).padStart(14)}
                {formatNum(entry.total).padStart(16)}
              </text>
              <text
                fg="green"
                backgroundColor={isSelected() ? "blue" : undefined}
              >
                {formatCost(entry.cost).padStart(12)}
              </text>
            </box>
          );
        }}
      </For>
    </box>
  );
}
