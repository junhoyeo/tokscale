import { describe, expect, it } from "vitest";
import {
  getBrowserTimeZone,
  isValidTimeZone,
  resolveEffectiveTimeZone,
} from "@/lib/timezone";

describe("isValidTimeZone", () => {
  it("accepts IANA zones and rejects garbage", () => {
    expect(isValidTimeZone("UTC")).toBe(true);
    expect(isValidTimeZone("Asia/Seoul")).toBe(true);
    expect(isValidTimeZone("America/New_York")).toBe(true);
    expect(isValidTimeZone("Not/AZone")).toBe(false);
    expect(isValidTimeZone("auto")).toBe(false);
    expect(isValidTimeZone("")).toBe(false);
  });
});

describe("resolveEffectiveTimeZone", () => {
  it("keeps an explicit valid zone", () => {
    expect(resolveEffectiveTimeZone("Asia/Seoul")).toBe("Asia/Seoul");
  });

  it("resolves auto and invalid values to the browser zone", () => {
    expect(resolveEffectiveTimeZone("auto")).toBe(getBrowserTimeZone());
    expect(resolveEffectiveTimeZone("Not/AZone")).toBe(getBrowserTimeZone());
  });
});
