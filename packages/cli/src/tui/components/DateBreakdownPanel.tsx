import { For, createMemo } from "solid-js";
import type { DailyModelBreakdown } from "../types/index.js";
import { getSourceColor } from "../utils/colors.js";
import { formatTokens, formatCost } from "../utils/format.js";
import { ModelRow } from "./ModelRow.js";

function formatDateDisplay(dateStr: string): string {
  const date = new Date(dateStr + "T00:00:00");
  return date.toLocaleDateString("en-US", { weekday: "short", month: "short", day: "numeric", year: "numeric" });
}

export interface DateBreakdownPanelProps {
  breakdown: DailyModelBreakdown;
  isNarrow: boolean;
}

export function DateBreakdownPanel(props: DateBreakdownPanelProps) {
  const groupedBySource = createMemo(() => {
    if (!props.breakdown?.models) return new Map();
    const groups = new Map<string, typeof props.breakdown.models>();
    for (const model of props.breakdown.models) {
      const existing = groups.get(model.source) || [];
      existing.push(model);
      groups.set(model.source, existing);
    }
    for (const [, models] of groups) {
      models.sort((a, b) => {
        const totalA = a.tokens.input + a.tokens.output + (a.tokens.cacheRead || 0) + (a.tokens.cacheWrite || 0);
        const totalB = b.tokens.input + b.tokens.output + (b.tokens.cacheRead || 0) + (b.tokens.cacheWrite || 0);
        return totalB - totalA;
      });
    }
    return groups;
  });

  return (
    <box flexDirection="column" marginTop={1} paddingX={1}>
      <box flexDirection="row" justifyContent="space-between">
        <text bold fg="white">{formatDateDisplay(props.breakdown.date)}</text>
        <box flexDirection="row" gap={2}>
          <text fg="cyan">{formatTokens(props.breakdown.totalTokens)}</text>
          <text fg="green" bold>{formatCost(props.breakdown.cost)}</text>
        </box>
      </box>
      
      <box flexDirection="column" marginTop={1}>
        <For each={Array.from(groupedBySource().entries())}>
          {([source, models]) => (
            <box flexDirection="column">
              <box flexDirection="row" gap={1}>
                <text fg={getSourceColor(source)} bold>{`● ${source.toUpperCase()}`}</text>
                <text dim>{`(${models.length} model${models.length > 1 ? "s" : ""})`}</text>
              </box>
              <For each={models}>
                {(model) => (
                  <ModelRow
                    modelId={model.modelId}
                    tokens={{
                      input: model.tokens.input,
                      output: model.tokens.output,
                      cacheRead: model.tokens.cacheRead,
                      cacheWrite: model.tokens.cacheWrite,
                    }}
                    compact={props.isNarrow}
                    indent={2}
                  />
                )}
              </For>
            </box>
          )}
        </For>
      </box>
      
      <box flexDirection="row" marginTop={1}>
        <text dim>Click another day or same day to close</text>
      </box>
    </box>
  );
}
