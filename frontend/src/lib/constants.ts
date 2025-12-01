/**
 * Rendering constants for contribution graph
 * Used by both 2D and 3D views
 */

// =====================================
// 2D Canvas Constants
// =====================================

/** Width of each day box in pixels */
export const BOX_WIDTH = 10;

/** Margin between day boxes in pixels */
export const BOX_MARGIN = 2;

/** Height reserved for day labels (Sun-Sat) */
export const TEXT_HEIGHT = 15;

/** Canvas margin from edges */
export const CANVAS_MARGIN = 20;

/** Height reserved for header (month labels, title) */
export const HEADER_HEIGHT = 60;

/** Border radius for day boxes */
export const BOX_BORDER_RADIUS = 2;

/** Weeks in a year (53 to account for partial weeks) */
export const WEEKS_IN_YEAR = 53;

/** Days in a week */
export const DAYS_IN_WEEK = 7;

/** Font size for labels */
export const FONT_SIZE = 10;

/** Font family for canvas text */
export const FONT_FAMILY = "'SF Mono', ui-monospace, Menlo, Monaco, 'Cascadia Mono', 'Segoe UI Mono', monospace";

// =====================================
// 3D Isometric Constants (obelisk.js)
// =====================================

/** Base cube size in pixels (width/depth) */
export const CUBE_SIZE = 16;

/** Maximum cube height for highest intensity */
export const MAX_CUBE_HEIGHT = 100;

/** Minimum cube height for lowest intensity (>0) */
export const MIN_CUBE_HEIGHT = 3;

/** Origin point for isometric view */
export const ISO_ORIGIN = { x: 130, y: 90 };

/** Gap between cubes */
export const CUBE_GAP = 2;

/** 3D canvas width */
export const ISO_CANVAS_WIDTH = 1000;

/** 3D canvas height */
export const ISO_CANVAS_HEIGHT = 600;

// =====================================
// Day Labels
// =====================================

/** Short day names (Sunday start) */
export const DAY_LABELS_SHORT = ["Sun", "Mon", "Tue", "Wed", "Thu", "Fri", "Sat"];

/** Month names (short) */
export const MONTH_LABELS_SHORT = [
  "Jan",
  "Feb",
  "Mar",
  "Apr",
  "May",
  "Jun",
  "Jul",
  "Aug",
  "Sep",
  "Oct",
  "Nov",
  "Dec",
];

// =====================================
// Source Display Names and Colors
// =====================================

/** Display names for sources */
export const SOURCE_DISPLAY_NAMES: Record<string, string> = {
  opencode: "OpenCode",
  claude: "Claude Code",
  codex: "Codex CLI",
  gemini: "Gemini CLI",
};

/** Colors for source badges */
export const SOURCE_COLORS: Record<string, string> = {
  opencode: "#22c55e",  // green-500
  claude: "#f97316",    // orange-500
  codex: "#3b82f6",     // blue-500
  gemini: "#8b5cf6",    // violet-500
};

// =====================================
// Calculated Values
// =====================================

/** Total width of a day cell (box + margin) */
export const CELL_SIZE = BOX_WIDTH + BOX_MARGIN;

/** Calculate 2D canvas width based on weeks */
export const calculateCanvasWidth = (weeks: number = WEEKS_IN_YEAR): number =>
  CANVAS_MARGIN * 2 + TEXT_HEIGHT + weeks * CELL_SIZE;

/** Calculate 2D canvas height */
export const calculateCanvasHeight = (): number =>
  HEADER_HEIGHT + DAYS_IN_WEEK * CELL_SIZE + CANVAS_MARGIN;

// =====================================
// Animation & Interaction
// =====================================

/** Tooltip show delay in ms */
export const TOOLTIP_DELAY = 100;

/** Transition duration for theme changes */
export const THEME_TRANSITION_DURATION = 200;

/** Debounce delay for canvas interactions */
export const INTERACTION_DEBOUNCE = 16; // ~60fps
