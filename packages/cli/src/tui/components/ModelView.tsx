import { For } from "solid-js";
import type { TUIData, SortType } from "../hooks/useData.js";

interface ModelViewProps {
  data: TUIData;
  sortBy: SortType;
  sortDesc: boolean;
  selectedIndex: number;
  height: number;
}

export function ModelView(props: ModelViewProps) {
  const sortedEntries = () => {
    return [...props.data.modelEntries].sort((a, b) => {
      let cmp = 0;
      if (props.sortBy === "cost") cmp = a.cost - b.cost;
      else if (props.sortBy === "tokens") cmp = a.total - b.total;
      else cmp = a.model.localeCompare(b.model);
      return props.sortDesc ? -cmp : cmp;
    });
  };

  const visibleEntries = () => sortedEntries().slice(0, props.height - 3);

  const formatNum = (n: number) => {
    if (n >= 1_000_000_000) return `${(n / 1_000_000_000).toFixed(1)}B`;
    if (n >= 1_000_000) return `${(n / 1_000_000).toFixed(1)}M`;
    if (n >= 1_000) return `${(n / 1_000).toFixed(0)}K`;
    return n.toLocaleString();
  };

  const formatCost = (cost: number) => `$${cost.toFixed(2)}`;

  return (
    <box flexDirection="column">
      <box flexDirection="row">
        <text fg="cyan" bold>
          {"  Source/Model".padEnd(24)}
          {"Input".padStart(12)}
          {"Output".padStart(12)}
          {"Cache".padStart(12)}
          {"Total".padStart(14)}
          {"Cost".padStart(12)}
        </text>
      </box>
      <box borderStyle="single" borderTop={false} borderLeft={false} borderRight={false} borderBottom borderColor="gray" />

      <For each={visibleEntries()}>
        {(entry, i) => {
          const isSelected = () => i() === props.selectedIndex;
          const sourceLabel = entry.source.charAt(0).toUpperCase() + entry.source.slice(1);
          const displayName = `${sourceLabel} ${entry.model}`.slice(0, 22);

          return (
            <box flexDirection="row">
              <text
                backgroundColor={isSelected() ? "blue" : undefined}
                fg={isSelected() ? "white" : undefined}
              >
                {displayName.padEnd(24)}
                {formatNum(entry.input).padStart(12)}
                {formatNum(entry.output).padStart(12)}
                {formatNum(entry.cacheRead).padStart(12)}
                {formatNum(entry.total).padStart(14)}
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
