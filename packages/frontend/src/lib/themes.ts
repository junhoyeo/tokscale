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

export const getGradeColor = (palette: GraphColorPalette, intensity: 0 | 1 | 2 | 3 | 4): string => {
  const grades = [palette.grade0, palette.grade1, palette.grade2, palette.grade3, palette.grade4];
  return grades[intensity] || palette.grade0;
};
