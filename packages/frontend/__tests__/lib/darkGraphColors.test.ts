import { describe, expect, it } from "vitest";
import { DARK_GRAPH_EMPTY, getDarkGraphColor } from "../../src/lib/darkGraphColors";
import { colorPalettes, getDarkGradeColors, getPaletteNames } from "../../src/lib/themes";
import { getGraphCellSize } from "../../src/components/TokenGraph2D";

function luminance(hex: string): number {
  const channels = [1, 3, 5].map((offset) => Number.parseInt(hex.slice(offset, offset + 2), 16) / 255);
  const [r, g, b] = channels.map((value) => (value <= 0.04045 ? value / 12.92 : ((value + 0.055) / 1.055) ** 2.4));
  return 0.2126 * r + 0.7152 * g + 0.0722 * b;
}

function contrast(left: string, right: string): number {
  const [light, dark] = [luminance(left), luminance(right)].sort((a, b) => b - a);
  return (light + 0.05) / (dark + 0.05);
}

const CARD_SURFACE = "#07090d"; // --v2-surface, behind the local viewer graphs

describe("local viewer graph colours on the black canvas", () => {
  it("uses the profile's dark ramp for active days and a neutral step for empty days", () => {
    for (const name of getPaletteNames()) {
      const palette = colorPalettes[name];
      expect(getDarkGraphColor(palette, 0)).toBe(DARK_GRAPH_EMPTY);
      expect([1, 2, 3, 4].map((level) => getDarkGraphColor(palette, level as 1 | 2 | 3 | 4))).toEqual([
        ...getDarkGradeColors(palette),
      ]);
    }
  });

  it("keeps every palette readable on black: brighter with use, and 3:1 against an empty day", () => {
    for (const name of getPaletteNames()) {
      const palette = colorPalettes[name];
      const levels = ([1, 2, 3, 4] as const).map((level) => getDarkGraphColor(palette, level));
      for (const color of levels) {
        expect(contrast(color, DARK_GRAPH_EMPTY), `${name} ${color}`).toBeGreaterThanOrEqual(3);
        expect(contrast(color, CARD_SURFACE), `${name} ${color}`).toBeGreaterThanOrEqual(3);
      }
      for (let index = 1; index < levels.length; index += 1) {
        expect(luminance(levels[index]), name).toBeGreaterThan(luminance(levels[index - 1]));
      }
    }
  });

  it("leaves the shared palettes as they are", () => {
    expect(colorPalettes.halloween.grade4).toBe("#03001C");
    expect(colorPalettes.blue.grade1).toBe("#79b8ff");
  });
});

describe("2D graph cell size", () => {
  it("grows with the card, stops at 24px, and falls back to the 12px pitch on narrow cards", () => {
    expect(getGraphCellSize(0, 53)).toBe(12);
    expect(getGraphCellSize(320, 53)).toBe(12);
    expect(getGraphCellSize(32 + 53 * 16 + 24, 53)).toBe(16);
    expect(getGraphCellSize(1334, 53)).toBe(24);
    expect(getGraphCellSize(4000, 53)).toBe(24);
  });
});
