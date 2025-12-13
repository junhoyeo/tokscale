import { Show } from "solid-js";
import type { SourceType, SortType, TabType } from "../types/index.js";
import type { ColorPaletteName } from "../config/themes.js";
import type { TotalBreakdown } from "../hooks/useData.js";
import { getPalette } from "../config/themes.js";
import { formatTokens } from "../utils/format.js";

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
  onSourceToggle?: (source: SourceType) => void;
  onSortChange?: (sort: SortType) => void;
  onPaletteChange?: () => void;
  onRefresh?: () => void;
}

export function Footer(props: FooterProps) {
  const palette = () => getPalette(props.colorPalette);
  const showScrollInfo = () => 
    props.activeTab === "overview" && 
    props.totalItems && 
    props.scrollStart !== undefined && 
    props.scrollEnd !== undefined;

  const totals = () => props.totals || { input: 0, output: 0, cacheRead: 0, cacheWrite: 0, total: 0, cost: 0 };

  return (
    <box flexDirection="column" paddingX={1}>
      <box flexDirection="row" justifyContent="space-between">
        <box flexDirection="row" gap={1}>
          <SourceBadge name="1:OC" source="opencode" enabled={props.enabledSources.has("opencode")} onToggle={props.onSourceToggle} />
          <SourceBadge name="2:CC" source="claude" enabled={props.enabledSources.has("claude")} onToggle={props.onSourceToggle} />
          <SourceBadge name="3:CX" source="codex" enabled={props.enabledSources.has("codex")} onToggle={props.onSourceToggle} />
          <SourceBadge name="4:CR" source="cursor" enabled={props.enabledSources.has("cursor")} onToggle={props.onSourceToggle} />
          <SourceBadge name="5:GM" source="gemini" enabled={props.enabledSources.has("gemini")} onToggle={props.onSourceToggle} />
          <text dim>|</text>
          <SortButton label="Cost" sortType="cost" active={props.sortBy === "cost"} onClick={props.onSortChange} />
          <SortButton label="Name" sortType="name" active={props.sortBy === "name"} onClick={props.onSortChange} />
          <SortButton label="Tokens" sortType="tokens" active={props.sortBy === "tokens"} onClick={props.onSortChange} />
          <Show when={showScrollInfo()}>
            <text dim>|</text>
            <text dim>{`↓ ${props.scrollStart! + 1}-${props.scrollEnd} of ${props.totalItems}`}</text>
          </Show>
        </box>
        <box flexDirection="row" gap={1}>
          <text dim>Input:</text>
          <text fg="cyan">{formatTokens(totals().input)}</text>
          <text dim>Output:</text>
          <text fg="cyan">{formatTokens(totals().output)}</text>
          <text dim>Cache Read:</text>
          <text fg="cyan">{formatTokens(totals().cacheRead)}</text>
          <text dim>Cache Write:</text>
          <text fg="cyan">{formatTokens(totals().cacheWrite)}</text>
          <text dim>|</text>
          <text fg="green" bold>{`$${totals().cost.toFixed(2)}`}</text>
          <text dim>({props.modelCount} models)</text>
        </box>
      </box>
      <box flexDirection="row" gap={1}>
        <Show when={props.statusMessage} fallback={
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
