import { createEffect, createSignal, Switch, Match, onCleanup } from "solid-js";
import { useKeyboard, useTerminalDimensions, useRenderer } from "@opentui/solid";
import clipboardy from "clipboardy";
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
import { loadSettings, saveSettings, getCacheTimestamp } from "./config/settings.js";
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
  const renderer = useRenderer();
  const terminalDimensions = useTerminalDimensions();
  const columns = () => terminalDimensions().width;
  const rows = () => terminalDimensions().height;

  const settings = loadSettings();
  const [activeTab, setActiveTab] = createSignal<TabType>(props.initialTab ?? "overview");
  const [enabledSources, setEnabledSources] = createSignal<Set<SourceType>>(
    new Set(props.enabledSources ?? ALL_SOURCES)
  );
  const [sortBy, setSortBy] = createSignal<SortType>(props.sortBy ?? "tokens");
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
    sinceTs: props.sinceTs,
    untilTs: props.untilTs,
  };

  const { data, loading, error, refresh, loadingPhase, isRefreshing } = useData(() => enabledSources(), dateFilters);
  
  const cacheTimestamp = () => !isRefreshing() && !loading() ? getCacheTimestamp() : null;
  
  const [selectedDate, setSelectedDate] = createSignal<string | null>(null);

  const [statusMessage, setStatusMessage] = createSignal<string | null>(null);
  let statusTimeout: ReturnType<typeof setTimeout> | null = null;
  const [autoRefreshEnabled, setAutoRefreshEnabled] = createSignal(settings.autoRefreshEnabled ?? false);
  const [autoRefreshMs, setAutoRefreshMs] = createSignal(settings.autoRefreshMs ?? 60000);

  const showStatus = (msg: string, duration = 2000) => {
    if (statusTimeout) clearTimeout(statusTimeout);
    setStatusMessage(msg);
    statusTimeout = setTimeout(() => setStatusMessage(null), duration);
  };

  onCleanup(() => {
    if (statusTimeout) clearTimeout(statusTimeout);
  });

  const MIN_AUTO_REFRESH_MS = 30000;
  const MAX_AUTO_REFRESH_MS = 3600000;
  const AUTO_REFRESH_STEPS_MS = [
    30000,
    60000,
    120000,
    300000,
    600000,
  ];
  const AUTO_REFRESH_AFTER_MAX_STEP_MS = 600000;

  const clampAutoRefresh = (value: number) =>
    Math.min(MAX_AUTO_REFRESH_MS, Math.max(MIN_AUTO_REFRESH_MS, value));

  const formatIntervalSeconds = (ms: number) => {
    const seconds = Math.round(ms / 1000);
    if (seconds < 60) return `${seconds}s`;
    const minutes = Math.round(seconds / 60);
    return `${minutes}m`;
  };

  const getAutoRefreshIntervalStep = (current: number, direction: "up" | "down") => {
    const value = clampAutoRefresh(current);
    const cappedSteps = AUTO_REFRESH_STEPS_MS;
    const maxStep = cappedSteps[cappedSteps.length - 1];

    if (value > maxStep) {
      const delta = direction === "up" ? AUTO_REFRESH_AFTER_MAX_STEP_MS : -AUTO_REFRESH_AFTER_MAX_STEP_MS;
      return clampAutoRefresh(value + delta);
    }

    if (value === maxStep && direction === "up") {
      return clampAutoRefresh(value + AUTO_REFRESH_AFTER_MAX_STEP_MS);
    }

    if (value === maxStep && direction === "down") {
      return cappedSteps[cappedSteps.length - 2];
    }

    let idx = cappedSteps.findIndex((step) => step >= value);
    if (idx === -1) idx = cappedSteps.length - 1;

    if (direction === "up") {
      const nextIndex = value === cappedSteps[idx] ? idx + 1 : idx;
      return clampAutoRefresh(cappedSteps[Math.min(nextIndex, cappedSteps.length - 1)]);
    }

    const prevIndex = value === cappedSteps[idx] ? idx - 1 : idx - 1;
    return clampAutoRefresh(cappedSteps[Math.max(prevIndex, 0)]);
  };

  createEffect(() => {
    if (!autoRefreshEnabled()) return;
    const ms = autoRefreshMs();
    const interval = setInterval(() => {
      if (!loading() && !isRefreshing()) {
        refresh();
      }
    }, ms);
    onCleanup(() => clearInterval(interval));
  });

  const contentHeight = () => Math.max(rows() - 4, 12);
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
    if (sortBy() === sort) {
      setSortDesc(!sortDesc());
    } else {
      setSortBy(sort);
      setSortDesc(true);
    }
  };

  useKeyboard((key) => {
    if (key.name === "q") {
      renderer.destroy();
      return;
    }

    if (key.name === "r" && key.shift) {
      const next = !autoRefreshEnabled();
      setAutoRefreshEnabled(next);
      saveSettings({ autoRefreshEnabled: next });
      showStatus(`Auto update: ${next ? "ON" : "OFF"} (${formatIntervalSeconds(autoRefreshMs())})`);
      return;
    }

    if ((key.name === "+") || (key.name === "=" && key.shift)) {
      const next = getAutoRefreshIntervalStep(autoRefreshMs(), "up");
      setAutoRefreshMs(next);
      saveSettings({ autoRefreshMs: next });
      showStatus(`Auto update interval: ${formatIntervalSeconds(next)}`);
      return;
    }

    if (key.name === "-" || key.name === "_") {
      const next = getAutoRefreshIntervalStep(autoRefreshMs(), "down");
      setAutoRefreshMs(next);
      saveSettings({ autoRefreshMs: next });
      showStatus(`Auto update interval: ${formatIntervalSeconds(next)}`);
      return;
    }

    if (key.name === "r") {
      refresh();
      return;
    }

    if (key.name === "tab" || key.name === "right") {
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

    if (key.name === "c" && !key.meta && !key.ctrl) {
      handleSortChange("cost");
      return;
    }

    if (key.name === "y") {
      const d = data();
      if (!d) return;
      
      let textToCopy = "";
      const tab = activeTab();
      
      if (tab === "model") {
        const sorted = [...d.modelEntries].sort((a, b) => {
          if (sortBy() === "cost") return sortDesc() ? b.cost - a.cost : a.cost - b.cost;
          if (sortBy() === "tokens") return sortDesc() ? b.total - a.total : a.total - b.total;
          return sortDesc() ? b.model.localeCompare(a.model) : a.model.localeCompare(b.model);
        });
        const entry = sorted[selectedIndex()];
        if (entry) {
          textToCopy = `${entry.source} ${entry.model}: ${entry.total.toLocaleString()} tokens, $${entry.cost.toFixed(2)}`;
        }
      } else if (tab === "daily") {
        const sorted = [...d.dailyEntries].sort((a, b) => {
          if (sortBy() === "cost") return sortDesc() ? b.cost - a.cost : a.cost - b.cost;
          if (sortBy() === "tokens") return sortDesc() ? b.total - a.total : a.total - b.total;
          return sortDesc() ? b.date.localeCompare(a.date) : a.date.localeCompare(b.date);
        });
        const entry = sorted[selectedIndex()];
        if (entry) {
          textToCopy = `${entry.date}: ${entry.total.toLocaleString()} tokens, $${entry.cost.toFixed(2)}`;
        }
      } else if (tab === "overview") {
        const model = d.topModels[scrollOffset() + selectedIndex()];
        if (model) {
          textToCopy = `${model.modelId}: ${model.totalTokens.toLocaleString()} tokens, $${model.cost.toFixed(2)}`;
        }
      }
      
      if (textToCopy) {
        clipboardy.write(textToCopy)
          .then(() => showStatus("Copied to clipboard"))
          .catch(() => showStatus("Failed to copy"));
      }
      return;
    }
    if (key.name === "t") {
      handleSortChange("tokens");
      return;
    }

    if (key.name === "d") {
      handleSortChange("date");
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
    if (key.name === "6") { handleSourceToggle("amp"); return; }
    if (key.name === "7") { handleSourceToggle("droid"); return; }
    if (key.name === "8") { handleSourceToggle("openclaw"); return; }
    if (key.name === "9") { handleSourceToggle("pi"); return; }

    if (key.name === "up") {
      if (activeTab() === "overview") {
        if (selectedIndex() > 0) {
          setSelectedIndex(selectedIndex() - 1);
        } else if (scrollOffset() > 0) {
          setScrollOffset(scrollOffset() - 1);
        }
      } else {
        setSelectedIndex(Math.max(0, selectedIndex() - 1));
      }
      return;
    }

    if (key.name === "down") {
      if (activeTab() === "overview") {
        const maxVisible = Math.min(overviewItemsPerPage(), (data()?.topModels.length ?? 0) - scrollOffset());
        const maxOffset = Math.max(0, (data()?.topModels.length ?? 0) - overviewItemsPerPage());
        if (selectedIndex() < maxVisible - 1) {
          setSelectedIndex(selectedIndex() + 1);
        } else if (scrollOffset() < maxOffset) {
          setScrollOffset(scrollOffset() + 1);
        }
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
      const filename = `tokscale-export-${new Date().toISOString().split("T")[0]}.json`;
      import("node:fs")
        .then((fs) => {
          fs.writeFileSync(filename, JSON.stringify(exportData, null, 2));
          showStatus(`Exported to ${filename}`);
        })
        .catch(() => showStatus("Export failed"));
      return;
    }
  });

  const handleTabClick = (tab: TabType) => {
    setActiveTab(tab);
    setSelectedIndex(0);
    setScrollOffset(0);
    setSelectedDate(null);
  };

  return (
    <box flexDirection="column" width={columns()} height={rows()} backgroundColor="black">
      <Header activeTab={activeTab()} onTabClick={handleTabClick} width={columns()} />

      <box flexDirection="column" flexGrow={1} paddingX={1}>
        <Switch>
          <Match when={loading()}>
            <LoadingSpinner phase={loadingPhase()} />
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
                  sortBy={sortBy()}
                  sortDesc={sortDesc()}
                  selectedIndex={selectedIndex}
                  scrollOffset={scrollOffset}
                  height={contentHeight()}
                  width={columns()}
                />
              </Match>
              <Match when={activeTab() === "model"}>
                <ModelView
                  data={data()!}
                  sortBy={sortBy()}
                  sortDesc={sortDesc()}
                  selectedIndex={selectedIndex}
                  height={contentHeight()}
                  width={columns()}
                />
              </Match>
              <Match when={activeTab() === "daily"}>
                <DailyView
                  data={data()!}
                  sortBy={sortBy()}
                  sortDesc={sortDesc()}
                  selectedIndex={selectedIndex}
                  height={contentHeight()}
                  width={columns()}
                />
              </Match>
              <Match when={activeTab() === "stats"}>
                <StatsView
                  data={data()!}
                  height={contentHeight()}
                  colorPalette={colorPalette()}
                  width={columns()}
                  selectedDate={selectedDate()}
                  sortBy={sortBy()}
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
        statusMessage={statusMessage()}
        isRefreshing={isRefreshing()}
        loadingPhase={loadingPhase()}
        cacheTimestamp={cacheTimestamp()}
        autoRefreshEnabled={autoRefreshEnabled()}
        autoRefreshMs={autoRefreshMs()}
        width={columns()}
        onSourceToggle={handleSourceToggle}
        onSortChange={handleSortChange}
        onPaletteChange={handlePaletteChange}
        onRefresh={refresh}
      />
    </box>
  );
}
