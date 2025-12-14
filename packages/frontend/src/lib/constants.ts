// 2D Canvas
export const BOX_WIDTH = 10;
export const BOX_MARGIN = 2;
export const TEXT_HEIGHT = 15;
export const CANVAS_MARGIN = 20;
export const HEADER_HEIGHT = 60;
export const BOX_BORDER_RADIUS = 2;
export const WEEKS_IN_YEAR = 53;
export const DAYS_IN_WEEK = 7;
export const FONT_SIZE = 10;
export const FONT_FAMILY = "'SF Mono', ui-monospace, Menlo, Monaco, 'Cascadia Mono', 'Segoe UI Mono', monospace";

// 3D Isometric (obelisk.js)
export const CUBE_SIZE = 16;
export const MAX_CUBE_HEIGHT = 100;
export const MIN_CUBE_HEIGHT = 3;
export const ISO_ORIGIN = { x: 130, y: 90 };
export const CUBE_GAP = 2;
export const ISO_CANVAS_WIDTH = 1000;
export const ISO_CANVAS_HEIGHT = 600;

// Labels
export const DAY_LABELS_SHORT = ["Sun", "Mon", "Tue", "Wed", "Thu", "Fri", "Sat"];
export const MONTH_LABELS_SHORT = ["Jan", "Feb", "Mar", "Apr", "May", "Jun", "Jul", "Aug", "Sep", "Oct", "Nov", "Dec"];

// Source configuration
export const SOURCE_DISPLAY_NAMES: Record<string, string> = {
  opencode: "OpenCode",
  claude: "Claude Code",
  codex: "Codex CLI",
  gemini: "Gemini CLI",
  cursor: "Cursor",
};

export const SOURCE_COLORS: Record<string, string> = {
  opencode: "#22c55e",
  claude: "#f97316",
  codex: "#3b82f6",
  gemini: "#8b5cf6",
  cursor: "#000000",
};

// Derived values
export const CELL_SIZE = BOX_WIDTH + BOX_MARGIN;

export const calculateCanvasWidth = (weeks: number = WEEKS_IN_YEAR): number =>
  CANVAS_MARGIN * 2 + TEXT_HEIGHT + weeks * CELL_SIZE;

export const calculateCanvasHeight = (): number =>
  HEADER_HEIGHT + DAYS_IN_WEEK * CELL_SIZE + CANVAS_MARGIN;

// Interaction timing
export const TOOLTIP_DELAY = 100;
export const THEME_TRANSITION_DURATION = 200;
export const INTERACTION_DEBOUNCE = 16;
