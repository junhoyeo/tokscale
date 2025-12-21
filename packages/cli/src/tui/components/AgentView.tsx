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
const MSGS_COL_WIDTH = 10;
const METRIC_COLUMNS_WIDTH_FULL = INPUT_COL_WIDTH + OUTPUT_COL_WIDTH + CACHE_READ_COL_WIDTH + CACHE_WRITE_COL_WIDTH + TOTAL_COL_WIDTH + COST_COL_WIDTH;
const METRIC_COLUMNS_WIDTH_NARROW = TOTAL_COL_WIDTH + MSGS_COL_WIDTH + COST_COL_WIDTH;
const SIDE_PADDING = 2;
const MIN_NAME_COLUMN = 20;
const MIN_NAME_COLUMN_NARROW = 14;

const AGENT_COLORS: Record<string, string> = {
  "OmO": "#FFD700",
  "explore": "#00CED1",
  "executor": "#FF6347",
  "librarian": "#9370DB",
  "oracle": "#20B2AA",
  "build": "#FF8C00",
  "task-orchestrator": "#4169E1",
  "general": "#808080",
  "plan": "#32CD32",
  "plan-reviewer": "#BA55D3",
  "frontend-ui-ux-engineer": "#FF69B4",
  "compaction": "#A0522D",
  "multimodal-looker": "#00BFFF",
  "figma-architect": "#FF1493",
  "document-writer": "#228B22",
};

function getAgentColor(agent: string): string {
  return AGENT_COLORS[agent] || "#808080";
}

interface AgentViewProps {
  data: TUIData;
  sortBy: SortType;
  sortDesc: boolean;
  selectedIndex: Accessor<number>;
  height: number;
  width: number;
}

export function AgentView(props: AgentViewProps) {
  const sortedEntries = createMemo(() => {
    const entries = props.data.agentEntries || [];
    const sortBy = props.sortBy;
    const sortDesc = props.sortDesc;
    
    return [...entries].sort((a, b) => {
      let cmp = 0;
      if (sortBy === "cost") cmp = a.cost - b.cost;
      else if (sortBy === "tokens") cmp = a.total - b.total;
      else cmp = a.agent.localeCompare(b.agent);
      return sortDesc ? -cmp : cmp;
    });
  });

  const isNarrowTerminal = () => isNarrow(props.width);
  
  const nameColumnWidths = createMemo(() => {
    const metricWidth = isNarrowTerminal() ? METRIC_COLUMNS_WIDTH_NARROW : METRIC_COLUMNS_WIDTH_FULL;
    const minName = isNarrowTerminal() ? MIN_NAME_COLUMN_NARROW : MIN_NAME_COLUMN;
    const available = Math.max(props.width - SIDE_PADDING - metricWidth, minName);
    const nameColumn = Math.max(minName, available);

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
      let displayName = entry.agent;
      if (displayName.length > nameWidth) {
        displayName = nameWidth > 1 ? `${displayName.slice(0, nameWidth - 1)}…` : displayName.slice(0, 1);
      }

      return {
        entry,
        displayName,
        nameWidth,
        input: formatTokensCompact(entry.input),
        output: formatTokensCompact(entry.output),
        cacheRead: formatTokensCompact(entry.cacheRead),
        cacheWrite: formatTokensCompact(entry.cacheWrite),
        total: formatTokensCompact(entry.total),
        messages: entry.messageCount.toLocaleString(),
        cost: formatCostFull(entry.cost),
      };
    });
  });

  const sortArrow = () => (props.sortDesc ? "▼" : "▲");
  const totalHeader = () => (props.sortBy === "tokens" ? `${sortArrow()} Total` : "Total");
  const costHeader = () => (props.sortBy === "cost" ? `${sortArrow()} Cost` : "Cost");

  const renderHeader = () => {
    if (isNarrowTerminal()) {
      return ` ${"Agent".padEnd(nameColumnWidths().column - 1)}${totalHeader().padStart(TOTAL_COL_WIDTH)}${"Msgs".padStart(MSGS_COL_WIDTH)}${costHeader().padStart(COST_COL_WIDTH)}`;
    }
    return ` ${"Agent".padEnd(nameColumnWidths().column - 1)}${"Input".padStart(INPUT_COL_WIDTH)}${"Output".padStart(OUTPUT_COL_WIDTH)}${"C.Read".padStart(CACHE_READ_COL_WIDTH)}${"C.Write".padStart(CACHE_WRITE_COL_WIDTH)}${totalHeader().padStart(TOTAL_COL_WIDTH)}${costHeader().padStart(COST_COL_WIDTH)}`;
  };

  const renderRowData = (row: ReturnType<typeof formattedRows>[number]) => {
    if (isNarrowTerminal()) {
      return `${row.displayName.padEnd(row.nameWidth)}${row.total.padStart(TOTAL_COL_WIDTH)}${row.messages.padStart(MSGS_COL_WIDTH)}`;
    }
    return `${row.displayName.padEnd(row.nameWidth)}${row.input.padStart(INPUT_COL_WIDTH)}${row.output.padStart(OUTPUT_COL_WIDTH)}${row.cacheRead.padStart(CACHE_READ_COL_WIDTH)}${row.cacheWrite.padStart(CACHE_WRITE_COL_WIDTH)}${row.total.padStart(TOTAL_COL_WIDTH)}`;
  };

  const hasData = createMemo(() => (props.data.agentEntries?.length || 0) > 0);

  return (
    <box flexDirection="column">
      {hasData() ? (
        <>
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
                    fg={getAgentColor(row.entry.agent)} 
                    bg={rowBg()}
                  >●</text>
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
        </>
      ) : (
        <box flexDirection="column">
          <text fg="yellow">  No agent data available.</text>
          <text fg="gray">  Agent tracking is only available for OpenCode sessions.</text>
        </box>
      )}
    </box>
  );
}
