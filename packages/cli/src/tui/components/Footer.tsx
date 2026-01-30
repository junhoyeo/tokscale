import {
  Show,
  createSignal,
  createEffect,
  onMount,
  onCleanup,
} from "solid-js";
import type {
  SourceType,
  SortType,
  TabType,
  LoadingPhase,
} from "../types/index.js";
import type { ColorPaletteName } from "../config/themes.js";
import type { TotalBreakdown } from "../hooks/useData.js";
import { getPalette } from "../config/themes.js";
import { formatTokens } from "../utils/format.js";
import { isVeryNarrow } from "../utils/responsive.js";

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
  autoRefreshEnabled?: boolean;
  autoRefreshMs?: number;
  width?: number;
  onSourceToggle?: (source: SourceType) => void;
  onSortChange?: (sort: SortType) => void;
  onPaletteChange?: () => void;
  onRefresh?: () => void;
}

function formatTimeAgo(timestamp: number, now: number): string {
  const seconds = Math.max(Math.floor((now - timestamp) / 1000), 0);
  if (seconds < 60) return `${seconds}s ago`;
  const minutes = Math.floor(seconds / 60);
  if (minutes < 60) return `${minutes}m ago`;
  const hours = Math.floor(minutes / 60);
  if (hours < 24) return `${hours}h ago`;
  const days = Math.floor(hours / 24);
  return `${days}d ago`;
}

function formatIntervalSeconds(ms: number | undefined): string {
  if (!ms || ms <= 0) return "0s";
  const seconds = Math.round(ms / 1000);
  if (seconds < 60) return `${seconds}s`;
  const minutes = Math.round(seconds / 60);
  return `${minutes}m`;
}

