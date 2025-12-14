"use client";

import { useState, useEffect, useCallback, useRef } from "react";
import type { ColorPaletteName } from "./themes";
import { DEFAULT_PALETTE } from "./themes";

export type ThemePreference = "light" | "dark" | "system";

export interface Settings {
  theme: ThemePreference;
  paletteName: ColorPaletteName;
}

const DEFAULT_SETTINGS: Settings = {
  theme: "system",
  paletteName: DEFAULT_PALETTE,
};

const STORAGE_KEY = "tokscale-settings";

function getStoredSettings(): Settings {
  if (typeof window === "undefined") return DEFAULT_SETTINGS;

  try {
    const stored = localStorage.getItem(STORAGE_KEY);
    if (stored) {
      const parsed = JSON.parse(stored);
      return {
        theme: parsed.theme || DEFAULT_SETTINGS.theme,
        paletteName: parsed.paletteName || DEFAULT_SETTINGS.paletteName,
      };
    }
  } catch {
    // Invalid JSON or localStorage error
  }

  return DEFAULT_SETTINGS;
}

function saveSettings(settings: Settings): void {
  if (typeof window === "undefined") return;
  try {
    localStorage.setItem(STORAGE_KEY, JSON.stringify(settings));
  } catch {
    // localStorage might be full or disabled
  }
}

function getSystemTheme(): "light" | "dark" {
  if (typeof window === "undefined") return "light";
  return window.matchMedia("(prefers-color-scheme: dark)").matches ? "dark" : "light";
}

function applyThemeToDocument(resolvedTheme: "light" | "dark"): void {
  if (typeof document === "undefined") return;
  const root = document.documentElement;
  root.classList.remove("light", "dark");
  root.classList.add(resolvedTheme);
}

export function useSettings() {
  const [settings, setSettings] = useState<Settings>(() => {
    if (typeof window === "undefined") return DEFAULT_SETTINGS;
    return getStoredSettings();
  });

  const [resolvedTheme, setResolvedTheme] = useState<"light" | "dark">(() => {
    if (typeof window === "undefined") return "light";
    const stored = getStoredSettings();
    return stored.theme === "system" ? getSystemTheme() : stored.theme;
  });

  const mountedRef = useRef(false);
  const [mounted, setMounted] = useState(false);

  useEffect(() => {
    applyThemeToDocument(resolvedTheme);
    mountedRef.current = true;
    // eslint-disable-next-line react-hooks/set-state-in-effect -- Intentional: track mount state for SSR hydration
    setMounted(true);
  }, []); // eslint-disable-line react-hooks/exhaustive-deps

  useEffect(() => {
    if (!mountedRef.current) return;

    const mediaQuery = window.matchMedia("(prefers-color-scheme: dark)");

    const handleChange = (e: MediaQueryListEvent) => {
      if (settings.theme === "system") {
        const newResolved = e.matches ? "dark" : "light";
        setResolvedTheme(() => {
          applyThemeToDocument(newResolved);
          return newResolved;
        });
      }
    };

    mediaQuery.addEventListener("change", handleChange);
    return () => mediaQuery.removeEventListener("change", handleChange);
  }, [settings.theme]);

  useEffect(() => {
    if (!mountedRef.current) return;

    const resolved = settings.theme === "system" ? getSystemTheme() : settings.theme;
    // eslint-disable-next-line react-hooks/set-state-in-effect -- Intentional: sync resolved theme when settings change
    setResolvedTheme((prev) => {
      if (prev !== resolved) {
        applyThemeToDocument(resolved);
        return resolved;
      }
      return prev;
    });
  }, [settings.theme]);

  const setTheme = useCallback((theme: ThemePreference) => {
    setSettings((prev) => {
      const newSettings = { ...prev, theme };
      saveSettings(newSettings);
      return newSettings;
    });
  }, []);

  const setPalette = useCallback((paletteName: ColorPaletteName) => {
    setSettings((prev) => {
      const newSettings = { ...prev, paletteName };
      saveSettings(newSettings);
      return newSettings;
    });
  }, []);

  return {
    theme: settings.theme,
    paletteName: settings.paletteName,
    resolvedTheme,
    setTheme,
    setPalette,
    mounted,
  };
}
