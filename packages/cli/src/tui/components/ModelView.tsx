import { Box, Text } from "ink";
import type { TUIData, SortType } from "../hooks/useData.js";

interface ModelViewProps {
  data: TUIData | null;
  sortBy: SortType;
  sortDesc: boolean;
  selectedIndex: number;
  height: number;
}

export function ModelView({ data, sortBy, sortDesc, selectedIndex, height }: ModelViewProps) {
  if (!data) return null;

  const sortedEntries = [...data.modelEntries].sort((a, b) => {
    let cmp = 0;
    if (sortBy === "cost") cmp = a.cost - b.cost;
    else if (sortBy === "tokens") cmp = a.total - b.total;
    else cmp = a.model.localeCompare(b.model);
    return sortDesc ? -cmp : cmp;
  });

  const visibleEntries = sortedEntries.slice(0, height - 3);

  const formatNum = (n: number) => {
    if (n >= 1_000_000_000) return `${(n / 1_000_000_000).toFixed(1)}B`;
    if (n >= 1_000_000) return `${(n / 1_000_000).toFixed(1)}M`;
    if (n >= 1_000) return `${(n / 1_000).toFixed(0)}K`;
    return n.toLocaleString();
  };

  const formatCost = (cost: number) => `$${cost.toFixed(2)}`;

  return (
    <Box flexDirection="column">
      <Box>
        <Text color="cyan" bold>
          {"  Source/Model".padEnd(24)}
          {"Input".padStart(12)}
          {"Output".padStart(12)}
          {"Cache".padStart(12)}
          {"Total".padStart(14)}
          {"Cost".padStart(12)}
        </Text>
      </Box>
      <Box borderStyle="single" borderTop={false} borderLeft={false} borderRight={false} borderBottom borderColor="gray" />
      
      {visibleEntries.map((entry, i) => {
        const isSelected = i === selectedIndex;
        const sourceLabel = entry.source.charAt(0).toUpperCase() + entry.source.slice(1);
        const displayName = `${sourceLabel} ${entry.model}`.slice(0, 22);
        
        return (
          <Box key={`${entry.source}-${entry.model}`}>
            <Text 
              backgroundColor={isSelected ? "blue" : undefined}
              color={isSelected ? "white" : undefined}
            >
              {displayName.padEnd(24)}
              {formatNum(entry.input).padStart(12)}
              {formatNum(entry.output).padStart(12)}
              {formatNum(entry.cacheRead).padStart(12)}
              {formatNum(entry.total).padStart(14)}
            </Text>
            <Text 
              color="green" 
              backgroundColor={isSelected ? "blue" : undefined}
            >
              {formatCost(entry.cost).padStart(12)}
            </Text>
          </Box>
        );
      })}
    </Box>
  );
}
