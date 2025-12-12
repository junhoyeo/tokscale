import { Show, For } from "solid-js";
import { BarChart } from "./BarChart.js";
import { Legend } from "./Legend.js";
import { ModelListItem } from "./ModelListItem.js";
import type { TUIData } from "../hooks/useData.js";

interface OverviewViewProps {
  data: TUIData;
  selectedIndex: number;
  scrollOffset: number;
  height: number;
  width: number;
}

function formatCost(cost: number): string {
  if (cost >= 1000) return `$${(cost / 1000).toFixed(1)}K`;
  return `$${cost.toFixed(2)}`;
}

export function OverviewView(props: OverviewViewProps) {
  const safeHeight = () => Math.max(props.height, 12);
  const chartHeight = () => Math.max(5, Math.floor(safeHeight() * 0.35));
  const listHeight = () => Math.max(4, safeHeight() - chartHeight() - 4);
  const itemsPerPage = () => Math.max(1, Math.floor(listHeight() / 2));

  const topModelsForLegend = () => props.data.topModels.slice(0, 5).map(m => m.modelId);

  const visibleModels = () => props.data.topModels.slice(props.scrollOffset, props.scrollOffset + itemsPerPage());
  const totalModels = () => props.data.topModels.length;
  const endIndex = () => Math.min(props.scrollOffset + visibleModels().length, totalModels());

  return (
    <box flexDirection="column" gap={1}>
      <box flexDirection="column">
        <BarChart data={props.data.chartData} width={props.width - 4} height={chartHeight()} />
        <Legend models={topModelsForLegend()} />
      </box>

      <box flexDirection="column">
        <box flexDirection="row" justifyContent="space-between" marginBottom={0}>
          <text bold>Models by Cost</text>
          <box flexDirection="row">
            <text dim>Total: </text>
            <text fg="green">{formatCost(props.data.totalCost)}</text>
          </box>
        </box>

        <box flexDirection="column">
          <For each={visibleModels()}>
            {(model, i) => (
              <ModelListItem
                modelId={model.modelId}
                percentage={model.percentage}
                inputTokens={model.inputTokens}
                outputTokens={model.outputTokens}
                isSelected={props.scrollOffset + i() === props.selectedIndex}
              />
            )}
          </For>
        </box>

        <Show when={totalModels() > visibleModels().length}>
          <text dim>{`↓ ${props.scrollOffset + 1}-${endIndex()} of ${totalModels()} models (↑↓ to scroll)`}</text>
        </Show>
      </box>
    </box>
  );
}
