import { Show } from "solid-js";
import type { SourceType, SortType, TabType } from "../App.js";
import type { ColorPaletteName } from "../config/themes.js";
import { getPalette } from "../config/themes.js";

interface FooterProps {
  enabledSources: Set<SourceType>;
  sortBy: SortType;
  totalCost: number;
  modelCount: number;
  activeTab: TabType;
  scrollStart?: number;
  scrollEnd?: number;
  totalItems?: number;
  colorPalette: ColorPaletteName;
}

export function Footer(props: FooterProps) {
  const formatCost = (cost: number) => {
    if (cost >= 1000) return `$${(cost / 1000).toFixed(1)}K`;
    return `$${cost.toFixed(2)}`;
  };

  const palette = () => getPalette(props.colorPalette);
  const showScrollInfo = () => 
    props.activeTab === "overview" && 
    props.totalItems && 
    props.scrollStart !== undefined && 
    props.scrollEnd !== undefined;

  return (
    <box flexDirection="column" paddingX={1}>
      <box flexDirection="row" justifyContent="space-between">
        <box flexDirection="row" gap={1}>
          <SourceBadge name="1:OC" enabled={props.enabledSources.has("opencode")} />
          <SourceBadge name="2:CC" enabled={props.enabledSources.has("claude")} />
          <SourceBadge name="3:CX" enabled={props.enabledSources.has("codex")} />
          <SourceBadge name="4:CR" enabled={props.enabledSources.has("cursor")} />
          <SourceBadge name="5:GM" enabled={props.enabledSources.has("gemini")} />
          <text dim>|</text>
          <text dim>Sort:</text>
          <text fg="white">{props.sortBy === "cost" ? "Cost" : props.sortBy === "name" ? "Name" : "Tokens"}</text>
          <Show when={showScrollInfo()}>
            <text dim>|</text>
            <text dim>{`↓ ${props.scrollStart! + 1}-${props.scrollEnd} of ${props.totalItems} models`}</text>
          </Show>
        </box>
        <box flexDirection="row" gap={1}>
          <text dim>Total:</text>
          <text fg="green" bold>{formatCost(props.totalCost)}</text>
          <text dim>({props.modelCount})</text>
        </box>
      </box>
      <box>
        <text dim>
          {`↑↓ scroll • tab/d view • c/n/t sort • 1-5 filter • p theme (${palette().name}) • r refresh • q quit`}
        </text>
      </box>
    </box>
  );
}

function SourceBadge(props: { name: string; enabled: boolean }) {
  return (
    <text fg={props.enabled ? "green" : "gray"}>
      {`[${props.enabled ? "●" : "○"}${props.name}]`}
    </text>
  );
}
