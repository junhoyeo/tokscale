import { Show, createMemo } from "solid-js";
import { TokenBreakdown, type TokenBreakdownData } from "./TokenBreakdown.js";
import { getModelColor } from "../utils/colors.js";

interface ModelRowProps {
  modelId: string;
  tokens: TokenBreakdownData;
  percentage?: number;
  isActive?: boolean;
  compact?: boolean;
  indent?: number;
  maxNameWidth?: number;
}

export function ModelRow(props: ModelRowProps) {
  const color = () => getModelColor(props.modelId);
  const bgColor = createMemo(() => props.isActive ? "blue" : undefined);
  
  const truncateName = (name: string) => {
    const max = props.maxNameWidth ?? 50;
    return name.length > max ? name.slice(0, max - 1) + "…" : name;
  };

  const indentStr = () => " ".repeat(props.indent ?? 0);

  return (
    <box flexDirection="column">
      <box flexDirection="row" backgroundColor={bgColor()}>
        <Show when={props.indent} fallback={<text />}>
          <text>{indentStr()}</text>
        </Show>
        <text fg={color()} bg={bgColor()}>●</text>
        <text fg={props.isActive ? "white" : undefined} bg={bgColor()}>{` ${truncateName(props.modelId)}`}</text>
        <Show when={props.percentage !== undefined} fallback={<text />}>
          <text dim bg={bgColor()}>{` (${props.percentage!.toFixed(1)}%)`}</text>
        </Show>
      </box>
      <box flexDirection="row">
        <TokenBreakdown 
          tokens={props.tokens} 
          compact={props.compact}
          indent={(props.indent ?? 0) + 2}
        />
      </box>
    </box>
  );
}
