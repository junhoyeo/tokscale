import { homedir } from "os";
import { join } from "path";
import { readFileSync, writeFileSync, existsSync, mkdirSync } from "fs";
import type { TUIData, DailyModelBreakdown, ChartMode, Resolution } from "../types/index.js";

const CONFIG_DIR = join(homedir(), ".config", "tokscale");
const CACHE_DIR = join(homedir(), ".cache", "tokscale");
const CONFIG_FILE = join(CONFIG_DIR, "tui-settings.json");
const CACHE_FILE = join(CACHE_DIR, "tui-data-cache.json");

const CACHE_STALE_THRESHOLD_MS = 60 * 1000;

export interface TUISettings {
  colorPalette: string;
  chartMode: ChartMode;
  chartResolution: Resolution;
  showRateWhiskers: boolean;
}

const DEFAULT_SETTINGS: TUISettings = {
  colorPalette: 'green',
  chartMode: 'bar',
  chartResolution: '1d',
  showRateWhiskers: false,
};

interface CachedTUIData {
  timestamp: number;
  enabledSources: string[];
  data: Omit<TUIData, 'dailyBreakdowns'> & {
    dailyBreakdowns: Array<[string, DailyModelBreakdown]>;
  };
}

export function loadSettings(): TUISettings {
  try {
    if (existsSync(CONFIG_FILE)) {
      const loaded = JSON.parse(readFileSync(CONFIG_FILE, "utf-8"));
      return { ...DEFAULT_SETTINGS, ...loaded };
    }
  } catch {
  }
  return DEFAULT_SETTINGS;
}

export function saveSettings(settings: TUISettings): void {
  if (!existsSync(CONFIG_DIR)) {
    mkdirSync(CONFIG_DIR, { recursive: true });
  }
  writeFileSync(CONFIG_FILE, JSON.stringify(settings, null, 2));
}

function sourcesMatch(enabledSources: Set<string>, cachedSources: string[]): boolean {
  const cachedSet = new Set(cachedSources);
  if (enabledSources.size !== cachedSet.size) {
    return false;
  }
  for (const source of enabledSources) {
    if (!cachedSet.has(source)) {
      return false;
    }
  }
  return true;
}

export function loadCachedData(enabledSources: Set<string>): TUIData | null {
  try {
    if (!existsSync(CACHE_FILE)) {
      return null;
    }
    
    const cached: CachedTUIData = JSON.parse(readFileSync(CACHE_FILE, "utf-8"));
    
    if (!sourcesMatch(enabledSources, cached.enabledSources)) {
      return null;
    }
    
    if (!cached.data.dailyBreakdowns || !Array.isArray(cached.data.dailyBreakdowns)) {
      return null;
    }
    
    return {
      ...cached.data,
      dailyBreakdowns: new Map(cached.data.dailyBreakdowns),
    };
  } catch {
    return null;
  }
}

export function saveCachedData(data: TUIData, enabledSources: Set<string>): void {
  try {
    if (!existsSync(CACHE_DIR)) {
      mkdirSync(CACHE_DIR, { recursive: true });
    }
    
    const serializableBreakdowns = Array.from(data.dailyBreakdowns.entries());
    const cached: CachedTUIData = {
      timestamp: Date.now(),
      enabledSources: Array.from(enabledSources),
      data: {
        ...data,
        dailyBreakdowns: serializableBreakdowns,
      },
    };
    
    writeFileSync(CACHE_FILE, JSON.stringify(cached));
  } catch {
  }
}

export function isCacheStale(enabledSources: Set<string>): boolean {
  try {
    if (!existsSync(CACHE_FILE)) {
      return true;
    }
    
    const cached: CachedTUIData = JSON.parse(readFileSync(CACHE_FILE, "utf-8"));
    const cacheAge = Date.now() - cached.timestamp;
    
    if (!sourcesMatch(enabledSources, cached.enabledSources)) {
      return true;
    }
    
    return cacheAge > CACHE_STALE_THRESHOLD_MS;
  } catch {
    return true;
  }
}

export function getCacheTimestamp(): number | null {
  try {
    if (!existsSync(CACHE_FILE)) {
      return null;
    }
    const cached: CachedTUIData = JSON.parse(readFileSync(CACHE_FILE, "utf-8"));
    return cached.timestamp;
  } catch {
    return null;
  }
}
