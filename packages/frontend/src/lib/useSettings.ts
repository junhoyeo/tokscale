"use client";

import { useCallback, useEffect, useSyncExternalStore } from "react";
import type { ColorPaletteName } from "./themes";
import { DEFAULT_PALETTE } from "./themes";
import {
  type LeaderboardSortBy,
  SORT_BY_COOKIE_NAME,
  isValidSortBy,
} from "./leaderboard/constants";

export type { LeaderboardSortBy };

export interface Settings {
  paletteName: ColorPaletteName;
  leaderboardSortBy: LeaderboardSortBy;
}

const DEFAULT_SETTINGS: Settings = {
  paletteName: DEFAULT_PALETTE,
  leaderboardSortBy: "tokens",
};

const STORAGE_KEY = "tokscale-settings";
const SETTINGS_EVENT = "tokscale-settings-changed";

let cachedRawSettings: string | null = null;
let cachedSettings: Settings = DEFAULT_SETTINGS;

function setSortByCookie(sortBy: LeaderboardSortBy): void {
  if (typeof document === "undefined") return;
  document.cookie = `${SORT_BY_COOKIE_NAME}=${sortBy}; path=/; max-age=${60 * 60 * 24 * 365}; SameSite=Lax`;
}

function getStoredSettings(): Settings {
  if (typeof window === "undefined") return DEFAULT_SETTINGS;

  try {
    const stored = localStorage.getItem(STORAGE_KEY);
    if (!stored) {
      cachedRawSettings = null;
      cachedSettings = DEFAULT_SETTINGS;
      return DEFAULT_SETTINGS;
    }

    if (stored === cachedRawSettings) {
      return cachedSettings;
    }

    const parsed = JSON.parse(stored);
    cachedRawSettings = stored;
    cachedSettings = {
      paletteName: parsed.paletteName || DEFAULT_SETTINGS.paletteName,
      leaderboardSortBy: isValidSortBy(parsed.leaderboardSortBy)
        ? parsed.leaderboardSortBy
        : DEFAULT_SETTINGS.leaderboardSortBy,
    };
    return cachedSettings;
  } catch {
    // Invalid JSON or localStorage error
    cachedRawSettings = null;
    cachedSettings = DEFAULT_SETTINGS;
  }

  return DEFAULT_SETTINGS;
}

function saveSettings(settings: Settings): void {
  if (typeof window === "undefined") return;
  try {
    const serialized = JSON.stringify(settings);
    cachedRawSettings = serialized;
    cachedSettings = settings;
    localStorage.setItem(STORAGE_KEY, serialized);
    window.dispatchEvent(new Event(SETTINGS_EVENT));
  } catch {
    // localStorage might be full or disabled
  }
}

function subscribeToSettings(onStoreChange: () => void): () => void {
  if (typeof window === "undefined") {
    return () => {};
  }

  const handleStorage = (event: StorageEvent) => {
    if (!event.key || event.key === STORAGE_KEY) {
      onStoreChange();
    }
  };

  window.addEventListener("storage", handleStorage);
  window.addEventListener(SETTINGS_EVENT, onStoreChange);
  return () => {
    window.removeEventListener("storage", handleStorage);
    window.removeEventListener(SETTINGS_EVENT, onStoreChange);
  };
}

function subscribeToMounted(): () => void {
  return () => {};
}

function applyDarkModeToDocument(): void {
  if (typeof document === "undefined") return;
  const root = document.documentElement;
  root.classList.remove("light");
  root.classList.add("dark");
}

export function useSettings() {
  const settings = useSyncExternalStore(
    subscribeToSettings,
    getStoredSettings,
    () => DEFAULT_SETTINGS,
  );
  const mounted = useSyncExternalStore(
    subscribeToMounted,
    () => true,
    () => false,
  );

  useEffect(() => {
    applyDarkModeToDocument();
    setSortByCookie(settings.leaderboardSortBy);
  }, [settings.leaderboardSortBy]);

  const setPalette = useCallback((paletteName: ColorPaletteName) => {
    saveSettings({ ...getStoredSettings(), paletteName });
  }, []);

  const setLeaderboardSort = useCallback((sortBy: LeaderboardSortBy) => {
    setSortByCookie(sortBy);
    saveSettings({ ...getStoredSettings(), leaderboardSortBy: sortBy });
  }, []);

  return {
    paletteName: settings.paletteName,
    setPalette,
    leaderboardSortBy: settings.leaderboardSortBy,
    setLeaderboardSort,
    mounted,
  };
}
