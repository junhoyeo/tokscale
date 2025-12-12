import { getModelColor } from "../utils/colors.js";

interface ModelListItemProps {
  modelId: string;
  percentage: number;
  inputTokens: number;
  outputTokens: number;
  isSelected: boolean;
}

function formatTokens(n: number): string {
  if (n >= 1_000_000_000) return `${(n / 1_000_000_000).toFixed(1)}B`;
  if (n >= 1_000_000) return `${(n / 1_000_000).toFixed(1)}M`;
  if (n >= 1_000) return `${(n / 1_000).toFixed(1)}K`;
  return n.toString();
}

export function ModelListItem(props: ModelListItemProps) {
  const color = () => getModelColor(props.modelId);
  const bgColor = () => props.isSelected ? "blue" : undefined;

  return (
    <box flexDirection="column">
      <box flexDirection="row" backgroundColor={bgColor()}>
        <text fg={color()}>●</text>
        <text fg={props.isSelected ? "white" : undefined}>{` ${props.modelId} `}</text>
        <text dim>{`(${props.percentage.toFixed(1)}%)`}</text>
      </box>
      <text dim>{`  In: ${formatTokens(props.inputTokens)} · Out: ${formatTokens(props.outputTokens)}`}</text>
    </box>
  );
}
