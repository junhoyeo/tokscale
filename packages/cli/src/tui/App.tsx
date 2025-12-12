import { useState, useEffect } from "react";
import { Box, Text, useInput, useApp, useStdout } from "ink";
import { Header } from "./components/Header.js";
import { Footer } from "./components/Footer.js";
import { ModelView } from "./components/ModelView.js";
import { DailyView } from "./components/DailyView.js";
import { StatsView } from "./components/StatsView.js";
import { useData } from "./hooks/useData.js";

export type TabType = "model" | "daily" | "stats";
export type SortType = "cost" | "name" | "tokens";
export type SourceType = "opencode" | "claude" | "codex" | "cursor" | "gemini";

export interface AppState {
  activeTab: TabType;
  enabledSources: Set<SourceType>;
  sortBy: SortType;
  sortDesc: boolean;
  selectedIndex: number;
}

function useStdoutDimensions(): [number, number] {
  const { stdout } = useStdout();
  const [dimensions, setDimensions] = useState<[number, number]>([stdout.columns || 80, stdout.rows || 24]);
  
  useEffect(() => {
    const handler = () => setDimensions([stdout.columns || 80, stdout.rows || 24]);
    stdout.on("resize", handler);
    return () => { stdout.off("resize", handler); };
  }, [stdout]);
  
  return dimensions;
}

export function App() {
  const { exit } = useApp();
  const [columns, rows] = useStdoutDimensions();
  
  const [state, setState] = useState<AppState>({
    activeTab: "model",
    enabledSources: new Set(["opencode", "claude", "codex", "cursor", "gemini"]),
    sortBy: "cost",
    sortDesc: true,
    selectedIndex: 0,
  });

  const { data, loading, error, refresh } = useData(state.enabledSources);

  useInput((input, key) => {
    if (input === "q") {
      exit();
      return;
    }

    if (input === "r") {
      refresh();
      return;
    }

    if (key.tab) {
      setState((s) => ({
        ...s,
        activeTab: s.activeTab === "model" ? "daily" : s.activeTab === "daily" ? "stats" : "model",
        selectedIndex: 0,
      }));
      return;
    }

    if (input === "d") {
      setState((s) => ({
        ...s,
        activeTab: s.activeTab === "model" ? "daily" : s.activeTab === "daily" ? "stats" : "model",
        selectedIndex: 0,
      }));
      return;
    }

    if (input === "c") {
      setState((s) => ({ ...s, sortBy: "cost", sortDesc: true }));
      return;
    }
    if (input === "n") {
      setState((s) => ({ ...s, sortBy: "name", sortDesc: false }));
      return;
    }
    if (input === "t") {
      setState((s) => ({ ...s, sortBy: "tokens", sortDesc: true }));
      return;
    }

    if (input === "1") {
      setState((s) => {
        const newSources = new Set(s.enabledSources);
        if (newSources.has("opencode")) newSources.delete("opencode");
        else newSources.add("opencode");
        return { ...s, enabledSources: newSources };
      });
      return;
    }
    if (input === "2") {
      setState((s) => {
        const newSources = new Set(s.enabledSources);
        if (newSources.has("claude")) newSources.delete("claude");
        else newSources.add("claude");
        return { ...s, enabledSources: newSources };
      });
      return;
    }
    if (input === "3") {
      setState((s) => {
        const newSources = new Set(s.enabledSources);
        if (newSources.has("codex")) newSources.delete("codex");
        else newSources.add("codex");
        return { ...s, enabledSources: newSources };
      });
      return;
    }
    if (input === "4") {
      setState((s) => {
        const newSources = new Set(s.enabledSources);
        if (newSources.has("cursor")) newSources.delete("cursor");
        else newSources.add("cursor");
        return { ...s, enabledSources: newSources };
      });
      return;
    }
    if (input === "5") {
      setState((s) => {
        const newSources = new Set(s.enabledSources);
        if (newSources.has("gemini")) newSources.delete("gemini");
        else newSources.add("gemini");
        return { ...s, enabledSources: newSources };
      });
      return;
    }

    if (key.upArrow) {
      setState((s) => ({ ...s, selectedIndex: Math.max(0, s.selectedIndex - 1) }));
      return;
    }
    if (key.downArrow) {
      setState((s) => ({ ...s, selectedIndex: s.selectedIndex + 1 }));
      return;
    }

    if (input === "e" && data) {
      import("node:fs").then((fs) => {
        const exportData = {
          exportedAt: new Date().toISOString(),
          totalCost: data.totalCost,
          modelCount: data.modelCount,
          models: data.modelEntries,
          daily: data.dailyEntries,
          stats: data.stats,
        };
        const filename = `token-usage-export-${new Date().toISOString().split("T")[0]}.json`;
        fs.writeFileSync(filename, JSON.stringify(exportData, null, 2));
      });
      return;
    }
  });

  const contentHeight = rows - 6;

  return (
    <Box flexDirection="column" width={columns} height={rows}>
      <Header activeTab={state.activeTab} />
      
      <Box flexDirection="column" flexGrow={1} paddingX={1}>
        {loading ? (
          <Box justifyContent="center" alignItems="center" flexGrow={1}>
            <Text color="cyan">Loading data...</Text>
          </Box>
        ) : error ? (
          <Box justifyContent="center" alignItems="center" flexGrow={1}>
            <Text color="red">Error: {error}</Text>
          </Box>
        ) : (
          <>
            {state.activeTab === "model" && (
              <ModelView 
                data={data} 
                sortBy={state.sortBy} 
                sortDesc={state.sortDesc}
                selectedIndex={state.selectedIndex}
                height={contentHeight}
              />
            )}
            {state.activeTab === "daily" && (
              <DailyView 
                data={data} 
                sortBy={state.sortBy}
                sortDesc={state.sortDesc}
                selectedIndex={state.selectedIndex}
                height={contentHeight}
              />
            )}
            {state.activeTab === "stats" && (
              <StatsView data={data} height={contentHeight} />
            )}
          </>
        )}
      </Box>

      <Footer 
        enabledSources={state.enabledSources}
        sortBy={state.sortBy}
        totalCost={data?.totalCost ?? 0}
        modelCount={data?.modelCount ?? 0}
      />
    </Box>
  );
}
