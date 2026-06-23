import { describe, it, expect } from "vitest";
import { formatRelativeTime } from "../../src/lib/format";

const NOW = new Date("2026-06-10T12:00:00.000Z");

describe("formatRelativeTime", () => {
  it("returns 'never' for null, undefined, and invalid input", () => {
    expect(formatRelativeTime(null, NOW)).toBe("never");
    expect(formatRelativeTime(undefined, NOW)).toBe("never");
    expect(formatRelativeTime("not-a-date", NOW)).toBe("never");
  });

  it("returns 'just now' for timestamps under a minute old", () => {
    expect(formatRelativeTime("2026-06-10T11:59:30.000Z", NOW)).toBe("just now");
    expect(formatRelativeTime("2026-06-10T12:00:00.000Z", NOW)).toBe("just now");
  });

  it("clamps future timestamps to 'just now'", () => {
    expect(formatRelativeTime("2026-06-10T13:00:00.000Z", NOW)).toBe("just now");
  });

  it("formats minutes", () => {
    expect(formatRelativeTime("2026-06-10T11:55:00.000Z", NOW)).toBe("5m ago");
    expect(formatRelativeTime("2026-06-10T11:01:00.000Z", NOW)).toBe("59m ago");
  });

  it("formats hours", () => {
    expect(formatRelativeTime("2026-06-10T09:00:00.000Z", NOW)).toBe("3h ago");
    expect(formatRelativeTime("2026-06-09T12:30:00.000Z", NOW)).toBe("23h ago");
  });

  it("formats days", () => {
    expect(formatRelativeTime("2026-06-09T11:00:00.000Z", NOW)).toBe("1d ago");
    expect(formatRelativeTime("2026-05-12T12:00:00.000Z", NOW)).toBe("29d ago");
  });

  it("formats months", () => {
    expect(formatRelativeTime("2026-05-11T12:00:00.000Z", NOW)).toBe("1mo ago");
    expect(formatRelativeTime("2025-06-12T12:00:00.000Z", NOW)).toBe("12mo ago");
  });

  it("formats years", () => {
    expect(formatRelativeTime("2025-06-10T12:00:00.000Z", NOW)).toBe("1y ago");
    expect(formatRelativeTime("2023-06-01T12:00:00.000Z", NOW)).toBe("3y ago");
  });
});
