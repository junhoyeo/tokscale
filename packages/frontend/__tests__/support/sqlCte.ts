import { expect } from "vitest";

/**
 * The body of one CTE in a rendered statement: from `<name> AS (` up to the
 * next CTE's `<next> AS (`. Asserting inside one CTE catches a filter that
 * moved to a sibling CTE, which a whole-statement `toContain` cannot.
 */
export function cteBody(sql: string, name: string, next: string): string {
  const start = sql.indexOf(`${name} AS (`);
  const end = sql.indexOf(`${next} AS (`, start);
  expect(start, `${name} CTE missing`).toBeGreaterThanOrEqual(0);
  expect(end, `${next} CTE missing after ${name}`).toBeGreaterThan(start);
  return sql.slice(start, end);
}
