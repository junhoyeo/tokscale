import { describe, expect, it } from "vitest";
import { MEASURE_DISPLAY_ORDER, displayRank, sortForDisplay } from "@/lib/capital/ledger/display";
import { MEASURES, type Measure } from "@/lib/capital/ledger/schema";

describe("amount display order", () => {
  it("ranks every measure exactly once", () => {
    expect([...MEASURE_DISPLAY_ORDER].sort()).toEqual([...MEASURES].sort());
  });

  it("shows the amount sold under a Form D before the total offering size", () => {
    const formD: Array<{ id: string; measure: Measure }> = [
      { id: "offering", measure: "form_d_offering_amount" },
      { id: "sold", measure: "form_d_amount_sold" },
    ];
    expect(sortForDisplay(formD, (a) => a.measure).map((a) => a.id)).toEqual(["sold", "offering"]);
  });

  it("puts money raised before valuations and per-share prices, and keeps file order within a measure", () => {
    const amounts: Array<{ id: string; measure: Measure }> = [
      { id: "price", measure: "price_per_share" },
      { id: "post", measure: "valuation_post" },
      { id: "notes_a", measure: "debt_raised" },
      { id: "equity", measure: "equity_raised" },
      { id: "notes_b", measure: "debt_raised" },
    ];
    expect(sortForDisplay(amounts, (a) => a.measure).map((a) => a.id)).toEqual(["equity", "notes_a", "notes_b", "post", "price"]);
    expect(displayRank("ipo_proceeds")).toBeLessThan(displayRank("form_d_offering_amount"));
    expect(displayRank("cumulative_investment")).toBeGreaterThan(displayRank("equity_investment"));
  });
});
