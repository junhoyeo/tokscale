export const NARROW_TERMINAL_WIDTH = 80;
export const VERY_NARROW_TERMINAL_WIDTH = 60;

export const isNarrow = (width: number | undefined): boolean =>
  (width ?? 100) < NARROW_TERMINAL_WIDTH;

export const isVeryNarrow = (width: number | undefined): boolean =>
  (width ?? 100) < VERY_NARROW_TERMINAL_WIDTH;
