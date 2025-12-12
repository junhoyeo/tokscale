import { Box, Text } from "ink";
import type { TUIData, SortType } from "../hooks/useData.js";

interface DailyViewProps {
  data: TUIData | null;
  sortBy: SortType;
  sortDesc: boolean;
  selectedIndex: number;
  height: number;
}

export function DailyView({ data, sortBy, sortDesc, selectedIndex, height }: DailyViewProps) {
  if (!data) return null;

  const sortedEntries = [...data.dailyEntries].sort((a, b) => {
    let cmp = 0;
    if (sortBy === "cost") cmp = a.cost - b.cost;
    else if (sortBy === "tokens") cmp = a.total - b.total;
    else cmp = a.date.localeCompare(b.date);
    return sortDesc ? -cmp : cmp;
  });

  const visibleEntries = sortedEntries.slice(0, height - 3);

  const formatNum = (n: number) => {
    if (n >= 1_000_000_000) return `${(n / 1_000_000_000).toFixed(1)}B`;
    if (n >= 1_000_000) return `${(n / 1_000_000).toFixed(1)}M`;
    if (n >= 1_000) return `${Math.floor(n / 1_000).toLocaleString()},${String(n % 1000).padStart(3, "0")}`;
    return n.toLocaleString();
  };

  const formatCost = (cost: number) => `$${cost.toFixed(2)}`;

  return (
    <Box flexDirection="column">
      <Box>
        <Text color="cyan" bold>
          {"  Date".padEnd(14)}
          {"Input".padStart(14)}
          {"Output".padStart(14)}
          {"Cache".padStart(14)}
          {"Total".padStart(16)}
          {"Cost".padStart(12)}
        </Text>
      </Box>
      <Box borderStyle="single" borderTop={false} borderLeft={false} borderRight={false} borderBottom borderColor="gray" />
      
      {visibleEntries.map((entry, i) => {
        const isSelected = i === selectedIndex;
        
        return (
          <Box key={entry.date}>
            <Text 
              backgroundColor={isSelected ? "blue" : undefined}
              color={isSelected ? "white" : undefined}
            >
              {entry.date.padEnd(14)}
              {formatNum(entry.input).padStart(14)}
              {formatNum(entry.output).padStart(14)}
              {formatNum(entry.cache).padStart(14)}
              {formatNum(entry.total).padStart(16)}
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
