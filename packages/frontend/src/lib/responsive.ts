export const BREAKPOINTS = {
  xs: 320,
  sm: 390,
  md: 480,
  lg: 640,
  xl: 768,
  '2xl': 1024,
  '3xl': 1280,
} as const;

export const LEGACY_BREAKPOINTS = {
  phone: 560,
  compact: 480,
  tablet: 768,
  navXs: 374,
  navMd: 767,
} as const;

export type Breakpoint = keyof typeof BREAKPOINTS;
export type LegacyBreakpoint = keyof typeof LEGACY_BREAKPOINTS;

/**
 * @example ${up('md')} { padding: 2rem; }
 */
export const up = (bp: Breakpoint) => `@media (min-width: ${BREAKPOINTS[bp]}px)`;

/**
 * @example ${down('lg')} { padding: 1rem; }
 */
export const down = (bp: Breakpoint) => `@media (max-width: ${BREAKPOINTS[bp] - 1}px)`;

/**
 * @example ${between('sm', 'lg')} { padding: 1.5rem; }
 */
export const between = (min: Breakpoint, max: Breakpoint) =>
  `@media (min-width: ${BREAKPOINTS[min]}px) and (max-width: ${BREAKPOINTS[max] - 1}px})`;

export const legacy = {
  down: (bp: LegacyBreakpoint) => `@media (max-width: ${LEGACY_BREAKPOINTS[bp]}px)`,
  up: (bp: LegacyBreakpoint) => `@media (min-width: ${LEGACY_BREAKPOINTS[bp] + 1}px)`,
};
