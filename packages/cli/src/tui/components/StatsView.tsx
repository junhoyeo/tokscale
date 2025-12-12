import { Box, Text } from "ink";
import type { TUIData } from "../hooks/useData.js";

interface StatsViewProps {
  data: TUIData | null;
  height: number;
}

const MONTHS = ["Jan", "Feb", "Mar", "Apr", "May", "Jun", "Jul", "Aug", "Sep", "Oct", "Nov", "Dec"];
const DAYS = ["", "Mon", "", "Wed", "", "Fri", ""];

const COLORS = {
  0: "gray",
  1: "#ffebe6",
  2: "#ffb3a1", 
  3: "#ff7a5c",
  4: "#ff4117",
} as const;

export function StatsView({ data, height: _height }: StatsViewProps) {
  if (!data) return null;

  const formatTokens = (n: number) => {
    if (n >= 1_000_000_000) return `${(n / 1_000_000_000).toFixed(1)}B`;
    if (n >= 1_000_000) return `${(n / 1_000_000).toFixed(1)}M`;
    if (n >= 1_000) return `${(n / 1_000).toFixed(1)}K`;
    return n.toString();
  };

  const grid = buildContributionGrid(data.contributions);

  return (
    <Box flexDirection="column" gap={1}>
      <Box gap={2}>
        <Box 
          backgroundColor="gray" 
          paddingX={1}
        >
          <Text color="white" bold>Models</Text>
        </Box>
        <Text dimColor>(tab to cycle)</Text>
      </Box>

      <Box flexDirection="column">
        <Box gap={1} marginLeft={4}>
          {MONTHS.map((month, i) => (
            <Text key={`month-${i}`} dimColor>{month.padEnd(4)}</Text>
          ))}
        </Box>
        
        {DAYS.map((day, dayIndex) => (
          <Box key={`day-${dayIndex}`}>
            <Text dimColor>{day.padStart(3)} </Text>
            {grid[dayIndex]?.map((cell, weekIndex) => (
              <Text 
                key={`cell-${dayIndex}-${weekIndex}`} 
                color={cell.level === 0 ? "gray" : undefined}
                backgroundColor={cell.level > 0 ? COLORS[cell.level as keyof typeof COLORS] : undefined}
              >
                {cell.level === 0 ? "·" : "█"}
              </Text>
            )) || null}
          </Box>
        ))}
      </Box>

      <Box gap={2} marginTop={1}>
        <Text dimColor>Less</Text>
        <Box gap={0}>
          {[0, 1, 2, 3, 4].map(level => (
            <Text 
              key={level}
              color={level === 0 ? "gray" : undefined}
              backgroundColor={level > 0 ? COLORS[level as keyof typeof COLORS] : undefined}
            >
              {level === 0 ? "·" : "█"}
            </Text>
          ))}
        </Box>
        <Text dimColor>More</Text>
      </Box>

      <Box flexDirection="column" marginTop={1}>
        <Box gap={4}>
          <Box flexDirection="column">
            <Box gap={1}>
              <Text dimColor>Favorite model:</Text>
              <Text color="cyan">{data.stats.favoriteModel}</Text>
            </Box>
            <Box gap={1}>
              <Text dimColor>Sessions:</Text>
              <Text color="cyan">{data.stats.sessions.toLocaleString()}</Text>
            </Box>
            <Box gap={1}>
              <Text dimColor>Current streak:</Text>
              <Text color="cyan">{data.stats.currentStreak} days</Text>
            </Box>
            <Box gap={1}>
              <Text dimColor>Active days:</Text>
              <Text color="cyan">{data.stats.activeDays}/{data.stats.totalDays}</Text>
            </Box>
          </Box>
          
          <Box flexDirection="column">
            <Box gap={1}>
              <Text dimColor>Total tokens:</Text>
              <Text color="cyan">{formatTokens(data.stats.totalTokens)}</Text>
            </Box>
            <Box gap={1}>
              <Text dimColor>Longest session:</Text>
              <Text color="cyan">{data.stats.longestSession}</Text>
            </Box>
            <Box gap={1}>
              <Text dimColor>Longest streak:</Text>
              <Text color="cyan">{data.stats.longestStreak} days</Text>
            </Box>
            <Box gap={1}>
              <Text dimColor>Peak hour:</Text>
              <Text color="cyan">{data.stats.peakHour}</Text>
            </Box>
          </Box>
        </Box>
      </Box>

      <Box marginTop={1}>
        <Text color="yellow" italic>
          Your total spending is ${data.totalCost.toFixed(2)} on AI coding assistants!
        </Text>
      </Box>
    </Box>
  );
}

interface GridCell {
  date: string | null;
  level: number;
}

function buildContributionGrid(contributions: TUIData["contributions"]): GridCell[][] {
  const grid: GridCell[][] = Array.from({ length: 7 }, () => []);
  
  const today = new Date();
  const startDate = new Date(today);
  startDate.setDate(startDate.getDate() - 364);
  
  const contributionMap = new Map(contributions.map(c => [c.date, c.level]));
  
  const currentDate = new Date(startDate);
  while (currentDate <= today) {
    const dateStr = currentDate.toISOString().split("T")[0];
    const dayOfWeek = currentDate.getDay();
    const level = contributionMap.get(dateStr) || 0;
    
    grid[dayOfWeek].push({ date: dateStr, level });
    currentDate.setDate(currentDate.getDate() + 1);
  }
  
  return grid;
}
