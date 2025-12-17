import { Show, createSignal, createMemo, onMount, onCleanup } from "solid-js";
import type { SourceType, SortType, TabType, LoadingPhase, IntervalBucket, ChartMode, Resolution } from "../types/index.js";
import type { ColorPaletteName } from "../config/themes.js";
import type { TotalBreakdown } from "../hooks/useData.js";
import { getPalette } from "../config/themes.js";
import { formatTokens, formatCost } from "../utils/format.js";
import { isNarrow, isVeryNarrow, isMicro } from "../utils/responsive.js";

interface FooterProps {
  enabledSources: Set<SourceType>;
  sortBy: SortType;
  totals?: TotalBreakdown;
  modelCount: number;
  activeTab: TabType;
  scrollStart?: number;
  scrollEnd?: number;
  totalItems?: number;
  colorPalette: ColorPaletteName;
  statusMessage?: string | null;
  isRefreshing?: boolean;
  loadingPhase?: LoadingPhase;
  cacheTimestamp?: number | null;
  width?: number;
  selectedBucket?: IntervalBucket;
  chartMode?: ChartMode;
  chartResolution?: Resolution;
  showWhiskers?: boolean;
  onSourceToggle?: (source: SourceType) => void;
  onSortChange?: (sort: SortType) => void;
  onPaletteChange?: () => void;
  onRefresh?: () => void;
}

function formatTimeAgo(timestamp: number): string {
  const seconds = Math.floor((Date.now() - timestamp) / 1000);
  if (seconds < 60) return `${seconds}s ago`;
  const minutes = Math.floor(seconds / 60);
  if (minutes < 60) return `${minutes}m ago`;
  const hours = Math.floor(minutes / 60);
  if (hours < 24) return `${hours}h ago`;
  const days = Math.floor(hours / 24);
  return `${days}d ago`;
}

function formatIntervalTimeUTC(ms: number): string {
  const date = new Date(ms);
  const hours = date.getUTCHours().toString().padStart(2, '0');
  const minutes = date.getUTCMinutes().toString().padStart(2, '0');
  return `${hours}:${minutes}`;
}

function formatChartModeLabel(mode: ChartMode): string {
  return mode.charAt(0).toUpperCase() + mode.slice(1);
}

export function Footer(props: FooterProps) {
  const palette = () => getPalette(props.colorPalette);
  const isNarrowTerminal = () => isNarrow(props.width);
  const isVeryNarrowTerminal = () => isVeryNarrow(props.width);
  const isMicroTerminal = () => isMicro(props.width);
  
  const showScrollInfo = () => 
    props.activeTab === "overview" && 
    props.totalItems && 
    props.scrollStart !== undefined && 
    props.scrollEnd !== undefined;

  const totals = () => props.totals || { input: 0, output: 0, cacheRead: 0, cacheWrite: 0, reasoning: 0, total: 0, cost: 0 };

  const hasIntervalBucket = () => props.selectedBucket !== undefined;
  const hasModeIndicators = () => props.chartMode !== undefined;

  return (
    <box flexDirection="column" paddingX={1}>
      <box flexDirection="row" justifyContent="space-between">
        <box flexDirection="row" gap={1}>
          <SourceBadge name={isVeryNarrowTerminal() ? "1" : "1:OC"} source="opencode" enabled={props.enabledSources.has("opencode")} onToggle={props.onSourceToggle} />
          <SourceBadge name={isVeryNarrowTerminal() ? "2" : "2:CC"} source="claude" enabled={props.enabledSources.has("claude")} onToggle={props.onSourceToggle} />
          <SourceBadge name={isVeryNarrowTerminal() ? "3" : "3:CX"} source="codex" enabled={props.enabledSources.has("codex")} onToggle={props.onSourceToggle} />
          <SourceBadge name={isVeryNarrowTerminal() ? "4" : "4:CR"} source="cursor" enabled={props.enabledSources.has("cursor")} onToggle={props.onSourceToggle} />
          <SourceBadge name={isVeryNarrowTerminal() ? "5" : "5:GM"} source="gemini" enabled={props.enabledSources.has("gemini")} onToggle={props.onSourceToggle} />
          <Show when={!isVeryNarrowTerminal()}>
            <text dim>|</text>
            <SortButton label="Cost" sortType="cost" active={props.sortBy === "cost"} onClick={props.onSortChange} />
            <SortButton label="Tokens" sortType="tokens" active={props.sortBy === "tokens"} onClick={props.onSortChange} />
          </Show>
          <Show when={showScrollInfo() && !isVeryNarrowTerminal()}>
            <text dim>|</text>
            <text dim>{`↓ ${props.scrollStart! + 1}-${props.scrollEnd} of ${props.totalItems}`}</text>
          </Show>
        </box>
        <box flexDirection="row" gap={1}>
          <Show when={!isNarrowTerminal()}>
            <text dim>In:</text>
            <text fg="cyan">{formatTokens(totals().input)}</text>
            <text dim>Out:</text>
            <text fg="cyan">{formatTokens(totals().output)}</text>
            <text dim>Cache:</text>
            <text fg="cyan">{formatTokens(totals().cacheRead + totals().cacheWrite)}</text>
            <text dim>|</text>
          </Show>
          <text fg="green" bold>{`$${totals().cost.toFixed(2)}`}</text>
          <Show when={!isVeryNarrowTerminal()}>
            <text dim>({props.modelCount} models)</text>
          </Show>
        </box>
      </box>
      <box flexDirection="row" gap={1}>
        <Show when={props.statusMessage} fallback={
          <Show when={isVeryNarrowTerminal()} fallback={
            <>
              <text dim>↑↓ scroll • ←→/tab view • y copy •</text>
              <box onMouseDown={props.onPaletteChange}>
                <text fg="magenta">{`[p:${palette().name}]`}</text>
              </box>
              <box onMouseDown={props.onRefresh}>
                <text fg="yellow">[r:refresh]</text>
              </box>
              <text dim>• e export • q quit</text>
            </>
          }>
            <text dim>↑↓•←→•y•</text>
            <box onMouseDown={props.onPaletteChange}>
              <text fg="magenta">[p]</text>
            </box>
            <box onMouseDown={props.onRefresh}>
              <text fg="yellow">[r]</text>
            </box>
            <text dim>•e•q</text>
          </Show>
        }>
          <text fg="green" bold>{props.statusMessage}</text>
        </Show>
      </box>
      <Show when={props.isRefreshing}>
        <LoadingStatusLine phase={props.loadingPhase} />
      </Show>
      <Show when={!props.isRefreshing && props.cacheTimestamp}>
        <box flexDirection="row">
          <text dim>{`Last updated: ${formatTimeAgo(props.cacheTimestamp!)}`}</text>
        </box>
      </Show>
      <Show when={hasIntervalBucket()}>
        <IntervalDetailLine
          bucket={props.selectedBucket!}
          width={props.width}
        />
      </Show>
      <Show when={hasModeIndicators()}>
        <ModeIndicators
          chartMode={props.chartMode!}
          resolution={props.chartResolution}
          showWhiskers={props.showWhiskers}
          width={props.width}
        />
      </Show>
    </box>
  );
}

