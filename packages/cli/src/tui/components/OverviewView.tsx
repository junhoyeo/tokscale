import { Show, For, createMemo, type Accessor } from "solid-js";
import { BarChart } from "./BarChart.js";
import { Legend } from "./Legend.js";
import type { TUIData, SortType } from "../hooks/useData.js";
import { formatCost, formatTokens, formatTokensCompact } from "../utils/format.js";
import { getModelColor } from "../utils/colors.js";
import { isNarrow, isVeryNarrow } from "../utils/responsive.js";

interface OverviewViewProps {
  data: TUIData;
  sortBy: SortType;
  sortDesc: boolean;
  selectedIndex: Accessor<number>;
  scrollOffset: Accessor<number>;
  height: number;
  width: number;
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
  const truncateModelName = (name: string) => {
    const max = maxModelNameWidth();
    return name.length > max ? name.slice(0, max - 1) + "…" : name;
  };

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

  return (
    <box flexDirection="column" gap={1}>
      <box flexDirection="column">
        <BarChart data={props.data.chartData} width={props.width - 4} height={chartHeight()} />
        <Legend models={topModelsForLegend()} width={props.width} />
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
              const bgColor = createMemo(() => isActive() ? "blue" : undefined);
              const color = () => getModelColor(model.modelId);
              
              return (
                <box flexDirection="column">
                  <box flexDirection="row" backgroundColor={bgColor()}>
                    <text fg={color()} bg={bgColor()}>●</text>
                    <text fg={isActive() ? "white" : undefined} bg={bgColor()}>{` ${truncateModelName(model.modelId)} `}</text>
                    <text dim bg={bgColor()}>{`(${getPercentage(model).toFixed(1)}%)`}</text>
                  </box>
                  <box flexDirection="row">
                    <Show when={isVeryNarrowTerminal()} fallback={
                      <>
                        <text fg="#666666">{"  In: "}</text><text fg="#AAAAAA">{formatTokens(model.inputTokens)}</text>
                        <text fg="#666666">{" · Out: "}</text><text fg="#AAAAAA">{formatTokens(model.outputTokens)}</text>
                        <text fg="#666666">{" · CR: "}</text><text fg="#AAAAAA">{formatTokens(model.cacheReadTokens)}</text>
                        <text fg="#666666">{" · CW: "}</text><text fg="#AAAAAA">{formatTokens(model.cacheWriteTokens)}</text>
                      </>
                    }>
                      <text fg="#666666">{"  "}</text>
                      <text fg="#AAAAAA">{formatTokensCompact(model.inputTokens)}</text>
                      <text fg="#666666">{"/"}</text>
                      <text fg="#AAAAAA">{formatTokensCompact(model.outputTokens)}</text>
                      <text fg="#666666">{"/"}</text>
                      <text fg="#AAAAAA">{formatTokensCompact(model.cacheReadTokens)}</text>
                      <text fg="#666666">{"/"}</text>
                      <text fg="#AAAAAA">{formatTokensCompact(model.cacheWriteTokens)}</text>
                    </Show>
                  </box>
                </box>
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
