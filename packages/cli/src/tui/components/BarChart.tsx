import { For, Show, createMemo } from "solid-js";

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

import { formatTokensCompact } from "../utils/format.js";

const BLOCKS = [" ", "▁", "▂", "▃", "▄", "▅", "▆", "▇", "█"];
const MONTH_NAMES = ["Jan", "Feb", "Mar", "Apr", "May", "Jun", "Jul", "Aug", "Sep", "Oct", "Nov", "Dec"];
const REPEAT_CACHE_MAX_SIZE = 256;

const repeatCache = new Map<string, string>();
function getRepeatedString(char: string, count: number): string {
  const key = `${char}:${count}`;
  let cached = repeatCache.get(key);
  if (!cached) {
    if (repeatCache.size >= REPEAT_CACHE_MAX_SIZE) {
      const firstKey = repeatCache.keys().next().value;
      if (firstKey) repeatCache.delete(firstKey);
    }
    cached = char.repeat(count);
    repeatCache.set(key, cached);
  }
  return cached;
}

export function BarChart(props: BarChartProps) {
  const data = () => props.data;
  const width = () => props.width;
  const height = () => props.height;

  const safeHeight = () => Math.max(height(), 1);
  
  const maxTotal = createMemo(() => {
    const arr = data();
    if (arr.length === 0) return 1;
    let max = arr[0].total;
    for (let i = 1; i < arr.length; i++) {
      if (arr[i].total > max) max = arr[i].total;
    }
    return Math.max(max, 1);
  });

  const chartWidth = () => Math.max(width() - 8, 20);
  const barWidth = () => Math.max(1, Math.floor(chartWidth() / Math.min(data().length, 52)));
  const visibleBars = () => Math.min(data().length, Math.floor(chartWidth() / barWidth()));
  const visibleData = createMemo(() => data().slice(-visibleBars()));

  const sortedModelsMap = createMemo(() => {
    const vd = visibleData();
    const map = new Map<string, { modelId: string; tokens: number; color: string }[]>();
    for (const point of vd) {
      const models = point.models ?? [];
      const sorted = [...models].sort((a, b) => a.modelId.localeCompare(b.modelId));
      map.set(point.date, sorted);
    }
    return map;
  });

  const rowIndices = createMemo(() => {
    const sh = safeHeight();
    const indices: number[] = new Array(sh);
    for (let i = 0; i < sh; i++) {
      indices[i] = sh - 1 - i;
    }
    return indices;
  });

  const dateLabels = createMemo(() => {
    const vd = visibleData();
    if (vd.length === 0) return [];
    
    const labelInterval = Math.max(1, Math.floor(vd.length / 3));
    const labels: string[] = [];
    
    for (let i = 0; i < vd.length; i += labelInterval) {
      const dateStr = vd[i].date;
      const parts = dateStr.split("-");
      if (parts.length === 3) {
        const month = parseInt(parts[1], 10);
        const day = parseInt(parts[2], 10);
        labels.push(`${MONTH_NAMES[month - 1]} ${day}`);
      } else {
        labels.push(dateStr.slice(5));
      }
    }
    return labels;
  });

  const axisWidth = () => Math.min(chartWidth(), visibleBars() * barWidth());
  const labelPadding = () => {
    const labels = dateLabels();
    return labels.length > 0 ? Math.floor(axisWidth() / labels.length) : 0;
  };

  const getBarContent = (point: ChartDataPoint, row: number): { char: string; color: string } => {
    const mt = maxTotal();
    const sh = safeHeight();
    const rowThreshold = ((row + 1) / sh) * mt;
    const prevThreshold = (row / sh) * mt;
    const thresholdDiff = rowThreshold - prevThreshold;
    const bw = barWidth();

    if (point.total <= prevThreshold) {
      return { char: getRepeatedString(" ", bw), color: "dim" };
    }

    const sortedModels = sortedModelsMap().get(point.date) ?? [];
    if (sortedModels.length === 0) {
      return { char: getRepeatedString(" ", bw), color: "dim" };
    }

    let currentHeight = 0;
    let maxOverlap = 0;
    let color = sortedModels[0].color;

    const rowStart = prevThreshold;
    const rowEnd = rowThreshold;

    for (const m of sortedModels) {
      const mStart = currentHeight;
      const mEnd = currentHeight + m.tokens;
      currentHeight += m.tokens;

      const overlapStart = Math.max(mStart, rowStart);
      const overlapEnd = Math.min(mEnd, rowEnd);
      const overlap = Math.max(0, overlapEnd - overlapStart);

      if (overlap > maxOverlap) {
        maxOverlap = overlap;
        color = m.color;
      }
    }

    if (point.total >= rowThreshold) {
      return { char: getRepeatedString("█", bw), color };
    }

    const ratio = thresholdDiff > 0 ? (point.total - prevThreshold) / thresholdDiff : 1;
    const blockIndex = Math.min(8, Math.max(1, Math.floor(ratio * 8)));
    return { char: getRepeatedString(BLOCKS[blockIndex], bw), color };
  };

  return (
    <Show when={data().length > 0} fallback={<text dim>No chart data</text>}>
      <box flexDirection="column">
        <text bold>Tokens per Day</text>
        <For each={rowIndices()}>
          {(row) => {
            const yLabel = row === safeHeight() - 1 ? formatTokensCompact(maxTotal()).padStart(6) : "      ";
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
          <text dim>{getRepeatedString("─", axisWidth())}</text>
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
