import { For, createMemo, type Accessor } from "solid-js";
import type { TUIData, SortType } from "../hooks/useData.js";
import { formatTokensCompact, formatCostFull } from "../utils/format.js";
import { isNarrow } from "../utils/responsive.js";

const STRIPE_BG = "#232328";

const INPUT_COL_WIDTH = 12;
const OUTPUT_COL_WIDTH = 12;
const CACHE_READ_COL_WIDTH = 10;
const CACHE_WRITE_COL_WIDTH = 10;
const TOTAL_COL_WIDTH = 14;
const COST_COL_WIDTH = 12;
const METRIC_COLUMNS_WIDTH_FULL = INPUT_COL_WIDTH + OUTPUT_COL_WIDTH + CACHE_READ_COL_WIDTH + CACHE_WRITE_COL_WIDTH + TOTAL_COL_WIDTH + COST_COL_WIDTH;
const METRIC_COLUMNS_WIDTH_NARROW = TOTAL_COL_WIDTH + COST_COL_WIDTH;
const SIDE_PADDING = 2;
const MIN_DATE_COLUMN = 14;

interface DailyViewProps {
  data: TUIData;
  sortBy: SortType;
  sortDesc: boolean;
  selectedIndex: Accessor<number>;
  height: number;
  width?: number;
}

export function DailyView(props: DailyViewProps) {
  const isNarrowTerminal = () => isNarrow(props.width);
  const terminalWidth = () => props.width ?? process.stdout.columns ?? 80;
  
  const dateColumnWidths = createMemo(() => {
    const metricWidth = isNarrowTerminal() ? METRIC_COLUMNS_WIDTH_NARROW : METRIC_COLUMNS_WIDTH_FULL;
    const minDate = MIN_DATE_COLUMN;
    const available = Math.max(terminalWidth() - SIDE_PADDING - metricWidth, minDate);
    const dateColumn = Math.max(minDate, available);

    return {
      column: dateColumn,
      text: dateColumn,
    };
  });
  
  const sortedEntries = createMemo(() => {
    const entries = props.data.dailyEntries;
    const sortBy = props.sortBy;
    const sortDesc = props.sortDesc;
    
    return [...entries].sort((a, b) => {
      let cmp = 0;
      if (sortBy === "cost") cmp = a.cost - b.cost;
      else if (sortBy === "tokens") cmp = a.total - b.total;
      else if (sortBy === "date") cmp = a.date.localeCompare(b.date);
      return sortDesc ? -cmp : cmp;
    });
  });

  const visibleEntries = createMemo(() => {
    const maxRows = Math.max(props.height - 3, 0);
    return sortedEntries().slice(0, maxRows);
  });

  const formattedRows = createMemo(() => {
    const dateColWidth = dateColumnWidths().column;
    const narrow = isNarrowTerminal();
    return visibleEntries().map((entry) => ({
      entry,
      dateColWidth,
      narrow,
      input: formatTokensCompact(entry.input),
      output: formatTokensCompact(entry.output),
      cacheRead: formatTokensCompact(entry.cacheRead),
      cacheWrite: formatTokensCompact(entry.cacheWrite),
      total: formatTokensCompact(entry.total),
      cost: formatCostFull(entry.cost),
    }));
  });

  const sortArrow = () => (props.sortDesc ? "▼" : "▲");
  const dateHeader = () => (props.sortBy === "date" ? `${sortArrow()} Date` : "Date");
  const totalHeader = () => (props.sortBy === "tokens" ? `${sortArrow()} Total` : "Total");
  const costHeader = () => (props.sortBy === "cost" ? `${sortArrow()} Cost` : "Cost");

  const renderHeader = () => {
    const dateColWidth = dateColumnWidths().column;
    if (isNarrowTerminal()) {
      return `${"Date".padEnd(dateColWidth)}${totalHeader().padStart(TOTAL_COL_WIDTH)}${costHeader().padStart(COST_COL_WIDTH)}`;
    }
    return `${("  " + dateHeader()).padEnd(dateColWidth)}${"Input".padStart(INPUT_COL_WIDTH)}${"Output".padStart(OUTPUT_COL_WIDTH)}${"C.Read".padStart(CACHE_READ_COL_WIDTH)}${"C.Write".padStart(CACHE_WRITE_COL_WIDTH)}${totalHeader().padStart(TOTAL_COL_WIDTH)}${costHeader().padStart(COST_COL_WIDTH)}`;
  };

  const renderRowData = (row: typeof formattedRows extends () => (infer T)[] ? T : never) => {
    if (row.narrow) {
      return `${row.entry.date.padEnd(row.dateColWidth)}${row.total.padStart(TOTAL_COL_WIDTH)}`;
    }
    return `${row.entry.date.padEnd(row.dateColWidth)}${row.input.padStart(INPUT_COL_WIDTH)}${row.output.padStart(OUTPUT_COL_WIDTH)}${row.cacheRead.padStart(CACHE_READ_COL_WIDTH)}${row.cacheWrite.padStart(CACHE_WRITE_COL_WIDTH)}${row.total.padStart(TOTAL_COL_WIDTH)}`;
  };

  return (
    <box flexDirection="column">
      <box flexDirection="row">
        <text fg="cyan" bold>
          {renderHeader()}
        </text>
      </box>

      <For each={formattedRows()}>
        {(row, i) => {
          const isActive = createMemo(() => i() === props.selectedIndex());
          const rowBg = createMemo(() => isActive() ? "blue" : (i() % 2 === 1 ? STRIPE_BG : undefined));

          return (
            <box flexDirection="row">
              <text
                bg={rowBg()}
                fg={isActive() ? "white" : undefined}
              >
                {renderRowData(row)}
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
