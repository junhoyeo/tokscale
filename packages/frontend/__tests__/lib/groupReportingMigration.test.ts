import { readFileSync } from "node:fs";
import { describe, expect, it } from "vitest";

describe("group reporting calendar migration", () => {
  it("validates the weekday constraint without holding an exclusive lock for the table scan", () => {
    const migration = readFileSync(
      new URL(
        "../../src/lib/db/migrations/0031_group_week_start_constraint.sql",
        import.meta.url,
      ),
      "utf8",
    );

    const addConstraint = migration.indexOf(
      'ADD CONSTRAINT "groups_week_starts_on_check"',
    );
    const notValid = migration.indexOf("NOT VALID", addConstraint);
    const validate = migration.indexOf(
      'VALIDATE CONSTRAINT "groups_week_starts_on_check"',
      notValid,
    );

    expect(addConstraint).toBeGreaterThanOrEqual(0);
    expect(notValid).toBeGreaterThan(addConstraint);
    expect(validate).toBeGreaterThan(notValid);
  });
});
