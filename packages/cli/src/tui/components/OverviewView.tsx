import { Show, For, createMemo, type Accessor } from "solid-js";
import { BarChart } from "./BarChart.js";
import { IntervalChart } from "./IntervalChart.js";
import { Legend } from "./Legend.js";
import { ModelRow } from "./ModelRow.js";
import type { TUIData, SortType } from "../hooks/useData.js";
import type { ChartMode, Resolution, IntervalBucket } from "../types/index.js";
import type { ColorPaletteName } from "../config/themes.js";
import { formatCost } from "../utils/format.js";
import { isNarrow, isVeryNarrow } from "../utils/responsive.js";

interface OverviewViewProps {
  data: TUIData;
  sortBy: SortType;
  sortDesc: boolean;
  selectedIndex: Accessor<number>;
  scrollOffset: Accessor<number>;
  height: number;
  width: number;
  chartMode?: ChartMode;
  chartResolution?: Resolution;
  showRateWhiskers?: boolean;
  intervalBuckets?: IntervalBucket[];
  selectedIntervalIndex?: number;
  onIntervalSelect?: (index: number) => void;
  colorPalette?: ColorPaletteName;
}

export function OverviewView(props: OverviewViewProps) {
  const safeHeight = () => Math.max(props.height, 12);
  const chartHeight = () => Math.max(5, Math.floor(safeHeight() * 0.35));
  const listHeight = () => Math.max(4, safeHeight() - chartHeight() - 4);
  const itemsPerPage = () => Math.max(1, Math.floor(listHeight() / 2));

  const isNarrowTerminal = () => isNarrow(props.width);
  const isVeryNarrowTerminal = () => isVeryNarrow(props.width);

  const legendModelLimit = () => isVeryNarrowTerminal() ? 3 : 5;
  const topModelsForLegend = () => props.data.topModels.slice(0, legendModelLimit()).map(m => m.modelId);

  const maxModelNameWidth = () => isVeryNarrowTerminal() ? 20 : isNarrowTerminal() ? 30 : 50;

  const sortedModels = createMemo(() => {
    const models = [...props.data.topModels];
    return models.sort((a, b) => {
      let cmp = 0;
      if (props.sortBy === "cost") cmp = a.cost - b.cost;
      else if (props.sortBy === "tokens") cmp = a.totalTokens - b.totalTokens;
      return props.sortDesc ? -cmp : cmp;
    });
  });

  const totalForPercentage = createMemo(() => {
    const models = props.data.topModels;
    if (props.sortBy === "tokens") {
      return models.reduce((sum, m) => sum + m.totalTokens, 0) || 1;
    }
    return models.reduce((sum, m) => sum + m.cost, 0) || 1;
  });

  const getPercentage = (model: typeof props.data.topModels[0]) => {
    if (props.sortBy === "tokens") {
      return (model.totalTokens / totalForPercentage()) * 100;
    }
    return (model.cost / totalForPercentage()) * 100;
  };

  const visibleModels = () => sortedModels().slice(props.scrollOffset(), props.scrollOffset() + itemsPerPage());
  const totalModels = () => sortedModels().length;
  const endIndex = () => Math.min(props.scrollOffset() + visibleModels().length, totalModels());

  const showIntervalChart = () => 
    props.chartResolution !== undefined && 
    props.chartResolution !== '1d' && 
    props.intervalBuckets !== undefined && 
    props.intervalBuckets.length > 0;

  const chartWidth = () => props.width - 4;

  return (
    <box flexDirection="column" gap={1}>
      <box flexDirection="column">
        <Show 
          when={showIntervalChart()} 
          fallback={
            <>
              <BarChart data={props.data.chartData} width={chartWidth()} height={chartHeight()} />
              <Legend models={topModelsForLegend()} width={props.width} />
            </>
          }
        >
          <IntervalChart
            data={props.intervalBuckets!}
            mode={props.chartMode ?? 'bar'}
            width={chartWidth()}
            height={chartHeight()}
            selectedIndex={props.selectedIntervalIndex ?? 0}
            onSelect={props.onIntervalSelect ?? (() => {})}
            showWhiskers={props.showRateWhiskers ?? false}
            colorPalette={props.colorPalette ?? 'green'}
          />
        </Show>
      </box>

      <box flexDirection="column">
        <box flexDirection="row" justifyContent="space-between" marginBottom={0}>
          <text bold>{isVeryNarrowTerminal() ? "Top Models" : `Models by ${props.sortBy === "tokens" ? "Tokens" : "Cost"}`}</text>
          <box flexDirection="row">
            <text dim>{isVeryNarrowTerminal() ? "" : "Total: "}</text>
            <text fg="green">{formatCost(props.data.totalCost)}</text>
          </box>
        </box>

        <box flexDirection="column">
          <For each={visibleModels()}>
            {(model, i) => {
              const isActive = createMemo(() => i() === props.selectedIndex());
              
              return (
                <ModelRow
                  modelId={model.modelId}
                  tokens={{
                    input: model.inputTokens,
                    output: model.outputTokens,
                    cacheRead: model.cacheReadTokens,
                    cacheWrite: model.cacheWriteTokens,
                  }}
                  percentage={getPercentage(model)}
                  isActive={isActive()}
                  compact={isVeryNarrowTerminal()}
                  maxNameWidth={maxModelNameWidth()}
                />
              );
            }}
          </For>
        </box>

        <Show when={totalModels() > visibleModels().length}>
          <text dim>{`↓ ${props.scrollOffset() + 1}-${endIndex()} of ${totalModels()} models (↑↓ to scroll)`}</text>
        </Show>
      </box>
    </box>
  );
}