interface SourceBadgeProps {
  name: string;
  source: SourceType;
  enabled: boolean;
  onToggle?: (source: SourceType) => void;
}

function SourceBadge(props: SourceBadgeProps) {
  const handleClick = () => props.onToggle?.(props.source);

  return (
    <box onMouseDown={handleClick}>
      <text fg={props.enabled ? "green" : "gray"}>
        {`[${props.enabled ? "●" : "○"}${props.name}]`}
      </text>
    </box>
  );
}

interface SortButtonProps {
  label: string;
  sortType: SortType;
  active: boolean;
  onClick?: (sort: SortType) => void;
}

function SortButton(props: SortButtonProps) {
  const handleClick = () => props.onClick?.(props.sortType);

  return (
    <box onMouseDown={handleClick}>
      <text fg={props.active ? "white" : "gray"} bold={props.active}>
        {props.label}
      </text>
    </box>
  );
}

const SPINNER_COLORS = ["#00FFFF", "#00D7D7", "#00AFAF", "#008787", "#666666", "#666666"];
const SPINNER_WIDTH = 6;
const SPINNER_HOLD_START = 20;
const SPINNER_HOLD_END = 6;
const SPINNER_TRAIL = 3;
const SPINNER_INTERVAL = 40;

const PHASE_MESSAGES: Record<LoadingPhase, string> = {
  "idle": "Initializing...",
  "loading-pricing": "Loading pricing data...",
  "syncing-cursor": "Syncing Cursor data...",
  "parsing-sources": "Parsing session files...",
  "finalizing-report": "Finalizing report...",
  "complete": "Complete",
};

interface LoadingStatusLineProps {
  phase?: LoadingPhase;
}

function LoadingStatusLine(props: LoadingStatusLineProps) {
  const [frame, setFrame] = createSignal(0);

  onMount(() => {
    const id = setInterval(() => setFrame(f => f + 1), SPINNER_INTERVAL);
    onCleanup(() => clearInterval(id));
  });

  const getSpinnerState = () => {
    const forwardFrames = SPINNER_WIDTH;
    const backwardFrames = SPINNER_WIDTH - 1;
    const totalCycle = forwardFrames + SPINNER_HOLD_END + backwardFrames + SPINNER_HOLD_START;
    const normalized = frame() % totalCycle;

    if (normalized < forwardFrames) {
      return { position: normalized, forward: true };
    } else if (normalized < forwardFrames + SPINNER_HOLD_END) {
      return { position: SPINNER_WIDTH - 1, forward: true };
    } else if (normalized < forwardFrames + SPINNER_HOLD_END + backwardFrames) {
      return { position: SPINNER_WIDTH - 2 - (normalized - forwardFrames - SPINNER_HOLD_END), forward: false };
    }
    return { position: 0, forward: false };
  };

  const getCharProps = (index: number) => {
    const { position, forward } = getSpinnerState();
    const distance = forward ? position - index : index - position;
    if (distance >= 0 && distance < SPINNER_TRAIL) {
      return { char: "■", color: SPINNER_COLORS[distance] };
    }
    return { char: "⬝", color: "#444444" };
  };

  const message = () => props.phase ? PHASE_MESSAGES[props.phase] : "Refreshing...";

  return (
    <box flexDirection="row" gap={1}>
      <box flexDirection="row" gap={0}>
        {Array.from({ length: SPINNER_WIDTH }, (_, i) => {
          const { char, color } = getCharProps(i);
          return <text fg={color}>{char}</text>;
        })}
      </box>
      <text dim>{message()}</text>
    </box>
  );
}

