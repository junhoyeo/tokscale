import { For, createMemo, type Accessor } from "solid-js";
import type { TUIData, SortType } from "../hooks/useData.js";
import { getModelColor } from "../utils/colors.js";
import { formatTokensCompact, formatCostFull } from "../utils/format.js";

const STRIPE_BG = "#232328";

const INPUT_COL_WIDTH = 12;
const OUTPUT_COL_WIDTH = 12;
const CACHE_COL_WIDTH = 12;
const TOTAL_COL_WIDTH = 14;
const COST_COL_WIDTH = 12;
const METRIC_COLUMNS_WIDTH = INPUT_COL_WIDTH + OUTPUT_COL_WIDTH + CACHE_COL_WIDTH + TOTAL_COL_WIDTH + COST_COL_WIDTH;
const SIDE_PADDING = 2;
const MIN_NAME_COLUMN = 24;

interface ModelViewProps {
  data: TUIData;
  sortBy: SortType;
  sortDesc: boolean;
  selectedIndex: Accessor<number>;
  height: number;
  width: number;
}

export function ModelView(props: ModelViewProps) {
  const sortedEntries = createMemo(() => {
    const entries = props.data.modelEntries;
    const sortBy = props.sortBy;
    const sortDesc = props.sortDesc;
    
    return [...entries].sort((a, b) => {
      let cmp = 0;
      if (sortBy === "cost") cmp = a.cost - b.cost;
      else if (sortBy === "tokens") cmp = a.total - b.total;
      else cmp = a.model.localeCompare(b.model);
      return sortDesc ? -cmp : cmp;
    });
  });

  const nameColumnWidths = createMemo(() => {
    const available = Math.max(props.width - SIDE_PADDING - METRIC_COLUMNS_WIDTH, MIN_NAME_COLUMN);
    const nameColumn = Math.max(MIN_NAME_COLUMN, available);

    return {
      column: nameColumn,
      text: Math.max(nameColumn - 1, 1),
    };
  });

  const visibleEntries = createMemo(() => {
    const maxRows = Math.max(props.height - 3, 0);
    return sortedEntries().slice(0, maxRows);
  });

  const formattedRows = createMemo(() => {
    const nameWidth = nameColumnWidths().text;
    return visibleEntries().map((entry) => {
      const sourceLabel = entry.source.charAt(0).toUpperCase() + entry.source.slice(1);
      const fullName = `${sourceLabel} ${entry.model}`;
      let displayName = fullName;
      if (fullName.length > nameWidth) {
        displayName = nameWidth > 1 ? `${fullName.slice(0, nameWidth - 1)}…` : fullName.slice(0, 1);
      }

      return {
        entry,
        displayName,
        nameWidth,
        input: formatTokensCompact(entry.input),
        output: formatTokensCompact(entry.output),
        cache: formatTokensCompact(entry.cacheRead),
        total: formatTokensCompact(entry.total),
        cost: formatCostFull(entry.cost),
      };
    });
  });

  const sortArrow = () => (props.sortDesc ? "▼" : "▲");
  const nameHeader = () => ` Source/Model${props.sortBy === "name" ? " " + sortArrow() : ""}`;
  const totalHeader = () => (props.sortBy === "tokens" ? `${sortArrow()} Total` : "Total");
  const costHeader = () => (props.sortBy === "cost" ? `${sortArrow()} Cost` : "Cost");

  return (
    <box flexDirection="column">
      <box flexDirection="row">
        <text fg="cyan" bold>
          {nameHeader().padEnd(nameColumnWidths().column)}
          {"Input".padStart(INPUT_COL_WIDTH)}
          {"Output".padStart(OUTPUT_COL_WIDTH)}
          {"Cache".padStart(CACHE_COL_WIDTH)}
          {totalHeader().padStart(TOTAL_COL_WIDTH)}
          {costHeader().padStart(COST_COL_WIDTH)}
        </text>
      </box>

      <For each={formattedRows()}>
        {(row, i) => {
          const isActive = createMemo(() => i() === props.selectedIndex());
          const rowBg = createMemo(() => isActive() ? "blue" : (i() % 2 === 1 ? STRIPE_BG : undefined));
          
          return (
            <box flexDirection="row">
              <text 
                fg={getModelColor(row.entry.model)} 
                bg={rowBg()}
              >●</text>
              <text
                bg={rowBg()}
                fg={isActive() ? "white" : undefined}
              >
                {row.displayName.padEnd(row.nameWidth)}
                {row.input.padStart(INPUT_COL_WIDTH)}
                {row.output.padStart(OUTPUT_COL_WIDTH)}
                {row.cache.padStart(CACHE_COL_WIDTH)}
                {row.total.padStart(TOTAL_COL_WIDTH)}
              </text>
              <text
                fg="green"
                bg={rowBg()}
              >
                {row.cost.padStart(COST_COL_WIDTH)}
              </text>
            </box>
          );
        }}
      </For>
    </box>
  );
}
