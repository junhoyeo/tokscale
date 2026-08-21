import { expect } from "vitest";

/**
 * Extracts every `total_cost` cost-cast precision width from a set of
 * serialized SQL statements.
 *
 * The leaderboard / profile / embed queries cast cost columns to DECIMAL for
 * `ORDER BY` and `SUM`. `submissions.total_cost` is `decimal(18,4)`; a
 * width-based check (rather than a literal `DECIMAL(18,4)` string match)
 * catches a re-narrowing to ANY precision below the column — e.g.
 * `DECIMAL(15,4)` — which would overflow on costs past the narrowed ceiling.
 */
export function costCastWidths(sqlTexts: string[]): number[] {
  return sqlTexts
    .filter((text) => /CAST\([^)]*(?:total_cost|totalCost)[^)]*AS DECIMAL/.test(text))
    .flatMap((text) =>
      [...text.matchAll(/DECIMAL\((\d+),\s*4\)/g)].map((match) => Number(match[1]))
    );
}

/**
 * Asserts the SQL casts at least one cost column and that every cost cast stays
 * at the full `decimal(18,4)` column precision (width >= 18), so a narrowed
 * cast that would overflow on large totals fails the test.
 */
export function expectNoNarrowedCostCast(sqlTexts: string[]): void {
  const widths = costCastWidths(sqlTexts);
  expect(widths.length).toBeGreaterThan(0);
  expect(widths.every((width) => width >= 18)).toBe(true);
}
