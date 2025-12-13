import { For, createMemo, type Accessor } from "solid-js";
import type { TUIData, SortType } from "../hooks/useData.js";
import { formatTokensCompact, formatCostFull } from "../utils/format.js";

const STRIPE_BG = "#232328";

interface DailyViewProps {
  data: TUIData;
  sortBy: SortType;
  sortDesc: boolean;
  selectedIndex: Accessor<number>;
  height: number;
}

export function DailyView(props: DailyViewProps) {
  const sortedEntries = createMemo(() => {
    const entries = props.data.dailyEntries;
    const sortBy = props.sortBy;
    const sortDesc = props.sortDesc;
    
    return [...entries].sort((a, b) => {
      let cmp = 0;
      if (sortBy === "cost") cmp = a.cost - b.cost;
      else if (sortBy === "tokens") cmp = a.total - b.total;
      else cmp = a.date.localeCompare(b.date);
      return sortDesc ? -cmp : cmp;
    });
  });

  const visibleEntries = createMemo(() => sortedEntries().slice(0, props.height - 3));

  const sortArrow = () => (props.sortDesc ? "▼" : "▲");
  const dateHeader = () => (props.sortBy === "name" ? `${sortArrow()} Date` : "Date");
  const totalHeader = () => (props.sortBy === "tokens" ? `${sortArrow()} Total` : "Total");
  const costHeader = () => (props.sortBy === "cost" ? `${sortArrow()} Cost` : "Cost");

  return (
    <box flexDirection="column">
      <box flexDirection="row">
        <text fg="cyan" bold>
          {`  ${dateHeader()}`.padEnd(14)}
          {"Input".padStart(14)}
          {"Output".padStart(14)}
          {"Cache".padStart(14)}
          {totalHeader().padStart(16)}
          {costHeader().padStart(12)}
        </text>
      </box>

      <For each={visibleEntries()}>
        {(entry, i) => {
          const isActive = createMemo(() => i() === props.selectedIndex());
          const rowBg = createMemo(() => isActive() ? "blue" : (i() % 2 === 1 ? STRIPE_BG : undefined));
          
          return (
            <box flexDirection="row">
              <text
                bg={rowBg()}
                fg={isActive() ? "white" : undefined}
              >
                {entry.date.padEnd(14)}
                {formatTokensCompact(entry.input).padStart(14)}
                {formatTokensCompact(entry.output).padStart(14)}
                {formatTokensCompact(entry.cache).padStart(14)}
                {formatTokensCompact(entry.total).padStart(16)}
              </text>
              <text
                fg="green"
                bg={rowBg()}
              >
                {formatCostFull(entry.cost).padStart(12)}
              </text>
            </box>
          );
        }}
      </For>
    </box>
  );
}
