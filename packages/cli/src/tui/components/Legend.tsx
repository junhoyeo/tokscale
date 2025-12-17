import { For, Show } from "solid-js";
import { 
  getModelColor,
  TOKEN_TYPE_COLORS,
  TOKEN_TYPE_LABELS,
  TOKEN_TYPE_FULL_LABELS,
  TOKEN_TYPE_ORDER,
  type TokenType
} from "../utils/colors.js";
import { isNarrow, isVeryNarrow, shouldUseAscii } from "../utils/responsive.js";

interface LegendProps {
  models: string[];
  width?: number;
  showTokenTypes?: boolean;
}

const LEGEND_CHARS = {
  bullet: "●",
  separator: "·",
  ellipsis: "…",
  block: "█",
};

const ASCII_LEGEND_CHARS = {
  bullet: "*",
  separator: "-",
  ellipsis: "...",
  block: "#",
};

export function Legend(props: LegendProps) {
  const isNarrowTerminal = () => isNarrow(props.width);
  const isVeryNarrowTerminal = () => isVeryNarrow(props.width);
  const isCompactMode = () => isNarrowTerminal() && !isVeryNarrowTerminal();
  const useAscii = shouldUseAscii();
  
  const chars = useAscii ? ASCII_LEGEND_CHARS : LEGEND_CHARS;

  const maxModelNameWidth = () => isVeryNarrowTerminal() ? 12 : isNarrowTerminal() ? 18 : 30;
  const truncateModelName = (name: string) => {
    const max = maxModelNameWidth();
    const ellipsisLen = useAscii ? 3 : 1;
    return name.length > max ? name.slice(0, max - ellipsisLen) + chars.ellipsis : name;
  };

  const models = () => props.models;
  const showTokenTypes = () => props.showTokenTypes ?? false;

  const renderTokenTypeLegend = () => (
    <box flexDirection="row" gap={1} flexWrap="wrap">
      <For each={TOKEN_TYPE_ORDER}>
        {(tokenType: TokenType, i) => (
          <box flexDirection="row" gap={0}>
            <text dim>[</text>
            <text fg={TOKEN_TYPE_COLORS[tokenType]}>{TOKEN_TYPE_LABELS[tokenType]}</text>
            <text dim>]</text>
            <text fg={TOKEN_TYPE_COLORS[tokenType]}>{chars.block}</text>
            <text>{` ${TOKEN_TYPE_FULL_LABELS[tokenType]}`}</text>
            <Show when={i() < TOKEN_TYPE_ORDER.length - 1}>
              <text dim>{`  ${chars.separator}`}</text>
            </Show>
          </box>
        )}
      </For>
    </box>
  );

  const renderModelLegend = () => (
    <box flexDirection="row" gap={1} flexWrap="wrap">
      <For each={models()}>
        {(modelId, i) => (
          <box flexDirection="row" gap={0}>
            <text fg={getModelColor(modelId)}>{chars.bullet}</text>
            <text>{` ${truncateModelName(modelId)}`}</text>
            <Show when={i() < models().length - 1}>
              <text dim>{isVeryNarrowTerminal() ? " " : `  ${chars.separator}`}</text>
            </Show>
          </box>
        )}
      </For>
    </box>
  );

  return (
    <Show when={!isCompactMode()}>
      <Show when={showTokenTypes()} fallback={
        <Show when={models().length > 0}>
          {renderModelLegend()}
        </Show>
      }>
        {renderTokenTypeLegend()}
      </Show>
    </Show>
  );
}
