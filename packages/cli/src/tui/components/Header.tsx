import { Box, Text } from "ink";
import type { TabType } from "../App.js";

interface HeaderProps {
  activeTab: TabType;
}

export function Header({ activeTab }: HeaderProps) {
  return (
    <Box paddingX={1} paddingY={0} justifyContent="space-between">
      <Box gap={2}>
        <Tab name="Model" active={activeTab === "model"} />
        <Tab name="Daily" active={activeTab === "daily"} />
        <Tab name="Stats" active={activeTab === "stats"} />
      </Box>
      <Text color="cyan" bold>Token Usage Tracker</Text>
    </Box>
  );
}

function Tab({ name, active }: { name: string; active: boolean }) {
  if (active) {
    return (
      <Box>
        <Text backgroundColor="cyan" color="black" bold> {name} </Text>
      </Box>
    );
  }
  return <Text dimColor>{name}</Text>;
}
