import { createSignal, Switch, Match } from "solid-js";
import { useKeyboard, useTerminalDimensions } from "@opentui/solid";
import { Header } from "./components/Header.js";
import { Footer } from "./components/Footer.js";
import { ModelView } from "./components/ModelView.js";
import { DailyView } from "./components/DailyView.js";
import { StatsView } from "./components/StatsView.js";
import { OverviewView } from "./components/OverviewView.js";
import { useData } from "./hooks/useData.js";
import type { ColorPaletteName } from "./config/themes.js";
import { DEFAULT_PALETTE, getPaletteNames } from "./config/themes.js";
import { loadSettings, saveSettings } from "./config/settings.js";

export type TabType = "overview" | "model" | "daily" | "stats";
export type SortType = "cost" | "name" | "tokens";
export type SourceType = "opencode" | "claude" | "codex" | "cursor" | "gemini";

export function App() {
  const terminalDimensions = useTerminalDimensions();
  const columns = () => terminalDimensions().width;
  const rows = () => terminalDimensions().height;

  const settings = loadSettings();
  const [activeTab, setActiveTab] = createSignal<TabType>("overview");
  const [enabledSources, setEnabledSources] = createSignal<Set<SourceType>>(
    new Set(["opencode", "claude", "codex", "cursor", "gemini"])
  );
  const [sortBy, setSortBy] = createSignal<SortType>("cost");
  const [sortDesc, setSortDesc] = createSignal(true);
  const [selectedIndex, setSelectedIndex] = createSignal(0);
  const [scrollOffset, setScrollOffset] = createSignal(0);
  const [colorPalette, setColorPalette] = createSignal<ColorPaletteName>(
    (settings.colorPalette as ColorPaletteName) || DEFAULT_PALETTE
  );

  const { data, loading, error, refresh } = useData(() => enabledSources());

  const contentHeight = () => Math.max(rows() - 6, 12);
  const overviewChartHeight = () => Math.max(5, Math.floor(contentHeight() * 0.35));
  const overviewListHeight = () => Math.max(4, contentHeight() - overviewChartHeight() - 4);
  const overviewItemsPerPage = () => Math.max(1, Math.floor(overviewListHeight() / 2));

  useKeyboard((key) => {
    if (key.name === "q") {
      process.exit(0);
    }

    if (key.name === "r") {
      refresh();
      return;
    }

    const cycleTab = (current: TabType): TabType => {
      const tabs: TabType[] = ["overview", "model", "daily", "stats"];
      const idx = tabs.indexOf(current);
      return tabs[(idx + 1) % tabs.length];
    };

    if (key.name === "tab" || key.name === "d") {
      setActiveTab(cycleTab(activeTab()));
      setSelectedIndex(0);
      setScrollOffset(0);
      return;
    }

    if (key.name === "c") {
      setSortBy("cost");
      setSortDesc(true);
      return;
    }
    if (key.name === "n") {
      setSortBy("name");
      setSortDesc(false);
      return;
    }
    if (key.name === "t") {
      setSortBy("tokens");
      setSortDesc(true);
      return;
    }

    if (key.name === "p") {
      const palettes = getPaletteNames();
      const currentIdx = palettes.indexOf(colorPalette());
      const nextIdx = (currentIdx + 1) % palettes.length;
      const newPalette = palettes[nextIdx];
      saveSettings({ colorPalette: newPalette });
      setColorPalette(newPalette);
      return;
    }

    const toggleSource = (source: SourceType) => {
      const newSources = new Set(enabledSources());
      if (newSources.has(source)) newSources.delete(source);
      else newSources.add(source);
      setEnabledSources(newSources);
    };

    if (key.name === "1") { toggleSource("opencode"); return; }
    if (key.name === "2") { toggleSource("claude"); return; }
    if (key.name === "3") { toggleSource("codex"); return; }
    if (key.name === "4") { toggleSource("cursor"); return; }
    if (key.name === "5") { toggleSource("gemini"); return; }

    if (key.name === "up") {
      if (activeTab() === "overview" && scrollOffset() > 0) {
        setScrollOffset(scrollOffset() - 1);
      } else {
        setSelectedIndex(Math.max(0, selectedIndex() - 1));
      }
      return;
    }

    if (key.name === "down") {
      if (activeTab() === "overview") {
        const maxOffset = Math.max(0, (data()?.topModels.length ?? 0) - overviewItemsPerPage());
        setScrollOffset(Math.min(maxOffset, scrollOffset() + 1));
      } else {
        setSelectedIndex(selectedIndex() + 1);
      }
      return;
    }

    if (key.name === "e" && data()) {
      import("node:fs").then((fs) => {
        const d = data()!;
        const exportData = {
          exportedAt: new Date().toISOString(),
          totalCost: d.totalCost,
          modelCount: d.modelCount,
          models: d.modelEntries,
          daily: d.dailyEntries,
          stats: d.stats,
        };
        const filename = `token-usage-export-${new Date().toISOString().split("T")[0]}.json`;
        fs.writeFileSync(filename, JSON.stringify(exportData, null, 2));
      });
      return;
    }
  });

  return (
    <box flexDirection="column" width={columns()} height={rows()}>
      <Header activeTab={activeTab()} />

      <box flexDirection="column" flexGrow={1} paddingX={1}>
        <Switch>
          <Match when={loading()}>
            <box justifyContent="center" alignItems="center" flexGrow={1}>
              <text fg="cyan">Loading data...</text>
            </box>
          </Match>
          <Match when={error()}>
            <box justifyContent="center" alignItems="center" flexGrow={1}>
              <text fg="red">{`Error: ${error()}`}</text>
            </box>
          </Match>
          <Match when={data()}>
            <Switch>
              <Match when={activeTab() === "overview"}>
                <OverviewView
                  data={data()!}
                  selectedIndex={selectedIndex()}
                  scrollOffset={scrollOffset()}
                  height={contentHeight()}
                  width={columns()}
                />
              </Match>
              <Match when={activeTab() === "model"}>
                <ModelView
                  data={data()!}
                  sortBy={sortBy()}
                  sortDesc={sortDesc()}
                  selectedIndex={selectedIndex()}
                  height={contentHeight()}
                />
              </Match>
              <Match when={activeTab() === "daily"}>
                <DailyView
                  data={data()!}
                  sortBy={sortBy()}
                  sortDesc={sortDesc()}
                  selectedIndex={selectedIndex()}
                  height={contentHeight()}
                />
              </Match>
              <Match when={activeTab() === "stats"}>
                <StatsView
                  data={data()!}
                  height={contentHeight()}
                  colorPalette={colorPalette()}
                />
              </Match>
            </Switch>
          </Match>
        </Switch>
      </box>

      <Footer
        enabledSources={enabledSources()}
        sortBy={sortBy()}
        totalCost={data()?.totalCost ?? 0}
        modelCount={data()?.modelCount ?? 0}
        activeTab={activeTab()}
        scrollStart={scrollOffset()}
        scrollEnd={Math.min(scrollOffset() + overviewItemsPerPage(), data()?.topModels.length ?? 0)}
        totalItems={data()?.topModels.length}
        colorPalette={colorPalette()}
      />
    </box>
  );
}
