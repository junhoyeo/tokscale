// Pin a non-UTC timezone BEFORE any Date/Intl usage so the fallback assertions
// name a literal zone instead of re-deriving it from the code under test. CI
// often runs in UTC, where a `getBrowserTimeZone()` that ignored the host and
// returned "UTC" outright would look correct.
process.env.TZ = "America/New_York";

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

describe("getBrowserTimeZone", () => {
  it("reports the host zone", () => {
    expect(getBrowserTimeZone()).toBe("America/New_York");
  });
});

describe("resolveEffectiveTimeZone", () => {
  it("keeps an explicit valid zone", () => {
    expect(resolveEffectiveTimeZone("Asia/Seoul")).toBe("Asia/Seoul");
  });

  it("resolves auto and invalid values to the host zone", () => {
    expect(resolveEffectiveTimeZone("auto")).toBe("America/New_York");
    expect(resolveEffectiveTimeZone("Not/AZone")).toBe("America/New_York");
  });
});
