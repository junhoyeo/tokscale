import { describe, expect, it } from "vitest";
import {
  calendarInstantInTimeZone,
  extendDateRangeEndToToday,
  getTodayInTimeZone,
  isValidTimeZone,
  resolveEffectiveTimeZone,
} from "@/lib/timezone";

// 2026-07-23T16:00:00Z — still the 23rd in UTC and New York, already the
// 24th in Seoul. This is the case where a KST user's early-morning usage is
// bucketed as the 24th while the server's UTC "today" is still the 23rd.
const NOW = new Date("2026-07-23T16:00:00.000Z");

function formatInTimeZone(timestamp: number, timeZone: string): string {
  return new Intl.DateTimeFormat("en-US", {
    timeZone,
    year: "numeric",
    month: "2-digit",
    day: "2-digit",
    weekday: "long",
  }).format(new Date(timestamp));
}

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
    const browserZone =
      Intl.DateTimeFormat().resolvedOptions().timeZone ?? "UTC";
    expect(resolveEffectiveTimeZone("auto")).toBe(browserZone);
    expect(resolveEffectiveTimeZone("Not/AZone")).toBe(browserZone);
  });
});

describe("getTodayInTimeZone", () => {
  it("returns the calendar date in the requested zone, not UTC", () => {
    expect(getTodayInTimeZone("UTC", NOW)).toBe("2026-07-23");
    expect(getTodayInTimeZone("Asia/Seoul", NOW)).toBe("2026-07-24");
    expect(getTodayInTimeZone("America/New_York", NOW)).toBe("2026-07-23");
  });
});

describe("extendDateRangeEndToToday", () => {
  it("extends a UTC-bucketed end when local today is ahead", () => {
    expect(extendDateRangeEndToToday("2026-07-23", "Asia/Seoul", NOW)).toBe(
      "2026-07-24",
    );
  });

  it("keeps the end when local today is not ahead", () => {
    expect(extendDateRangeEndToToday("2026-07-23", "UTC", NOW)).toBe(
      "2026-07-23",
    );
    expect(
      extendDateRangeEndToToday("2026-12-31", "Asia/Seoul", NOW),
    ).toBe("2026-12-31");
  });

  it("passes through a missing end", () => {
    expect(extendDateRangeEndToToday(null, "Asia/Seoul", NOW)).toBeNull();
    expect(extendDateRangeEndToToday(undefined, "Asia/Seoul", NOW)).toBeNull();
  });
});

describe("calendarInstantInTimeZone", () => {
  const UTC_MIDNIGHT = Date.UTC(2026, 6, 24); // 2026-07-24, a Friday

  it("returns the input unchanged for UTC", () => {
    expect(calendarInstantInTimeZone(UTC_MIDNIGHT, "UTC")).toBe(UTC_MIDNIGHT);
  });

  it("anchors the instant so formatters label the intended calendar day", () => {
    for (const timeZone of [
      "Asia/Seoul",
      "America/New_York",
      "Pacific/Auckland",
      "Pacific/Kiritimati",
    ]) {
      const instant = calendarInstantInTimeZone(UTC_MIDNIGHT, timeZone);
      expect(formatInTimeZone(instant, timeZone)).toBe("Friday, 07/24/2026");
    }
  });
});