export function Footer(props: FooterProps) {
  const palette = () => getPalette(props.colorPalette);
  const isVeryNarrowTerminal = () => isVeryNarrow(props.width);
  const [now, setNow] = createSignal(Date.now());
  let nowInterval: ReturnType<typeof setInterval> | null = null;

  createEffect(() => {
    if (props.cacheTimestamp) {
      if (!nowInterval) {
        nowInterval = setInterval(() => setNow(Date.now()), 1000);
      }
    } else if (nowInterval) {
      clearInterval(nowInterval);
      nowInterval = null;
    }
  });

  onCleanup(() => {
    if (nowInterval) clearInterval(nowInterval);
  });

  const showScrollInfo = () =>
    props.activeTab === "overview" &&
    props.totalItems &&
    props.scrollStart !== undefined &&
    props.scrollEnd !== undefined;

  const totals = () =>
    props.totals || {
      input: 0,
      output: 0,
      cacheRead: 0,
      cacheWrite: 0,
      reasoning: 0,
      total: 0,
      cost: 0,
    };

  return (
    <box flexDirection="column" paddingX={1}>
      <box flexDirection="row" justifyContent="space-between">
        <box flexDirection="row" gap={1}>
          <SourceBadge
            name={isVeryNarrowTerminal() ? "1" : "1:OC"}
            source="opencode"
            enabled={props.enabledSources.has("opencode")}
            onToggle={props.onSourceToggle}
          />
          <SourceBadge
            name={isVeryNarrowTerminal() ? "2" : "2:CC"}
            source="claude"
            enabled={props.enabledSources.has("claude")}
            onToggle={props.onSourceToggle}
          />
          <SourceBadge
            name={isVeryNarrowTerminal() ? "3" : "3:CX"}
            source="codex"
            enabled={props.enabledSources.has("codex")}
            onToggle={props.onSourceToggle}
          />
          <SourceBadge
            name={isVeryNarrowTerminal() ? "4" : "4:CR"}
            source="cursor"
            enabled={props.enabledSources.has("cursor")}
            onToggle={props.onSourceToggle}
          />
          <SourceBadge
            name={isVeryNarrowTerminal() ? "5" : "5:GM"}
            source="gemini"
            enabled={props.enabledSources.has("gemini")}
            onToggle={props.onSourceToggle}
          />
          <SourceBadge
            name={isVeryNarrowTerminal() ? "6" : "6:AM"}
            source="amp"
            enabled={props.enabledSources.has("amp")}
            onToggle={props.onSourceToggle}
          />
          <SourceBadge
            name={isVeryNarrowTerminal() ? "7" : "7:DR"}
            source="droid"
            enabled={props.enabledSources.has("droid")}
            onToggle={props.onSourceToggle}
          />
          <SourceBadge
            name={isVeryNarrowTerminal() ? "8" : "8:CL"}
            source="openclaw"
            enabled={props.enabledSources.has("openclaw")}
            onToggle={props.onSourceToggle}
          />
          <Show when={!isVeryNarrowTerminal()} fallback={<text />}>
            <box flexDirection="row" gap={1}>
              <text dim>|</text>
              <SortButton
                label="Date"
                sortType="date"
                active={props.sortBy === "date"}
                onClick={props.onSortChange}
              />
              <SortButton
                label="Cost"
                sortType="cost"
                active={props.sortBy === "cost"}
                onClick={props.onSortChange}
              />
              <SortButton
                label="Tokens"
                sortType="tokens"
                active={props.sortBy === "tokens"}
                onClick={props.onSortChange}
              />
            </box>
          </Show>
          <Show when={showScrollInfo() && !isVeryNarrowTerminal()} fallback={<text />}>
            <box flexDirection="row" gap={1}>
              <text dim>|</text>
              <text
                dim
              >{`v ${props.scrollStart! + 1}-${props.scrollEnd} of ${props.totalItems}`}</text>
            </box>
          </Show>
        </box>
        <box flexDirection="row" gap={1}>
          <text fg="cyan">{formatTokens(totals().total)}</text>
          <text dim>tokens</text>
          <text dim>|</text>
          <text fg="green" bold>{`$${totals().cost.toFixed(2)}`}</text>
          <Show when={!isVeryNarrowTerminal()} fallback={<text />}>
            <text dim>({props.modelCount} models)</text>
          </Show>
        </box>
      </box>
      <box flexDirection="row" gap={1}>
        <Show
          when={props.statusMessage}
          fallback={
            <Show
              when={isVeryNarrowTerminal()}
              fallback={
                <box flexDirection="row">
                  <text dim>{"Up/Down scroll | Left/Right view | y copy |"}</text>
                  <box onMouseDown={props.onPaletteChange}>
                    <text fg="magenta">{`[p:${palette().name}]`}</text>
                  </box>
                  <text fg={props.autoRefreshEnabled ? "green" : "gray"}>
                    {`[Shift+R:auto update ${formatIntervalSeconds(props.autoRefreshMs)}]`}
                  </text>
                  <text dim>{"[-/+ interval] |"}</text>
                  <box onMouseDown={props.onRefresh}>
                    <text fg="yellow">[r:refresh]</text>
                  </box>
                  <text dim>{"e export | q quit"}</text>
                </box>
              }
            >
              <box flexDirection="row">
                <text dim>{"^v|<>|y|"}</text>
                <box onMouseDown={props.onPaletteChange}>
                  <text fg="magenta">[p]</text>
                </box>
                <text fg={props.autoRefreshEnabled ? "green" : "gray"}>
                  {`[Shift+R:auto update ${formatIntervalSeconds(props.autoRefreshMs)}]`}
                </text>
                <text dim>{"-+|"}</text>
                <box onMouseDown={props.onRefresh}>
                  <text fg="yellow">[r]</text>
                </box>
                <text dim>{"|e|q"}</text>
              </box>
            </Show>
          }
        >
          <text fg="green" bold>
            {props.statusMessage}
          </text>
        </Show>
      </box>
      <Show when={props.isRefreshing} fallback={<text />}>
        <LoadingStatusLine phase={props.loadingPhase} />
      </Show>
      <Show when={!props.isRefreshing && props.cacheTimestamp} fallback={<text />}>
        <box flexDirection="row" gap={1}>
          <text
            dim
          >{`Last updated: ${formatTimeAgo(props.cacheTimestamp!, now())}`}</text>
          <Show when={props.autoRefreshEnabled} fallback={<text />}>
            <text
              dim
            >{`| Auto: ${formatIntervalSeconds(props.autoRefreshMs)}`}</text>
          </Show>
        </box>
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

const SPINNER_COLORS = [
  "#00FFFF",
  "#00D7D7",
  "#00AFAF",
  "#008787",
  "#666666",
  "#666666",
];
const SPINNER_WIDTH = 6;
const SPINNER_HOLD_START = 20;
const SPINNER_HOLD_END = 6;
const SPINNER_TRAIL = 3;
const SPINNER_INTERVAL = 40;

const PHASE_MESSAGES: Record<LoadingPhase, string> = {
  idle: "Initializing...",
  "parsing-sources": "Scanning session data...",
  "loading-pricing": "Loading pricing data...",
  "finalizing-report": "Finalizing report...",
  complete: "Complete",
};

interface LoadingStatusLineProps {
  phase?: LoadingPhase;
}

function LoadingStatusLine(props: LoadingStatusLineProps) {
  const [frame, setFrame] = createSignal(0);

  onMount(() => {
    const id = setInterval(() => setFrame((f) => f + 1), SPINNER_INTERVAL);
    onCleanup(() => clearInterval(id));
  });

  const getSpinnerState = () => {
    const forwardFrames = SPINNER_WIDTH;
    const backwardFrames = SPINNER_WIDTH - 1;
    const totalCycle =
      forwardFrames + SPINNER_HOLD_END + backwardFrames + SPINNER_HOLD_START;
    const normalized = frame() % totalCycle;

    if (normalized < forwardFrames) {
      return { position: normalized, forward: true };
    } else if (normalized < forwardFrames + SPINNER_HOLD_END) {
      return { position: SPINNER_WIDTH - 1, forward: true };
    } else if (normalized < forwardFrames + SPINNER_HOLD_END + backwardFrames) {
      return {
        position:
          SPINNER_WIDTH - 2 - (normalized - forwardFrames - SPINNER_HOLD_END),
        forward: false,
      };
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

  const message = () =>
    props.phase ? PHASE_MESSAGES[props.phase] : "Refreshing...";

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
