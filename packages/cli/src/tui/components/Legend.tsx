import { For, Show } from "solid-js";
import { getModelColor } from "../utils/colors.js";

interface LegendProps {
  models: string[];
}

export function Legend(props: LegendProps) {
  const models = () => props.models;

  return (
    <Show when={models().length > 0}>
      <box flexDirection="row" gap={1} flexWrap="wrap">
        <For each={models()}>
          {(modelId, i) => (
            <box flexDirection="row" gap={0}>
              <text fg={getModelColor(modelId)}>●</text>
              <text>{` ${modelId}`}</text>
              <Show when={i() < models().length - 1}>
                <text dim>  ·</text>
              </Show>
            </box>
          )}
        </For>
      </box>
    </Show>
  );
}
