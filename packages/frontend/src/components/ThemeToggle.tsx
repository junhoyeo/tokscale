"use client";

import { motion } from "framer-motion";
import { Monitor, Moon, Sun } from "lucide-react";
import type { ThemePreference } from "@/lib/useSettings";

const SIZES = {
  buttonSize: 28,
  containerPadding: 3,
  iconSize: 14,
} as const;

const CONTAINER_WIDTH = SIZES.buttonSize * 3 + SIZES.containerPadding * 2;
const CONTAINER_HEIGHT = SIZES.buttonSize + SIZES.containerPadding * 2;

const themes = [
  { value: "light" as const, label: "Light theme", icon: Sun },
  { value: "dark" as const, label: "Dark theme", icon: Moon },
  { value: "system" as const, label: "System theme", icon: Monitor },
] as const;

interface ThemeToggleProps {
  theme: ThemePreference;
  onThemeChange: (theme: ThemePreference) => void;
  mounted: boolean;
}

export function ThemeToggle({ theme, onThemeChange, mounted }: ThemeToggleProps) {
  const activeIndex = themes.findIndex((t) => t.value === theme);
  const indicatorX = SIZES.containerPadding + activeIndex * SIZES.buttonSize;

  if (!mounted) {
    return (
      <div
        className="animate-pulse rounded-full"
        style={{
          width: CONTAINER_WIDTH,
          height: CONTAINER_HEIGHT,
          background: "linear-gradient(to bottom, #212124, #1F1F20)",
        }}
        aria-hidden="true"
      />
    );
  }

  return (
    <div
      role="radiogroup"
      aria-label="Theme selection"
      className="relative inline-flex items-center rounded-full"
      style={{
        width: CONTAINER_WIDTH,
        height: CONTAINER_HEIGHT,
        padding: SIZES.containerPadding,
        background: "linear-gradient(to bottom, #212124, #1F1F20)",
        boxShadow: "inset 0 1px 0 0 rgba(255,255,255,0.05), 0 1px 3px rgba(0,0,0,0.08), 0 1px 2px rgba(0,0,0,0.04)",
        border: "1px solid rgba(255, 255, 255, 0.1)",
      }}
    >
      <motion.div
        className="absolute rounded-full"
        style={{
          top: SIZES.containerPadding,
          bottom: SIZES.containerPadding,
          width: SIZES.buttonSize,
          backgroundColor: "#1F1F20",
          boxShadow: "0 1px 3px rgba(0,0,0,0.1), 0 1px 2px rgba(0,0,0,0.06), 0 0 0 1px rgba(255, 255, 255, 0.1)",
        }}
        initial={false}
        animate={{ x: indicatorX }}
        transition={{ type: "spring", stiffness: 400, damping: 30 }}
      />

      {themes.map(({ value, label, icon: Icon }) => {
        const isActive = theme === value;

        return (
          <button
            key={value}
            type="button"
            role="radio"
            aria-checked={isActive}
            aria-label={label}
            tabIndex={isActive ? 0 : -1}
            onClick={() => onThemeChange(value)}
            onKeyDown={(e) => {
              if (e.key === "ArrowRight" || e.key === "ArrowDown") {
                e.preventDefault();
                onThemeChange(themes[(activeIndex + 1) % themes.length].value);
              } else if (e.key === "ArrowLeft" || e.key === "ArrowUp") {
                e.preventDefault();
                onThemeChange(themes[(activeIndex - 1 + themes.length) % themes.length].value);
              }
            }}
            className="relative z-10 flex items-center justify-center rounded-full transition-colors duration-200 focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-blue-500 focus-visible:ring-offset-2"
            style={{
              width: SIZES.buttonSize,
              height: SIZES.buttonSize,
              color: isActive ? "#FFFFFF" : "#696969",
            }}
          >
            <Icon size={SIZES.iconSize} strokeWidth={isActive ? 2.5 : 2} className="transition-all duration-200" />
          </button>
        );
      })}
    </div>
  );
}
