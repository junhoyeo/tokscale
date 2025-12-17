import { createMemo, For, Show } from "solid-js";
import type { IntervalBucket, ChartMode } from "../types/index.js";
import type { ColorPaletteName } from "../config/themes.js";
import { isNarrow, isVeryNarrow, isMicro, shouldUseAscii } from "../utils/responsive.js";
import { 
  TOKEN_TYPE_COLORS as TOKEN_COLORS,
  TOKEN_TYPE_LABELS as TOKEN_LABELS,
  TOKEN_TYPE_ORDER as TOKEN_ORDER
} from "../utils/colors.js";

export interface IntervalChartProps {
  data: IntervalBucket[];
  mode: ChartMode;
  width: number;
  height: number;
  selectedIndex: number;
  onSelect: (index: number) => void;
  showWhiskers: boolean;
  colorPalette: ColorPaletteName;
}

const CANDLE_CHARS = {
  wick: '│',
  bodyFull: '█',
  bodyHollow: '▒',
  doji: '─',
};

const ASCII_CANDLE_CHARS = {
  wick: '|',
  bodyFull: '#',
  bodyHollow: '=',
  doji: '-',
};

const CANDLE_COLORS = {
  increasing: '#0077BB',
  decreasing: '#EE7733',
};

const DOJI_THRESHOLD = 0.05;

const BLOCKS = [" ", "\u2581", "\u2582", "\u2583", "\u2584", "\u2585", "\u2586", "\u2587", "\u2588"];
const ASCII_BLOCKS = [" ", ".", ":", "-", "=", "+", "#", "#", "#"];

const AXIS_CHARS = {
  vertical: "\u2502",
  horizontal: "\u2500",
  marker: "▼",
};

const ASCII_AXIS_CHARS = {
  vertical: "|",
  horizontal: "-",
  marker: "v",
};

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

