export const NARROW_TERMINAL_WIDTH = 80;
export const VERY_NARROW_TERMINAL_WIDTH = 60;
export const MICRO_TERMINAL_WIDTH = 40;

export const isNarrow = (width: number | undefined): boolean =>
  (width ?? 100) < NARROW_TERMINAL_WIDTH;

export const isVeryNarrow = (width: number | undefined): boolean =>
  (width ?? 100) < VERY_NARROW_TERMINAL_WIDTH;

export const isMicro = (width: number | undefined): boolean =>
  (width ?? 100) < MICRO_TERMINAL_WIDTH;

/**
 * Detects if the terminal requires ASCII-only output.
 * Uses conservative detection:
 * - TERM=dumb indicates a minimal terminal
 * - CI=true indicates automated environment (may not support Unicode)
 * - Lack of UTF-8 in locale settings indicates limited encoding support
 * 
 * Result is cached after first call for performance.
 */
let asciiModeCache: boolean | null = null;

export const shouldUseAscii = (): boolean => {
  if (asciiModeCache !== null) return asciiModeCache;
  
  const term = process.env.TERM || '';
  if (term === 'dumb') {
    asciiModeCache = true;
    return true;
  }
  
  if (process.env.CI === 'true' || process.env.CI === '1') {
    asciiModeCache = true;
    return true;
  }
  
  const locale = process.env.LC_CTYPE || process.env.LC_ALL || process.env.LANG || '';
  if (locale && !locale.toLowerCase().includes('utf')) {
    asciiModeCache = true;
    return true;
  }
  
  asciiModeCache = false;
  return false;
};

/**
 * Clears the ASCII mode cache (useful for testing)
 */
export const clearAsciiModeCache = (): void => {
  asciiModeCache = null;
};
