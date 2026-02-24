import { For, Show } from "solid-js";
import { getModelColor } from "../utils/colors.js";
import { isNarrow, isVeryNarrow } from "../utils/responsive.js";

interface LegendProps {
  models: string[];
  width?: number;
}

export function Legend(props: LegendProps) {
  const isNarrowTerminal = () => isNarrow(props.width);
  const isVeryNarrowTerminal = () => isVeryNarrow(props.width);

  const maxModelNameWidth = () => isVeryNarrowTerminal() ? 12 : isNarrowTerminal() ? 18 : 30;
  const truncateModelName = (name: string) => {
    const max = maxModelNameWidth();
    return name.length > max ? name.slice(0, max - 1) + "…" : name;
  };

  const models = () => props.models;

  return (
    <Show when={models().length > 0}>
      <box flexDirection="row" gap={1} flexWrap="wrap">
        <For each={models()}>
          {(modelId, i) => (
            <box flexDirection="row" gap={0}>
              <text fg={getModelColor(modelId)}>●</text>
              <text>{` ${truncateModelName(modelId)}`}</text>
              <Show when={i() < models().length - 1} fallback={<></>}>
                <text dim>{isVeryNarrowTerminal() ? " " : "  ·"}</text>
              </Show>
            </box>
          )}
        </For>
      </box>
    </Show>
  );
}
