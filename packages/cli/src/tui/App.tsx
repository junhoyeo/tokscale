import { createSignal, Switch, Match } from "solid-js";
import { useKeyboard, useTerminalDimensions } from "@opentui/solid";
import { Header } from "./components/Header.js";
import { Footer } from "./components/Footer.js";
import { ModelView } from "./components/ModelView.js";
import { DailyView } from "./components/DailyView.js";
import { StatsView } from "./components/StatsView.js";
import { OverviewView } from "./components/OverviewView.js";
import { LoadingSpinner } from "./components/LoadingSpinner.js";
import { useData, type DateFilters } from "./hooks/useData.js";
import type { ColorPaletteName } from "./config/themes.js";
import { DEFAULT_PALETTE, getPaletteNames } from "./config/themes.js";
import { loadSettings, saveSettings } from "./config/settings.js";
import { TABS, ALL_SOURCES, type TUIOptions, type TabType, type SortType, type SourceType } from "./types/index.js";

export type AppProps = TUIOptions;

const PALETTE_NAMES = getPaletteNames();

function cycleTabForward(current: TabType): TabType {
  const idx = TABS.indexOf(current);
  return TABS[(idx + 1) % TABS.length];
}

function cycleTabBackward(current: TabType): TabType {
  const idx = TABS.indexOf(current);
  return TABS[(idx - 1 + TABS.length) % TABS.length];
}

export function App(props: AppProps) {
  const terminalDimensions = useTerminalDimensions();
  const columns = () => terminalDimensions().width;
  const rows = () => terminalDimensions().height;

  const settings = loadSettings();
  const [activeTab, setActiveTab] = createSignal<TabType>(props.initialTab ?? "overview");
  const [enabledSources, setEnabledSources] = createSignal<Set<SourceType>>(
    new Set(props.enabledSources ?? ALL_SOURCES)
  );
  const [sortBy, setSortBy] = createSignal<SortType>(props.sortBy ?? "cost");
  const [sortDesc, setSortDesc] = createSignal(props.sortDesc ?? true);
  const [selectedIndex, setSelectedIndex] = createSignal(0);
  const [scrollOffset, setScrollOffset] = createSignal(0);
  const [colorPalette, setColorPalette] = createSignal<ColorPaletteName>(
    props.colorPalette ?? (settings.colorPalette as ColorPaletteName) ?? DEFAULT_PALETTE
  );

  const dateFilters: DateFilters = {
    since: props.since,
    until: props.until,
    year: props.year,
  };

  const { data, loading, error, refresh } = useData(() => enabledSources(), dateFilters);

  const contentHeight = () => Math.max(rows() - 6, 12);
  const overviewChartHeight = () => Math.max(5, Math.floor(contentHeight() * 0.35));
  const overviewListHeight = () => Math.max(4, contentHeight() - overviewChartHeight() - 4);
  const overviewItemsPerPage = () => Math.max(1, Math.floor(overviewListHeight() / 2));

  const handleSourceToggle = (source: SourceType) => {
    const newSources = new Set(enabledSources());
    if (newSources.has(source)) {
      if (newSources.size > 1) {
        newSources.delete(source);
      }
    } else {
      newSources.add(source);
    }
    setEnabledSources(newSources);
  };

  const handlePaletteChange = () => {
    const currentIdx = PALETTE_NAMES.indexOf(colorPalette());
    const nextIdx = (currentIdx + 1) % PALETTE_NAMES.length;
    const newPalette = PALETTE_NAMES[nextIdx];
    saveSettings({ colorPalette: newPalette });
    setColorPalette(newPalette);
  };

  const handleSortChange = (sort: SortType) => {
    setSortBy(sort);
    setSortDesc(sort !== "name");
  };

  useKeyboard((key) => {
    if (key.name === "q") {
      process.exit(0);
    }

    if (key.name === "r") {
      refresh();
      return;
    }

    if (key.name === "tab" || key.name === "d" || key.name === "right") {
      setActiveTab(cycleTabForward(activeTab()));
      setSelectedIndex(0);
      setScrollOffset(0);
      return;
    }

    if (key.name === "left") {
      setActiveTab(cycleTabBackward(activeTab()));
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
      handlePaletteChange();
      return;
    }

    if (key.name === "1") { handleSourceToggle("opencode"); return; }
    if (key.name === "2") { handleSourceToggle("claude"); return; }
    if (key.name === "3") { handleSourceToggle("codex"); return; }
    if (key.name === "4") { handleSourceToggle("cursor"); return; }
    if (key.name === "5") { handleSourceToggle("gemini"); return; }

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
        const d = data();
        const maxIndex = activeTab() === "model" 
          ? (d?.modelEntries.length ?? 0)
          : (d?.dailyEntries.length ?? 0);
        if (maxIndex > 0) {
          setSelectedIndex(Math.min(selectedIndex() + 1, maxIndex - 1));
        }
      }
      return;
    }

    if (key.name === "e" && data()) {
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
      import("node:fs")
        .then((fs) => {
          fs.writeFileSync(filename, JSON.stringify(exportData, null, 2));
        })
        .catch(() => {});
      return;
    }
  });

  const handleTabClick = (tab: TabType) => {
    setActiveTab(tab);
    setSelectedIndex(0);
    setScrollOffset(0);
  };

  return (
    <box flexDirection="column" width={columns()} height={rows()}>
      <Header activeTab={activeTab()} onTabClick={handleTabClick} />

      <box flexDirection="column" flexGrow={1} paddingX={1}>
        <Switch>
          <Match when={loading()}>
            <LoadingSpinner />
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
        totals={data()?.totals}
        modelCount={data()?.modelCount ?? 0}
        activeTab={activeTab()}
        scrollStart={scrollOffset()}
        scrollEnd={Math.min(scrollOffset() + overviewItemsPerPage(), data()?.topModels.length ?? 0)}
        totalItems={data()?.topModels.length}
        colorPalette={colorPalette()}
        onSourceToggle={handleSourceToggle}
        onSortChange={handleSortChange}
        onPaletteChange={handlePaletteChange}
        onRefresh={refresh}
      />
    </box>
  );
}
