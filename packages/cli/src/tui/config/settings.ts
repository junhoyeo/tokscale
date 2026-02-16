import { homedir } from "os";
import { join } from "path";
import { readFileSync, writeFileSync, existsSync, mkdirSync } from "fs";
import type { TUIData, DailyModelBreakdown } from "../types/index.js";

const CONFIG_DIR = join(homedir(), ".config", "tokscale");
const CACHE_DIR = join(homedir(), ".cache", "tokscale");
const CONFIG_FILE = join(CONFIG_DIR, "settings.json");
const LEGACY_CONFIG_FILE = join(CONFIG_DIR, "tui-settings.json");
const CACHE_FILE = join(CACHE_DIR, "tui-data-cache.json");

const CACHE_STALE_THRESHOLD_MS = 60 * 1000;
const MIN_AUTO_REFRESH_MS = 30000;
const MAX_AUTO_REFRESH_MS = 3600000;
const DEFAULT_AUTO_REFRESH_MS = 60000;

const DEFAULT_NATIVE_TIMEOUT_MS = 300_000; // 5 minutes
const MIN_NATIVE_TIMEOUT_MS = 5_000; // 5 seconds
const MAX_NATIVE_TIMEOUT_MS = 3_600_000; // 1 hour

export interface TokscaleSettings {
  colorPalette: string;
  autoRefreshEnabled?: boolean;
  autoRefreshMs?: number;
  includeUnusedModels?: boolean;
  nativeTimeoutMs?: number;
}

function validateSettings(raw: unknown): TokscaleSettings {
  const defaults: TokscaleSettings = { 
    colorPalette: "blue", 
    autoRefreshEnabled: false, 
    autoRefreshMs: DEFAULT_AUTO_REFRESH_MS,
    includeUnusedModels: false,
    nativeTimeoutMs: DEFAULT_NATIVE_TIMEOUT_MS,
  };
  
  if (!raw || typeof raw !== "object") return defaults;
  
  const obj = raw as Record<string, unknown>;
  
  const colorPalette = typeof obj.colorPalette === "string" ? obj.colorPalette : defaults.colorPalette;
  const autoRefreshEnabled = typeof obj.autoRefreshEnabled === "boolean" ? obj.autoRefreshEnabled : defaults.autoRefreshEnabled;
  
  let autoRefreshMs = defaults.autoRefreshMs;
  if (typeof obj.autoRefreshMs === "number" && Number.isFinite(obj.autoRefreshMs)) {
    autoRefreshMs = Math.min(MAX_AUTO_REFRESH_MS, Math.max(MIN_AUTO_REFRESH_MS, obj.autoRefreshMs));
  }
  
  const includeUnusedModels = typeof obj.includeUnusedModels === "boolean" ? obj.includeUnusedModels : defaults.includeUnusedModels;
  
  let nativeTimeoutMs = defaults.nativeTimeoutMs;
  if (typeof obj.nativeTimeoutMs === "number" && Number.isFinite(obj.nativeTimeoutMs)) {
    nativeTimeoutMs = Math.min(MAX_NATIVE_TIMEOUT_MS, Math.max(MIN_NATIVE_TIMEOUT_MS, obj.nativeTimeoutMs));
  }
  
  return { colorPalette, autoRefreshEnabled, autoRefreshMs, includeUnusedModels, nativeTimeoutMs };
}

interface CachedTUIData {
  timestamp: number;
  enabledSources: string[];
  data: Omit<TUIData, 'dailyBreakdowns'> & {
    dailyBreakdowns: Array<[string, DailyModelBreakdown]>;
  };
}

export function loadSettings(): TokscaleSettings {
  try {
    if (existsSync(CONFIG_FILE)) {
      const raw = JSON.parse(readFileSync(CONFIG_FILE, "utf-8"));
      return validateSettings(raw);
    }
    if (existsSync(LEGACY_CONFIG_FILE)) {
      const raw = JSON.parse(readFileSync(LEGACY_CONFIG_FILE, "utf-8"));
      return validateSettings(raw);
    }
  } catch {
  }
  return { colorPalette: "blue", autoRefreshEnabled: false, autoRefreshMs: DEFAULT_AUTO_REFRESH_MS, includeUnusedModels: false, nativeTimeoutMs: DEFAULT_NATIVE_TIMEOUT_MS };
}

export function saveSettings(updates: Partial<TokscaleSettings>): void {
  try {
    if (!existsSync(CONFIG_DIR)) {
      mkdirSync(CONFIG_DIR, { recursive: true });
    }
    const current = loadSettings();
    const merged = { ...current, ...updates };
    writeFileSync(CONFIG_FILE, JSON.stringify(merged, null, 2));
  } catch {
  }
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
