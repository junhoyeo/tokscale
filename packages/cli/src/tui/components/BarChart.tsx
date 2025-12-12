import { For, Show } from "solid-js";

export interface ChartDataPoint {
  date: string;
  models: { modelId: string; tokens: number; color: string }[];
  total: number;
}

interface BarChartProps {
  data: ChartDataPoint[];
  width: number;
  height: number;
}

const BLOCKS = [" ", "▁", "▂", "▃", "▄", "▅", "▆", "▇", "█"];

function formatNumber(n: number): string {
  if (n >= 1_000_000_000) return `${(n / 1_000_000_000).toFixed(1)}B`;
  if (n >= 1_000_000) return `${(n / 1_000_000).toFixed(1)}M`;
  if (n >= 1_000) return `${(n / 1_000).toFixed(0)}K`;
  return n.toString();
}

function getDominantColor(models: { modelId: string; tokens: number; color: string }[]): string {
  if (models.length === 0) return "white";
  return models.reduce((max, m) => (m.tokens > max.tokens ? m : max), models[0]).color;
}

export function BarChart(props: BarChartProps) {
  const data = () => props.data;
  const width = () => props.width;
  const height = () => props.height;

  const safeHeight = () => Math.max(height(), 1);
  const maxTotal = () => Math.max(...data().map((d) => d.total), 1);
  const chartWidth = () => Math.max(width() - 8, 20);
  const barWidth = () => Math.max(1, Math.floor(chartWidth() / Math.min(data().length, 52)));
  const visibleBars = () => Math.min(data().length, Math.floor(chartWidth() / barWidth()));
  const visibleData = () => data().slice(-visibleBars());

  const rowIndices = () => {
    const indices: number[] = [];
    for (let i = safeHeight() - 1; i >= 0; i--) {
      indices.push(i);
    }
    return indices;
  };

  const dateLabels = () => {
    const labels: string[] = [];
    const vd = visibleData();
    if (vd.length > 0) {
      const labelInterval = Math.max(1, Math.floor(vd.length / 3));
      for (let i = 0; i < vd.length; i += labelInterval) {
        const dateStr = vd[i].date;
        const d = new Date(dateStr);
        const label = isNaN(d.getTime())
          ? dateStr.slice(5)
          : d.toLocaleDateString("en-US", { month: "short", day: "numeric" });
        labels.push(label);
      }
    }
    return labels;
  };

  const axisWidth = () => Math.min(chartWidth(), visibleBars() * barWidth());
  const labelPadding = () => dateLabels().length > 0 ? Math.floor(axisWidth() / dateLabels().length) : 0;

  const getBarContent = (point: ChartDataPoint, row: number): { char: string; color: string } => {
    const mt = maxTotal();
    const sh = safeHeight();
    const rowThreshold = ((row + 1) / sh) * mt;
    const prevThreshold = (row / sh) * mt;
    const thresholdDiff = rowThreshold - prevThreshold;
    const bw = barWidth();

    if (point.total <= prevThreshold) {
      return { char: " ".repeat(bw), color: "dim" };
    }

    const color = getDominantColor(point.models);

    if (point.total >= rowThreshold) {
      return { char: "█".repeat(bw), color };
    }

    const ratio = thresholdDiff > 0 ? (point.total - prevThreshold) / thresholdDiff : 1;
    const blockIndex = Math.min(8, Math.max(1, Math.floor(ratio * 8)));
    return { char: BLOCKS[blockIndex].repeat(bw), color };
  };

  return (
    <Show when={data().length > 0} fallback={<text dim>No chart data</text>}>
      <box flexDirection="column">
        <text bold>Tokens per Day</text>
        <For each={rowIndices()}>
          {(row) => {
            const yLabel = row === safeHeight() - 1 ? formatNumber(maxTotal()).padStart(6) : "      ";
            return (
              <box flexDirection="row">
                <text dim>{yLabel}│</text>
                <For each={visibleData()}>
                  {(point) => {
                    const bar = getBarContent(point, row);
                    return bar.color === "dim" 
                      ? <text dim>{bar.char}</text>
                      : <text fg={bar.color}>{bar.char}</text>;
                  }}
                </For>
              </box>
            );
          }}
        </For>
        <box flexDirection="row">
          <text dim>{"     0│"}</text>
          <text dim>{"─".repeat(axisWidth())}</text>
        </box>
        <Show when={dateLabels().length > 0}>
          <box flexDirection="row">
            <text dim>{"       "}</text>
            <text dim>
              {dateLabels().map((l) => l.padEnd(labelPadding())).join("")}
            </text>
          </box>
        </Show>
      </box>
    </Show>
  );
}