export function IntervalChart(props: IntervalChartProps) {
  const data = () => props.data;
  const width = () => props.width;
  const height = () => props.height;

  const isNarrowTerminal = () => isNarrow(width());
  const isVeryNarrowTerminal = () => isVeryNarrow(width());
  const isMicroTerminal = () => isMicro(width());
  const useAscii = shouldUseAscii();
  
  // Responsive mode detection
  // ≥80 cols: Full mode (chart + Y-axis + legend + detailed footer)
  // 60-79 cols: Compact mode (abbreviated labels, no legend)
  // 40-59 cols: Micro mode (sparkline + summary only)
  // <40 cols: Text fallback ("Too narrow for chart")
  const isFullMode = () => !isNarrowTerminal();
  const isCompactMode = () => isNarrowTerminal() && !isVeryNarrowTerminal();
  const isMicroMode = () => isVeryNarrowTerminal() && !isMicroTerminal();
  const isTooNarrow = () => isMicroTerminal();
  
  const chars = useAscii ? ASCII_CANDLE_CHARS : CANDLE_CHARS;
  const blocks = useAscii ? ASCII_BLOCKS : BLOCKS;
  const axis = useAscii ? ASCII_AXIS_CHARS : AXIS_CHARS;

  const yAxisWidth = () => isVeryNarrowTerminal() ? 0 : (isNarrowTerminal() ? 6 : 8);
  const chartWidth = () => Math.max(10, width() - yAxisWidth() - 2);
  const chartHeight = () => Math.max(3, height() - 2);

  const maxTotal = createMemo(() => {
    const arr = data();
    if (arr.length === 0) return 1;
    let max = 0;
    for (const bucket of arr) {
      const total = bucket.totals.input + bucket.totals.output + 
                    bucket.totals.cacheRead + bucket.totals.cacheWrite + bucket.totals.reasoning;
      if (total > max) max = total;
    }
    return Math.max(max, 1);
  });

  const maxRate = createMemo(() => {
    const arr = data();
    let max = 0;
    for (const bucket of arr) {
      if (bucket.rateStats && bucket.rateStats.maxTokensPerMin > max) {
        max = bucket.rateStats.maxTokensPerMin;
      }
    }
    return Math.max(max, 1);
  });

  const summaryStats = createMemo(() => {
    const arr = data();
    if (arr.length === 0) return { total: 0, buckets: 0, cost: 0 };
    let total = 0;
    let cost = 0;
    for (const bucket of arr) {
      total += bucket.totals.input + bucket.totals.output + 
               bucket.totals.cacheRead + bucket.totals.cacheWrite + bucket.totals.reasoning;
      cost += bucket.cost;
    }
    return { total, buckets: arr.length, cost };
  });

  const sparkline = createMemo(() => {
    const arr = data();
    if (arr.length === 0) return "";
    
    const mt = maxTotal();
    const availableWidth = Math.max(10, width() - 2);
    const bucketCount = Math.min(arr.length, availableWidth);
    const step = Math.max(1, Math.floor(arr.length / bucketCount));
    
    let result = "";
    
    for (let i = 0; i < arr.length && result.length < availableWidth; i += step) {
      const bucket = arr[i];
      const total = bucket.totals.input + bucket.totals.output + 
                    bucket.totals.cacheRead + bucket.totals.cacheWrite + bucket.totals.reasoning;
      const ratio = total / mt;
      const blockIndex = Math.min(8, Math.max(0, Math.floor(ratio * 8)));
      result += blocks[blockIndex];
    }
    return result;
  });

  const barWidth = () => Math.max(1, Math.floor(chartWidth() / Math.min(data().length, 52)));
  const visibleBuckets = () => Math.floor(chartWidth() / barWidth());

  const visibleData = createMemo(() => {
    const count = visibleBuckets();
    const arr = data();
    if (arr.length <= count) return arr;
    return arr.slice(-count);
  });

  const rowIndices = createMemo(() => {
    const h = chartHeight();
    const indices: number[] = new Array(h);
    for (let i = 0; i < h; i++) {
      indices[i] = h - 1 - i;
    }
    return indices;
  });

  const formatYValue = (value: number): string => {
    if (value >= 1_000_000) return `${(value / 1_000_000).toFixed(1)}M`;
    if (value >= 1_000) return `${Math.round(value / 1_000)}K`;
    return String(value);
  };

  const formatCompactValue = (value: number): string => {
    if (value >= 1_000_000) return `${(value / 1_000_000).toFixed(1)}M`;
    if (value >= 1_000) return `${(value / 1_000).toFixed(0)}K`;
    return String(value);
  };

  const getWhiskerChar = (bw: number): string => {
    if (bw === 1) return chars.wick;
    const center = Math.floor(bw / 2);
    return " ".repeat(center) + chars.wick + " ".repeat(bw - center - 1);
  };

  const isRowInWhiskerRange = (bucket: IntervalBucket, row: number, h: number): boolean => {
    if (!props.showWhiskers || !bucket.rateStats) return false;
    const mr = maxRate();
    if (mr <= 0) return false;
    
    const minRateFrac = bucket.rateStats.minTokensPerMin / mr;
    const maxRateFrac = bucket.rateStats.maxTokensPerMin / mr;
    const rowBottomFrac = row / h;
    const rowTopFrac = (row + 1) / h;
    
    return maxRateFrac >= rowBottomFrac && minRateFrac < rowTopFrac;
  };

  const getBarContent = (bucket: IntervalBucket, row: number): { char: string; color: string } => {
    const mt = maxTotal();
    const h = chartHeight();
    const bw = barWidth();
    
    const total = bucket.totals.input + bucket.totals.output + 
                  bucket.totals.cacheRead + bucket.totals.cacheWrite + bucket.totals.reasoning;
    
    const rowEnd = ((row + 1) / h) * mt;
    const rowStart = (row / h) * mt;
    
    const showWhiskerHere = isRowInWhiskerRange(bucket, row, h);
    
    if (total <= rowStart) {
      if (showWhiskerHere) {
        return { char: getWhiskerChar(bw), color: "cyan" };
      }
      return { char: getRepeatedString(" ", bw), color: "dim" };
    }
    
    let currentHeight = 0;
    let maxOverlap = 0;
    let color: string = TOKEN_COLORS.input;
    
    for (const tokenType of TOKEN_ORDER) {
      const tokenCount = bucket.totals[tokenType];
      if (tokenCount === 0) continue;
      
      const segmentStart = currentHeight;
      const segmentEnd = currentHeight + tokenCount;
      currentHeight += tokenCount;
      
      const overlapStart = Math.max(segmentStart, rowStart);
      const overlapEnd = Math.min(segmentEnd, rowEnd);
      const overlap = Math.max(0, overlapEnd - overlapStart);
      
      if (overlap > maxOverlap) {
        maxOverlap = overlap;
        color = TOKEN_COLORS[tokenType];
      }
    }
    
    if (total >= rowEnd) {
      return { char: getRepeatedString(blocks[8], bw), color };
    }
    
    const thresholdDiff = rowEnd - rowStart;
    const ratio = thresholdDiff > 0 ? (total - rowStart) / thresholdDiff : 1;
    const blockIndex = Math.min(8, Math.max(1, Math.floor(ratio * 8)));
    return { char: getRepeatedString(blocks[blockIndex], bw), color };
  };

  const getCandleContent = (
    bucket: IntervalBucket,
    bucketIndex: number,
    row: number
  ): { char: string; color: string } => {
    const h = chartHeight();
    const bw = barWidth();
    const mr = maxRate();
    const vd = visibleData();

    if (!bucket.rateStats) {
      return getBarContent(bucket, row);
    }

    const currentAvg = bucket.rateStats.avgTokensPerMin;
    const high = bucket.rateStats.maxTokensPerMin;
    const low = bucket.rateStats.minTokensPerMin;

    let prevAvg = currentAvg;
    if (bucketIndex > 0) {
      const prevBucket = vd[bucketIndex - 1];
      if (prevBucket?.rateStats) {
        prevAvg = prevBucket.rateStats.avgTokensPerMin;
      }
    }

    const open = prevAvg;
    const close = currentAvg;

    const openFrac = open / mr;
    const closeFrac = close / mr;
    const highFrac = high / mr;
    const lowFrac = low / mr;

    const rowBottomFrac = row / h;
    const rowTopFrac = (row + 1) / h;

    const bodyTop = Math.max(openFrac, closeFrac);
    const bodyBottom = Math.min(openFrac, closeFrac);

    const isDoji = bodyTop > 0 
      ? Math.abs(open - close) / Math.max(open, close) < DOJI_THRESHOLD 
      : true;
    const isIncreasing = close >= open;
    const color = isIncreasing ? CANDLE_COLORS.increasing : CANDLE_COLORS.decreasing;

    const inWickRange = highFrac >= rowBottomFrac && lowFrac < rowTopFrac;
    const inBodyRange = bodyTop >= rowBottomFrac && bodyBottom < rowTopFrac;

    if (!inWickRange) {
      return { char: getRepeatedString(" ", bw), color: "dim" };
    }

    const centerChar = (char: string): string => {
      if (bw === 1) return char;
      const center = Math.floor(bw / 2);
      return " ".repeat(center) + char + " ".repeat(bw - center - 1);
    };

    if (isDoji && rowTopFrac > bodyBottom && rowBottomFrac < bodyTop) {
      return { char: centerChar(chars.doji), color };
    }

    if (inBodyRange) {
      const bodyChar = isIncreasing ? chars.bodyFull : chars.bodyHollow;
      return { char: centerChar(bodyChar), color };
    }

    return { char: centerChar(chars.wick), color };
  };

  const getRowContent = (
    bucket: IntervalBucket,
    bucketIndex: number,
    row: number
  ): { char: string; color: string } => {
    if (props.mode === 'candle') {
      return getCandleContent(bucket, bucketIndex, row);
    }
    return getBarContent(bucket, row);
  };

  const timeLabels = createMemo(() => {
    const vd = visibleData();
    if (vd.length === 0) return [];
    
    const labelCount = isVeryNarrowTerminal() ? 2 : (isNarrowTerminal() ? 3 : 5);
    const labelInterval = Math.max(1, Math.floor(vd.length / labelCount));
    const labels: { text: string; width: number }[] = [];
    
    for (let i = 0; i < vd.length; i += labelInterval) {
      const bucket = vd[i];
      const date = new Date(bucket.startMs);
      const hours = date.getHours().toString().padStart(2, '0');
      const minutes = date.getMinutes().toString().padStart(2, '0');
      const label = `${hours}:${minutes}`;
      labels.push({ text: label, width: Math.floor(chartWidth() / labelCount) });
    }
    return labels;
  });

  const chartTitle = () => {
    if (isMicroMode() || isCompactMode()) return "Tokens";
    const modeLabel = props.mode === 'candle' ? 'Candle' : (props.mode === 'hybrid' ? 'Hybrid' : 'Volume');
    return `${modeLabel} Chart`;
  };

  const renderTooNarrow = () => (
    <box flexDirection="column">
      <text dim>Too narrow for chart</text>
      <text dim>Resize to {"\u2265"}40 cols</text>
    </box>
  );

  const renderMicroMode = () => {
    const stats = summaryStats();
    return (
      <box flexDirection="column">
        <text bold>Tokens</text>
        <text fg={TOKEN_COLORS.input}>{sparkline()}</text>
        <text dim>
          {formatCompactValue(stats.total)} | ${stats.cost.toFixed(2)}
        </text>
      </box>
    );
  };

  const renderLegend = () => (
    <box flexDirection="row">
      <text dim>Legend: </text>
      <For each={TOKEN_ORDER}>
        {(tokenType) => (
          <>
            <text fg={TOKEN_COLORS[tokenType]}>{useAscii ? "#" : "█"}</text>
            <text dim>{TOKEN_LABELS[tokenType]} </text>
          </>
        )}
      </For>
    </box>
  );

  const renderDetailedFooter = () => {
    const stats = summaryStats();
    return (
      <box flexDirection="row">
        <text dim>
          {stats.buckets} intervals | {formatCompactValue(stats.total)} tokens | ${stats.cost.toFixed(2)}
        </text>
      </box>
    );
  };

  const renderChart = () => (
    <box flexDirection="column">
      <text bold>{chartTitle()}</text>
      
      <For each={rowIndices()}>
        {(row) => {
          const yLabelWidth = yAxisWidth();
          const isTopRow = row === chartHeight() - 1;
          const yAxisMax = props.mode === 'candle' ? maxRate() : maxTotal();
          const yLabel = isTopRow && yLabelWidth > 0
            ? formatYValue(yAxisMax).padStart(yLabelWidth - 1)
            : (yLabelWidth > 0 ? " ".repeat(yLabelWidth - 1) : "");
          
          return (
            <box flexDirection="row">
              <Show when={yAxisWidth() > 0}>
                <text dim>{yLabel}{axis.vertical}</text>
              </Show>
              <For each={visibleData()}>
                {(bucket, index) => {
                  const content = getRowContent(bucket, index(), row);
                  const isSelected = () => index() === props.selectedIndex;
                  const handleClick = () => {
                    if (props.onSelect && index() >= 0 && index() < visibleData().length) {
                      props.onSelect(index());
                    }
                  };
                  return (
                    <box onMouseDown={handleClick}>
                      <Show when={isSelected()} fallback={
                        content.color === "dim" 
                          ? <text dim>{content.char}</text>
                          : <text fg={content.color}>{content.char}</text>
                      }>
                        <text bg="yellow" fg="black" bold>{content.char}</text>
                      </Show>
                    </box>
                  );
                }}
              </For>
            </box>
          );
        }}
      </For>
      
      <box flexDirection="row">
        <Show when={yAxisWidth() > 0}>
          <text dim>{isVeryNarrowTerminal() ? "" : "     0"}{axis.vertical}</text>
        </Show>
        <text dim>{getRepeatedString(axis.horizontal, Math.min(chartWidth(), visibleBuckets() * barWidth()))}</text>
      </box>
      
      <Show when={props.selectedIndex >= 0 && props.selectedIndex < visibleData().length}>
        <box flexDirection="row">
          <Show when={yAxisWidth() > 0}>
            <text dim>{" ".repeat(yAxisWidth())}</text>
          </Show>
          <text fg="yellow" bold>
            {(() => {
              const bw = barWidth();
              const idx = props.selectedIndex;
              const before = idx * bw;
              const marker = bw === 1 ? axis.marker : " ".repeat(Math.floor(bw / 2)) + axis.marker + " ".repeat(bw - Math.floor(bw / 2) - 1);
              const after = Math.max(0, visibleBuckets() * bw - before - bw);
              return " ".repeat(before) + marker + " ".repeat(after);
            })()}
          </text>
        </box>
      </Show>
      
      <Show when={timeLabels().length > 0}>
        <box flexDirection="row">
          <Show when={yAxisWidth() > 0}>
            <text dim>{" ".repeat(yAxisWidth())}</text>
          </Show>
          <text dim>
            {timeLabels().map((l) => l.text.padEnd(l.width)).join("")}
          </text>
        </box>
      </Show>
      
      <Show when={isFullMode()}>
        {renderLegend()}
      </Show>
      
      <Show when={isFullMode()}>
        {renderDetailedFooter()}
      </Show>
    </box>
  );

  return (
    <Show when={data().length > 0} fallback={<text dim>No interval data available</text>}>
      <Show when={!isTooNarrow()} fallback={renderTooNarrow()}>
        <Show when={!isMicroMode()} fallback={renderMicroMode()}>
          {renderChart()}
        </Show>
      </Show>
    </Show>
  );
}

export default IntervalChart;
