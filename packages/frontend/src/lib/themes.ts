export interface GraphColorPalette {
  name: string;
  grade0: string;
  grade1: string;
  grade2: string;
  grade3: string;
  grade4: string;
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

const GRAPH_EMPTY = "var(--color-graph-empty)";

export const colorPalettes: Record<ColorPaletteName, GraphColorPalette> = {
  green: {
    name: "Green",
    grade0: GRAPH_EMPTY,
    grade1: "#9be9a8",
    grade2: "#40c463",
    grade3: "#30a14e",
    grade4: "#216e39",
  },
  halloween: {
    name: "Halloween",
    grade0: GRAPH_EMPTY,
    grade1: "#FFEE4A",
    grade2: "#FFC501",
    grade3: "#FE9600",
    grade4: "#03001C",
  },
  teal: {
    name: "Teal",
    grade0: GRAPH_EMPTY,
    grade1: "#7ee5e5",
    grade2: "#2dc5c5",
    grade3: "#0d9e9e",
    grade4: "#0e6d6d",
  },
  blue: {
    name: "Blue",
    grade0: GRAPH_EMPTY,
    grade1: "#79b8ff",
    grade2: "#388bfd",
    grade3: "#1f6feb",
    grade4: "#0d419d",
  },
  pink: {
    name: "Pink",
    grade0: GRAPH_EMPTY,
    grade1: "#f0b5d2",
    grade2: "#d961a0",
    grade3: "#bf4b8a",
    grade4: "#99286e",
  },
  purple: {
    name: "Purple",
    grade0: GRAPH_EMPTY,
    grade1: "#cdb4ff",
    grade2: "#a371f7",
    grade3: "#8957e5",
    grade4: "#6e40c9",
  },
  orange: {
    name: "Orange",
    grade0: GRAPH_EMPTY,
    grade1: "#ffd699",
    grade2: "#ffb347",
    grade3: "#ff8c00",
    grade4: "#cc5500",
  },
  monochrome: {
    name: "Monochrome",
    grade0: GRAPH_EMPTY,
    grade1: "#9e9e9e",
    grade2: "#757575",
    grade3: "#424242",
    grade4: "#212121",
  },
  YlGnBu: {
    name: "YlGnBu",
    grade0: GRAPH_EMPTY,
    grade1: "#a1dab4",
    grade2: "#41b6c4",
    grade3: "#2c7fb8",
    grade4: "#253494",
  },
};

export const DEFAULT_PALETTE: ColorPaletteName = "blue";

export const getPaletteNames = (): ColorPaletteName[] =>
  Object.keys(colorPalettes) as ColorPaletteName[];

export const getPalette = (name: ColorPaletteName): GraphColorPalette =>
  colorPalettes[name] || colorPalettes[DEFAULT_PALETTE];

export const getGradeColor = (
  palette: GraphColorPalette,
  intensity: 0 | 1 | 2 | 3 | 4,
): string => {
  const grades = [
    palette.grade0,
    palette.grade1,
    palette.grade2,
    palette.grade3,
    palette.grade4,
  ];
  return grades[intensity] || palette.grade0;
};

const DARK_GRAPH_EMPTY = "#191f2b";
const MINIMUM_ACTIVE_CONTRAST = 3;
const MINIMUM_LUMINANCE_STEP = 0.02;
const darkPaletteCache = new WeakMap<
  GraphColorPalette,
  readonly [string, string, string, string]
>();

function mixHexWithWhite(color: string, colorWeight: number): string {
  const match = /^#([0-9a-f]{6})$/i.exec(color);
  if (!match) return color;

  const channels = [0, 2, 4].map((offset) =>
    Number.parseInt(match[1].slice(offset, offset + 2), 16),
  );
  return `#${channels
    .map((channel) =>
      Math.round(channel * colorWeight + 255 * (1 - colorWeight))
        .toString(16)
        .padStart(2, "0"),
    )
    .join("")}`;
}

function relativeLuminance(color: string): number {
  const match = /^#([0-9a-f]{6})$/i.exec(color);
  if (!match) return 0;

  const channels = [0, 2, 4].map((offset) =>
    Number.parseInt(match[1].slice(offset, offset + 2), 16),
  );
  const linear = channels.map((channel) => {
    const normalized = channel / 255;
    return normalized <= 0.04045
      ? normalized / 12.92
      : ((normalized + 0.055) / 1.055) ** 2.4;
  });
  return linear[0] * 0.2126 + linear[1] * 0.7152 + linear[2] * 0.0722;
}

function contrastRatio(left: string, right: string): number {
  const luminances = [relativeLuminance(left), relativeLuminance(right)].sort(
    (a, b) => b - a,
  );
  return (luminances[0] + 0.05) / (luminances[1] + 0.05);
}

/**
 * Convert a light-canvas graph palette into a monotonic ramp for the compact
 * service surface. Colors stay as close as possible to their source hue while
 * retaining 3:1 contrast and a visible luminance step between levels.
 */
export function getDarkGradeColors(
  palette: GraphColorPalette,
): readonly [string, string, string, string] {
  const cached = darkPaletteCache.get(palette);
  if (cached) return cached;

  const bases = [
    palette.grade4,
    palette.grade3,
    palette.grade2,
    palette.grade1,
  ];
  let previousLuminance = Number.NEGATIVE_INFINITY;
  const colors = bases.map((baseColor, index) => {
    for (let percentage = 100; percentage >= 0; percentage -= 1) {
      const candidate = mixHexWithWhite(baseColor, percentage / 100);
      const luminance = relativeLuminance(candidate);
      const separated =
        index === 0 || luminance >= previousLuminance + MINIMUM_LUMINANCE_STEP;

      if (
        separated &&
        contrastRatio(candidate, DARK_GRAPH_EMPTY) >= MINIMUM_ACTIVE_CONTRAST
      ) {
        previousLuminance = luminance;
        return candidate;
      }
    }

    previousLuminance = 1;
    return "#ffffff";
  }) as [string, string, string, string];

  darkPaletteCache.set(palette, colors);
  return colors;
}
