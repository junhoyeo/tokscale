import { describe, it, expect } from "vitest";
import { formatCurrency } from "../../src/lib/utils";

describe("formatCurrency", () => {
  it("formats sub-thousand amounts as plain dollars", () => {
    expect(formatCurrency(0)).toBe("$0.00");
    expect(formatCurrency(42.5)).toBe("$42.50");
    expect(formatCurrency(999.99)).toBe("$999.99");
  });

  it("formats thousands with K suffix", () => {
    expect(formatCurrency(1_000)).toBe("$1.00K");
    expect(formatCurrency(1_500)).toBe("$1.50K");
    expect(formatCurrency(123_456)).toBe("$123.46K");
  });

  it("formats millions with M suffix", () => {
    expect(formatCurrency(1_000_000)).toBe("$1.00M");
    expect(formatCurrency(2_345_678)).toBe("$2.35M");
    // Regression: production leaderboard total rendered as "$391803.96K"
    expect(formatCurrency(391_803_960)).toBe("$391.80M");
  });

  it("formats billions with B suffix", () => {
    expect(formatCurrency(1_000_000_000)).toBe("$1.00B");
    expect(formatCurrency(2_500_000_000)).toBe("$2.50B");
  });

  // Values near unit boundaries should promote to the next unit instead of
  // displaying "$1000.00K" / "$1000.00M" (same guard as formatTokenCount).
  it("promotes 999_995 to $1.00M instead of $1000.00K", () => {
    expect(formatCurrency(999_995)).toBe("$1.00M");
  });

  it("promotes 999_995_000 to $1.00B instead of $1000.00M", () => {
    expect(formatCurrency(999_995_000)).toBe("$1.00B");
  });

  it("does not promote values well below the boundary", () => {
    expect(formatCurrency(999_994)).toBe("$999.99K");
  });
});
