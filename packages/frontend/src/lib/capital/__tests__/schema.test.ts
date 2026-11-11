import { describe, expect, it } from "vitest";
import { isoDate, partialDate } from "@/lib/capital/ledger/schema";

describe("Capital date schemas", () => {
  it.each(["2026-02-30", "2026-13-01", "2026-00-01"])("rejects impossible ISO date %s", (value) => {
    expect(isoDate.safeParse(value).success).toBe(false);
  });

  it.each(["2026-00", "2026-13", "2026-02-30"])("rejects impossible partial date %s", (value) => {
    expect(partialDate.safeParse(value).success).toBe(false);
  });

  it("accepts a valid leap day in both schemas", () => {
    expect(isoDate.safeParse("2028-02-29").success).toBe(true);
    expect(partialDate.safeParse("2028-02-29").success).toBe(true);
  });

  it.each(["2026", "2026-01", "2026-01-31"])("keeps valid partial date %s", (value) => {
    expect(partialDate.safeParse(value).success).toBe(true);
  });
});
