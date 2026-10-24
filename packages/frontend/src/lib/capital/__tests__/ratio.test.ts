import { describe, expect, it } from "vitest";
import { valuationRatio } from "@/lib/capital/ledger/ratio";
import { ledger } from "@/lib/capital/ledger";
import type { Basis } from "@/lib/capital/ledger/schema";

const amount = (value: number | null, basis: Basis = "exact", currency = "EUR") => ({ value, basis, currency });

describe("valuation step-up bounds", () => {
  it("preserves the actual Mistral Series D lower bound and both formula operands", () => {
    const round = ledger.getEvent("mistral-series-d-2026")!;
    const step = ledger.stepUp(round)!;
    expect(round.valuation?.short).toBe(">€21B");
    expect(step.previous.slug).toBe("mistral-series-c-2025");
    expect(step.basis).toBe("more_than");
    expect(step.short).toBe(">1.79x");
    expect(step.formula).toBe(">€21B ÷ €11.7B");
  });

  it.each<[{ top: Basis; bottom: Basis; expected: string }]>([
    [{ top: "exact", bottom: "exact", expected: "2.00x" }],
    [{ top: "approximately", bottom: "exact", expected: "≈2.00x" }],
    [{ top: "exact", bottom: "approximately", expected: "≈2.00x" }],
    [{ top: "approximately", bottom: "approximately", expected: "≈2.00x" }],
    [{ top: "more_than", bottom: "exact", expected: ">2.00x" }],
    [{ top: "at_least", bottom: "exact", expected: "≥2.00x" }],
    [{ top: "up_to", bottom: "exact", expected: "≤2.00x" }],
    [{ top: "exact", bottom: "more_than", expected: "<2.00x" }],
    [{ top: "exact", bottom: "at_least", expected: "≤2.00x" }],
    [{ top: "exact", bottom: "up_to", expected: "≥2.00x" }],
    [{ top: "more_than", bottom: "up_to", expected: ">2.00x" }],
    [{ top: "at_least", bottom: "up_to", expected: "≥2.00x" }],
    [{ top: "up_to", bottom: "more_than", expected: "<2.00x" }],
    [{ top: "up_to", bottom: "at_least", expected: "≤2.00x" }],
  ])("retains or reverses the bound for $top / $bottom", ({ top, bottom, expected }) => {
    expect(valuationRatio(amount(20, top), amount(10, bottom))?.short).toBe(expected);
  });

  it.each<[Basis, Basis]>([
    ["more_than", "more_than"], ["more_than", "at_least"], ["at_least", "more_than"], ["at_least", "at_least"],
    ["up_to", "up_to"], ["approximately", "more_than"], ["up_to", "approximately"],
    ["up_to_approximately", "exact"], ["exact", "up_to_approximately"], ["range", "exact"], ["exact", "range"],
  ])("omits the ratio for incompatible %s / %s inputs", (top, bottom) => {
    expect(valuationRatio(amount(20, top), amount(10, bottom))).toBeNull();
  });

  it("rounds lower bounds down and upper bounds up, rather than strengthening them", () => {
    expect(valuationRatio(amount(19, "more_than"), amount(11))?.short).toBe(">1.72x");
    expect(valuationRatio(amount(19, "at_least"), amount(11))?.short).toBe("≥1.72x");
    expect(valuationRatio(amount(25, "up_to"), amount(11))?.short).toBe("≤2.28x");
    expect(valuationRatio(amount(25), amount(11, "more_than"))?.short).toBe("<2.28x");
  });

  it("rejects absent, zero, non-finite or cross-currency inputs", () => {
    expect(valuationRatio(amount(20), amount(0))).toBeNull();
    expect(valuationRatio(amount(0), amount(10, "more_than"))).toBeNull();
    expect(valuationRatio(amount(null), amount(10))).toBeNull();
    expect(valuationRatio(amount(20), amount(null))).toBeNull();
    expect(valuationRatio(amount(Infinity), amount(10))).toBeNull();
    expect(valuationRatio(amount(20), amount(Infinity))).toBeNull();
    expect(valuationRatio(amount(20, "exact", "USD"), amount(10))).toBeNull();
  });
});
