import { Show } from "solid-js";
import type { SourceType, SortType, TabType } from "../types/index.js";
import type { ColorPaletteName } from "../config/themes.js";
import type { TotalBreakdown } from "../hooks/useData.js";
import { getPalette } from "../config/themes.js";
import { formatTokens } from "../utils/format.js";
import { isNarrow, isVeryNarrow } from "../utils/responsive.js";

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
  width?: number;
  onSourceToggle?: (source: SourceType) => void;
  onSortChange?: (sort: SortType) => void;
  onPaletteChange?: () => void;
  onRefresh?: () => void;
}

export function Footer(props: FooterProps) {
  const palette = () => getPalette(props.colorPalette);
  const isNarrowTerminal = () => isNarrow(props.width);
  const isVeryNarrowTerminal = () => isVeryNarrow(props.width);
  
  const showScrollInfo = () => 
    props.activeTab === "overview" && 
    props.totalItems && 
    props.scrollStart !== undefined && 
    props.scrollEnd !== undefined;

  const totals = () => props.totals || { input: 0, output: 0, cacheRead: 0, cacheWrite: 0, reasoning: 0, total: 0, cost: 0 };

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
