import { Box, Text } from "ink";
import type { SourceType, SortType } from "../App.js";

interface FooterProps {
  enabledSources: Set<SourceType>;
  sortBy: SortType;
  totalCost: number;
  modelCount: number;
}

export function Footer({ enabledSources, sortBy, totalCost, modelCount }: FooterProps) {
  const formatCost = (cost: number) => {
    if (cost >= 1000) return `$${(cost / 1000).toFixed(1)}K`;
    return `$${cost.toFixed(2)}`;
  };

  return (
    <Box flexDirection="column" paddingX={1}>
      <Box justifyContent="space-between">
        <Box gap={1}>
          <Text dimColor>View:</Text>
          <Text>|</Text>
          <SourceBadge name="OpenCode" enabled={enabledSources.has("opencode")} />
          <SourceBadge name="Claude" enabled={enabledSources.has("claude")} />
          <SourceBadge name="Codex" enabled={enabledSources.has("codex")} />
          <SourceBadge name="Cursor" enabled={enabledSources.has("cursor")} />
          <SourceBadge name="Gemini" enabled={enabledSources.has("gemini")} />
          <Text>|</Text>
          <Text dimColor>Sort:</Text>
          <Text color="white">{sortBy === "cost" ? "Cost" : sortBy === "name" ? "Name" : "Tokens"} {sortBy === "name" ? "↑" : "↓"}</Text>
          <Text>|</Text>
          <Text dimColor>({modelCount} models)</Text>
        </Box>
        <Box gap={1}>
          <Text dimColor>Total:</Text>
          <Text color="green" bold>{formatCost(totalCost)}</Text>
        </Box>
      </Box>
      <Box>
        <Text dimColor>
          ↑/↓ navigate • d/tab cycle view • c/n/t sort • 1-5 filter • r refresh • q quit
        </Text>
      </Box>
    </Box>
  );
}

function SourceBadge({ name, enabled }: { name: string; enabled: boolean }) {
  return (
    <Text color={enabled ? "green" : "gray"}>
      [{enabled ? "●" : "○"} {name}]
    </Text>
  );
}
