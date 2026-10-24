import { describe, expect, it } from "vitest";
import { basisFamily, total, totalsByCurrency, tryQuantityTotal, tryTotal, LedgerTotalError } from "@/lib/capital/ledger/total";
import {
  formatAmountShort,
  formatAmountWords,
  formatMoney,
  formatQuantity,
  formatTerm,
  formatTotal,
  monogram,
} from "@/lib/capital/ledger/format";
import type { Amount } from "@/lib/capital/ledger/schema";

const amount = (over: Partial<Amount>): Amount => ({
  id: "a",
  measure: "equity_raised",
  value: 100,
  currency: "USD",
  basis: "exact",
  status: "closed",
  asOf: "2026-01-01",
  sourceId: "src_test",
  quote: "a quote copied from a source",
  ...over,
});

describe("ledger total() guard", () => {
  it("sums one measure, one currency and one basis family", () => {
    const t = total([amount({ id: "a", value: 65 }), amount({ id: "b", value: 30, basis: "approximately" })], {
      measure: "equity_raised",
      currency: "USD",
    });
    expect(t.value).toBe(95);
    expect(t.family).toBe("point");
    expect(t.approximate).toBe(true);
    expect(t.counted.map((a) => a.id)).toEqual(["a", "b"]);
  });

  it("throws on mixed measures, currencies and bases", () => {
    expect(() => total([amount({}), amount({ id: "b", measure: "debt_raised" })], { measure: "equity_raised", currency: "USD" })).toThrow(
      LedgerTotalError
    );
    expect(() => total([amount({}), amount({ id: "b", currency: "EUR" })], { measure: "equity_raised", currency: "USD" })).toThrow(
      /mixed currencies/
    );
    expect(() => total([amount({}), amount({ id: "b", basis: "up_to" })], { measure: "equity_raised", currency: "USD" })).toThrow(
      /mixed bases/
    );
    expect(() => total([amount({ basis: "range" })], { measure: "equity_raised", currency: "USD" })).toThrow(/range/);
  });

  it("never sums valuations, marks, run-rates, cumulative statements or estimates", () => {
    for (const measure of ["valuation_post", "fund_mark", "revenue_run_rate", "cumulative_investment", "estimated_capital_cost", "price_per_share"] as const) {
      expect(() => total([amount({ measure })], { measure, currency: "USD" })).toThrow(/cannot be summed/);
    }
  });

  it("keeps press-reported, estimated and undisclosed numbers out of totals", () => {
    expect(() => total([amount({ status: "reported" })], { measure: "equity_raised", currency: "USD" })).toThrow(/never enters a total/);
    expect(() => total([amount({ status: "estimated" })], { measure: "equity_raised", currency: "USD" })).toThrow(/never enters a total/);
    expect(() => total([amount({ status: "undisclosed", value: null })], { measure: "equity_raised", currency: "USD" })).toThrow();
  });

  it("skips includedIn, partOf and alternativeTo amounts instead of adding them twice", () => {
    const t = total(
      [
        amount({ id: "round", value: 65 }),
        amount({ id: "earlier", value: 5, includedIn: "evt_later" }),
        amount({ id: "part", value: 15, partOf: "round" }),
        amount({ id: "alt", value: 70, alternativeTo: "round", condition: "with options" }),
      ],
      { measure: "equity_raised", currency: "USD" }
    );
    expect(t.value).toBe(65);
    expect(t.skipped.map((s) => [s.amount.id, s.reason])).toEqual([
      ["earlier", "included_in"],
      ["part", "part_of"],
      ["alt", "alternative"],
    ]);
  });

  it("returns null, never zero, when nothing is countable", () => {
    expect(total([], { measure: "equity_raised", currency: "USD" }).value).toBeNull();
    expect(total([amount({ includedIn: "evt_x" })], { measure: "equity_raised", currency: "USD" }).value).toBeNull();
    expect(formatTotal(total([], { measure: "equity_raised", currency: "USD" }))).toBe("—");
  });

  it("totals per currency without converting, and tryTotal reports why a set cannot be summed", () => {
    const totals = totalsByCurrency([amount({ id: "u", value: 10 }), amount({ id: "e", value: 20, currency: "EUR" })], "equity_raised");
    expect(totals.map((t) => [t.currency, t.value])).toEqual([
      ["EUR", 20],
      ["USD", 10],
    ]);
    const mixed = tryTotal([amount({}), amount({ id: "b", basis: "more_than" })], { measure: "equity_raised", currency: "USD" });
    expect(mixed.ok).toBe(false);
    expect(basisFamily("up_to_approximately")).toBe("ceiling");
    expect(basisFamily("at_least")).toBe("floor");
    expect(basisFamily("range")).toBeNull();
  });
});

describe("capacity totals", () => {
  it("add only one measure with one basis family", () => {
    const ok = tryQuantityTotal(
      [
        { measure: "it_load_mw", value: 401, basis: "approximately" },
        { measure: "it_load_mw", value: 168, basis: "exact" },
        { measure: "capacity_gw", value: 5, basis: "up_to" },
      ],
      "it_load_mw"
    );
    expect(ok).toEqual({ ok: true, value: 569, family: "point", approximate: true, count: 2 });
    const mixed = tryQuantityTotal(
      [
        { measure: "capacity_gw", value: 5, basis: "up_to" },
        { measure: "capacity_gw", value: 3.5, basis: "approximately" },
      ],
      "capacity_gw"
    );
    expect(mixed).toMatchObject({ ok: false, largest: 5, largestBasis: "up_to" });
    expect(tryQuantityTotal([], "gpus")).toMatchObject({ ok: false, largest: null, largestBasis: null });
  });
});

describe("formatting", () => {
  it("prints stated amounts without inventing precision and keeps the currency", () => {
    expect(formatMoney(65_000_000_000, "USD")).toBe("$65B");
    expect(formatMoney(11_600_000_000, "USD")).toBe("$11.6B");
    expect(formatMoney(3_108_543_889, "USD")).toBe("$3.11B");
    expect(formatMoney(965_000_000_000, "USD")).toBe("$965B");
    expect(formatMoney(1_700_000_000, "EUR")).toBe("€1.7B");
    expect(formatAmountShort({ value: 2226.6, currency: "USD", basis: "exact", measure: "price_per_share" })).toBe("$2,226.60");
    expect(formatMoney(530_000_000, "USD")).toBe("$530M");
    expect(formatAmountShort({ value: 6_500_000_000, currency: "USD", basis: "up_to_approximately" })).toBe("≤≈$6.5B");
    expect(formatAmountWords({ value: 100_000_000_000, currency: "USD", basis: "more_than" })).toBe("more than $100B");
    expect(formatAmountShort({ value: null, currency: "USD", basis: "exact" })).toBe("Undisclosed");
    expect(formatQuantity({ value: 401, measure: "it_load_mw", basis: "approximately" }, true)).toBe("≈401 MW critical IT load");
    expect(formatTerm({ years: 7 })).toBe("7 yrs");
    expect(formatTerm({ through: "2031-05-31" })).toBe("through May 31, 2031");
    expect(monogram("Mistral AI")).toBe("MA");
    expect(monogram("Anthropic")).toBe("A");
  });
});