interface IntervalDetailLineProps {
  bucket: IntervalBucket;
  width?: number;
}

function IntervalDetailLine(props: IntervalDetailLineProps) {
  const isNarrowTerminal = () => isNarrow(props.width);
  const isVeryNarrowTerminal = () => isVeryNarrow(props.width);
  const isMicroTerminal = () => isMicro(props.width);

  const timeRange = () => {
    const start = formatIntervalTimeUTC(props.bucket.startMs);
    const end = formatIntervalTimeUTC(props.bucket.endMs);
    return `${start}-${end} UTC`;
  };

  const bucketTotal = () => {
    const t = props.bucket.totals;
    return t.input + t.output + t.cacheRead + t.cacheWrite + t.reasoning;
  };

  if (isMicroTerminal()) {
    return (
      <box flexDirection="row" gap={1}>
        <text fg="cyan">{timeRange()}</text>
        <text dim>|</text>
        <text fg="green">{formatCost(props.bucket.cost)}</text>
      </box>
    );
  }

  if (isVeryNarrowTerminal()) {
    return (
      <box flexDirection="row" gap={1}>
        <text fg="cyan">{timeRange()}</text>
        <text dim>|</text>
        <text fg="white">{formatTokens(bucketTotal())}</text>
        <text dim>|</text>
        <text fg="green">{formatCost(props.bucket.cost)}</text>
        <text dim>|</text>
        <text dim>{props.bucket.messages} msgs</text>
      </box>
    );
  }

  if (isNarrowTerminal()) {
    return (
      <box flexDirection="row" gap={1}>
        <text fg="cyan">{timeRange()}</text>
        <text dim>|</text>
        <text dim>In</text>
        <text fg="white">{formatTokens(props.bucket.totals.input)}</text>
        <text dim>Out</text>
        <text fg="white">{formatTokens(props.bucket.totals.output)}</text>
        <text dim>|</text>
        <text fg="green">{formatCost(props.bucket.cost)}</text>
        <text dim>|</text>
        <text dim>{props.bucket.messages} msgs</text>
      </box>
    );
  }

  return (
    <box flexDirection="row" gap={1}>
      <text fg="cyan">{timeRange()}</text>
      <text dim>|</text>
      <text dim>In</text>
      <text fg="white">{formatTokens(props.bucket.totals.input)}</text>
      <text dim>Out</text>
      <text fg="white">{formatTokens(props.bucket.totals.output)}</text>
      <text dim>CR</text>
      <text fg="white">{formatTokens(props.bucket.totals.cacheRead)}</text>
      <text dim>|</text>
      <text dim>Total</text>
      <text fg="white">{formatTokens(bucketTotal())}</text>
      <text dim>|</text>
      <text fg="green">{formatCost(props.bucket.cost)}</text>
      <text dim>|</text>
      <text dim>{props.bucket.messages} msgs</text>
    </box>
  );
}

interface ModeIndicatorsProps {
  chartMode: ChartMode;
  resolution?: Resolution;
  showWhiskers?: boolean;
  width?: number;
}

function ModeIndicators(props: ModeIndicatorsProps) {
  const isVeryNarrowTerminal = () => isVeryNarrow(props.width);
  const isMicroTerminal = () => isMicro(props.width);

  const modeLabel = () => formatChartModeLabel(props.chartMode);
  const resolutionLabel = () => props.resolution || '1d';
  const whiskersLabel = () => props.showWhiskers ? 'On' : 'Off';

  if (isMicroTerminal()) {
    return (
      <box flexDirection="row" gap={1}>
        <text fg="blue">{`[m:${modeLabel().charAt(0)}]`}</text>
        <text fg="yellow">{`[z:${resolutionLabel()}]`}</text>
      </box>
    );
  }

  if (isVeryNarrowTerminal()) {
    return (
      <box flexDirection="row" gap={1}>
        <text fg="blue">{`[m:${modeLabel()}]`}</text>
        <text fg="yellow">{`[z:${resolutionLabel()}]`}</text>
        <text fg="magenta">{`[w:${whiskersLabel().charAt(0)}]`}</text>
      </box>
    );
  }

  return (
    <box flexDirection="row" gap={1}>
      <text fg="blue">{`[m:${modeLabel()}]`}</text>
      <text fg="yellow">{`[z:${resolutionLabel()}]`}</text>
      <text fg="magenta">{`[w:${whiskersLabel()}]`}</text>
    </box>
  );
}
