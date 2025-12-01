/**
 * Color themes for contribution graph cells only
 * UI colors (backgrounds, cards, text) follow system dark/light mode
 */

/**
 * Graph color palette - only defines contribution intensity colors
 */
export interface GraphColorPalette {
  name: string;
  grade0: string; // Empty/no activity
  grade1: string; // Low activity
  grade2: string; // Medium activity
  grade3: string; // High activity
  grade4: string; // Very high activity
}

export type ColorPaletteName =
  | "green"
  | "halloween"
  | "teal"
  | "blue"
  | "pink"
  | "purple"
  | "orange"
  | "monochrome"
  | "YlGnBu";

/**
 * Color palettes for graph contribution cells
 * These only affect the graph cells, not the UI
 */
export const colorPalettes: Record<ColorPaletteName, GraphColorPalette> = {
  // GitHub Green (default)
  green: {
    name: "Green",
    grade0: "var(--color-graph-empty)",
    grade1: "#9be9a8",
    grade2: "#40c463",
    grade3: "#30a14e",
    grade4: "#216e39",
  },
  // Halloween
  halloween: {
    name: "Halloween",
    grade0: "var(--color-graph-empty)",
    grade1: "#FFEE4A",
    grade2: "#FFC501",
    grade3: "#FE9600",
    grade4: "#03001C",
  },
  // Teal/Aquamarine
  teal: {
    name: "Teal",
    grade0: "var(--color-graph-empty)",
    grade1: "#7ee5e5",
    grade2: "#2dc5c5",
    grade3: "#0d9e9e",
    grade4: "#0e6d6d",
  },
  // Blue
  blue: {
    name: "Blue",
    grade0: "var(--color-graph-empty)",
    grade1: "#79b8ff",
    grade2: "#388bfd",
    grade3: "#1f6feb",
    grade4: "#0d419d",
  },
  // Pink/Magenta
  pink: {
    name: "Pink",
    grade0: "var(--color-graph-empty)",
    grade1: "#f0b5d2",
    grade2: "#d961a0",
    grade3: "#bf4b8a",
    grade4: "#99286e",
  },
  // Purple (Dracula-inspired)
  purple: {
    name: "Purple",
    grade0: "var(--color-graph-empty)",
    grade1: "#cdb4ff",
    grade2: "#a371f7",
    grade3: "#8957e5",
    grade4: "#6e40c9",
  },
  // Orange
  orange: {
    name: "Orange",
    grade0: "var(--color-graph-empty)",
    grade1: "#ffd699",
    grade2: "#ffb347",
    grade3: "#ff8c00",
    grade4: "#cc5500",
  },
  // Monochrome (grayscale)
  monochrome: {
    name: "Monochrome",
    grade0: "var(--color-graph-empty)",
    grade1: "#9e9e9e",
    grade2: "#757575",
    grade3: "#424242",
    grade4: "#212121",
  },
  // YlGnBu (ColorBrewer)
  YlGnBu: {
    name: "YlGnBu",
    grade0: "var(--color-graph-empty)",
    grade1: "#a1dab4",
    grade2: "#41b6c4",
    grade3: "#2c7fb8",
    grade4: "#253494",
  },
};

/**
 * Default color palette
 */
export const DEFAULT_PALETTE: ColorPaletteName = "green";

/**
 * Get all palette names
 */
export const getPaletteNames = (): ColorPaletteName[] =>
  Object.keys(colorPalettes) as ColorPaletteName[];

/**
 * Get palette by name
 */
export const getPalette = (name: ColorPaletteName): GraphColorPalette =>
  colorPalettes[name] || colorPalettes[DEFAULT_PALETTE];

/**
 * Get grade color from palette based on intensity (0-4)
 */
export const getGradeColor = (
  palette: GraphColorPalette,
  intensity: 0 | 1 | 2 | 3 | 4
): string => {
  switch (intensity) {
    case 0:
      return palette.grade0;
    case 1:
      return palette.grade1;
    case 2:
      return palette.grade2;
    case 3:
      return palette.grade3;
    case 4:
      return palette.grade4;
    default:
      return palette.grade0;
  }
};

// Legacy exports for backward compatibility (will be removed)
export type ThemeName = ColorPaletteName;
export type Theme = GraphColorPalette & {
  background: string;
  text: string;
  meta: string;
};
export const DEFAULT_THEME = DEFAULT_PALETTE;
export const getThemeNames = getPaletteNames;
export const getTheme = (name: ThemeName): Theme => {
  const palette = getPalette(name);
  return {
    ...palette,
    background: "var(--color-canvas-default)",
    text: "var(--color-fg-default)",
    meta: "var(--color-fg-muted)",
  };
};
export const themes = Object.fromEntries(
  Object.entries(colorPalettes).map(([key, palette]) => [
    key,
    {
      ...palette,
      background: "var(--color-canvas-default)",
      text: "var(--color-fg-default)",
      meta: "var(--color-fg-muted)",
    },
  ])
) as Record<ThemeName, Theme>;
