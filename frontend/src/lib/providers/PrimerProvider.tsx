"use client";

import React from "react";
import { ThemeProvider, BaseStyles } from "@primer/react";
import { useSettings } from "@/lib/useSettings";

// Import Primer CSS primitives for theming
import "@primer/primitives/dist/css/functional/themes/light.css";
import "@primer/primitives/dist/css/functional/themes/dark.css";

/**
 * Primer Theme Provider wrapper
 * 
 * Integrates Primer's theming system with our existing theme management.
 * - Uses our useSettings hook to determine current theme
 * - Maps our theme values to Primer's colorMode
 * - Uses preventSSRMismatch for proper SSR/hydration
 */
export function PrimerProvider({ children }: { children: React.ReactNode }) {
  const { theme } = useSettings();

  // Map our theme preference to Primer's colorMode
  // Primer uses 'day' | 'night' | 'auto'
  const colorMode = theme === "system" ? "auto" : theme === "dark" ? "night" : "day";

  // Force the resolved scheme based on our existing theme system
  // This ensures consistency with our existing dark/light mode implementation
  const dayScheme = "light";
  const nightScheme = "dark";

  return (
    <ThemeProvider
      colorMode={colorMode}
      dayScheme={dayScheme}
      nightScheme={nightScheme}
      preventSSRMismatch
    >
      <BaseStyles>
        {children}
      </BaseStyles>
    </ThemeProvider>
  );
}

/**
 * Wrapper component to use Primer components outside of the theme context
 * Useful for components that need to render before the theme is loaded
 */
export function PrimerWrapper({ children }: { children: React.ReactNode }) {
  return (
    <ThemeProvider colorMode="auto" preventSSRMismatch>
      <BaseStyles>{children}</BaseStyles>
    </ThemeProvider>
  );
}
