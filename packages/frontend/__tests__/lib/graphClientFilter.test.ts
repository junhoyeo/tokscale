import { describe, expect, it } from "vitest";

import { toggleClientFilter, resolveSelectedDay } from "../../src/lib/utils";
import type { ClientType, DailyContribution } from "../../src/lib/types";

const availableClients: ClientType[] = ["claude", "codex", "gemini"];

function day(date: string): DailyContribution {
  return {
    date,
    totals: { tokens: 0, cost: 0, messages: 0 },
    intensity: 0,
    tokenBreakdown: { input: 0, output: 0, cacheRead: 0, cacheWrite: 0, reasoning: 0 },
    clients: [],
  };
}

describe("toggleClientFilter", () => {
  it("treats an empty filter as all clients, so a click deselects that client", () => {
    // Empty filter renders every chip active (show-all). Clicking one should remove
    // it from the full set, not select only that one (the pre-fix inversion bug).
    expect(toggleClientFilter("codex", [], availableClients)).toEqual(["claude", "gemini"]);
  });

  it("removes a client from an explicit subset when it is present", () => {
    expect(toggleClientFilter("codex", ["claude", "codex"], availableClients)).toEqual(["claude"]);
  });

  it("adds a client to an explicit subset when it is absent", () => {
    expect(toggleClientFilter("gemini", ["claude"], availableClients)).toEqual(["claude", "gemini"]);
  });

  it("normalizes back to the empty show-all sentinel when the toggle selects every client", () => {
    // Adding the last missing client would explicitly list all of them; collapse to []
    // so the Clear/Show-all affordances stay consistent.
    expect(toggleClientFilter("gemini", ["claude", "codex"], availableClients)).toEqual([]);
  });

  it("normalizes to the empty show-all sentinel when the last selected client is removed", () => {
    // Removing the only remaining client leaves zero selected, which means show-all
    // everywhere else in the UI — reuse the empty sentinel instead of a new one.
    expect(toggleClientFilter("claude", ["claude"], availableClients)).toEqual([]);
  });

  it("round-trips: deselect from empty then reselect returns to the empty sentinel", () => {
    const afterFirstClick = toggleClientFilter("codex", [], availableClients);
    expect(afterFirstClick).toEqual(["claude", "gemini"]);
    // Clicking the same chip again re-adds it, restoring the full set -> empty sentinel.
    expect(toggleClientFilter("codex", afterFirstClick, availableClients)).toEqual([]);
  });
});

describe("resolveSelectedDay", () => {
  const contributions = [day("2026-07-10"), day("2026-07-11"), day("2026-07-12")];

  it("returns null when no date is selected", () => {
    expect(resolveSelectedDay(null, contributions)).toBeNull();
  });

  it("resolves the live day object for a date present in the current data", () => {
    const resolved = resolveSelectedDay("2026-07-11", contributions);
    expect(resolved).toBe(contributions[1]);
  });

  it("returns null (closing the panel) when the date is absent from the filtered data", () => {
    expect(resolveSelectedDay("2026-01-01", contributions)).toBeNull();
  });

  it("re-resolves to the fresh object after the underlying data is replaced", () => {
    const filtered = [day("2026-07-11")];
    // Same date, different (filtered) array -> caller gets the new object, never stale.
    expect(resolveSelectedDay("2026-07-11", filtered)).toBe(filtered[0]);
    expect(resolveSelectedDay("2026-07-11", filtered)).not.toBe(contributions[1]);
  });
});
