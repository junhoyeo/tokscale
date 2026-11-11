import { getDarkGradeColors, type GraphColorPalette } from "./themes";

/** Empty days on the local viewer's black canvas: one neutral step above --v2-surface. */
export const DARK_GRAPH_EMPTY = "#13171d";

/**
 * Empty cubes in the 3D view. The cube shader darkens the side faces, so the
 * top face starts a little lighter than a flat empty cell.
 */
export const DARK_GRAPH_EMPTY_CUBE = "#1d222b";

/**
 * Colour for one contribution level on the always-dark v2 surfaces.
 *
 * The shared palettes are ramps for a light canvas: their darkest grade marks
 * the busiest day, which disappears on black (Halloween's top grade is
 * #03001C). Use the same reversed, contrast-lifted ramp as the profile graph
 * and the embeds instead, so busier days are brighter and every active level
 * keeps 3:1 contrast with an empty day. The palettes themselves are unchanged.
 */
export function getDarkGraphColor(
  palette: GraphColorPalette,
  intensity: 0 | 1 | 2 | 3 | 4,
): string {
  if (intensity <= 0) return DARK_GRAPH_EMPTY;
  return getDarkGradeColors(palette)[intensity - 1] ?? palette.grade1;
}
