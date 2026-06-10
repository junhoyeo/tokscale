import { describe, it, expect } from "vitest";
import { formatTokenCount } from "../../src/lib/utils";

describe("formatTokenCount", () => {
  it("formats small numbers with locale separators", () => {
    expect(formatTokenCount(0)).toBe("0");
    expect(formatTokenCount(42)).toBe("42");
    expect(formatTokenCount(999)).toBe("999");
  });

  it("formats thousands with K suffix", () => {
    expect(formatTokenCount(1_000)).toBe("1.0K");
    expect(formatTokenCount(1_500)).toBe("1.5K");
    expect(formatTokenCount(123_456)).toBe("123.5K");
  });

  it("formats millions with M suffix", () => {
    expect(formatTokenCount(1_000_000)).toBe("1.0M");
    expect(formatTokenCount(1_234_567)).toBe("1.2M");
  });

  it("formats billions with B suffix", () => {
    expect(formatTokenCount(1_000_000_000)).toBe("1.0B");
    expect(formatTokenCount(2_500_000_000)).toBe("2.5B");
  });

  it("formats trillions with T suffix", () => {
    expect(formatTokenCount(1_000_000_000_000)).toBe("1T");
    expect(formatTokenCount(1_500_000_000_000)).toBe("1.5T");
  });

  // Regression: values near unit boundaries should promote to the next unit
  // instead of displaying "1000.0K", "1000.0M", etc.
  // https://github.com/junhoyeo/tokscale/issues/474
  it("promotes 999_950 to 1.0M instead of 1000.0K", () => {
    expect(formatTokenCount(999_950)).toBe("1.0M");
  });

  it("promotes 999_999_500 to 1.0B instead of 1000.0M", () => {
    expect(formatTokenCount(999_999_500)).toBe("1.0B");
  });

  it("promotes 999_999_999_500 to 1.0T instead of 1000.0B", () => {
    expect(formatTokenCount(999_999_999_500)).toBe("1.0T");
  });

  it("does not promote values well below the boundary", () => {
    expect(formatTokenCount(999_949)).toBe("999.9K");
  });
});
