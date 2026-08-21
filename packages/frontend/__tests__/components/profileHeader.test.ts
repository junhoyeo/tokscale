// Pin a non-UTC timezone BEFORE any Date/Intl usage so the "local timezone"
// assertions are deterministic on any host (CI often runs in UTC). America/
// New_York is -04:00 in July (EDT) and -05:00 in January (EST).
process.env.TZ = "America/New_York";

import { describe, expect, it } from "vitest";
import { formatJoined, formatLastUpdated } from "../../src/components/profile";

// A fixed instant, 2026-07-10 15:30:00 UTC.
const ISO_A = "2026-07-10T15:30:00.000Z";
// A second, different instant used to prove the output tracks the *current*
// argument rather than a previously-captured value.
const ISO_B = "2025-01-02T08:05:00.000Z";

const UTC_A = "7/10/2026, 3:30:00 PM"; // server render (UTC)
const UTC_B = "1/2/2025, 8:05:00 AM";

const LOCAL_A = "7/10/2026, 11:30:00 AM"; // viewer render (America/New_York, EDT)
const LOCAL_B = "1/2/2025, 3:05:00 AM"; // America/New_York, EST

describe("formatLastUpdated", () => {
  it("returns null when there is no timestamp", () => {
    expect(formatLastUpdated(undefined, false)).toBeNull();
    expect(formatLastUpdated(null, true)).toBeNull();
    expect(formatLastUpdated("", true)).toBeNull();
  });

  it("formats in UTC before mount so the first client render matches the server (no hydration mismatch)", () => {
    // The server cannot know the viewer's timezone, so the pre-mount value must
    // be the stable UTC string that the server also renders.
    expect(formatLastUpdated(ISO_A, false)).toBe(UTC_A);
    expect(formatLastUpdated(ISO_B, false)).toBe(UTC_B);
  });

  it("formats in the viewer's local (non-UTC) timezone after mount", () => {
    const localA = formatLastUpdated(ISO_A, true);
    expect(localA).toBe(LOCAL_A);
    // The local value must genuinely differ from the UTC/server value, proving
    // the viewer's timezone is applied rather than UTC.
    expect(localA).not.toBe(UTC_A);
  });

  it("passes the caller's zone through instead of the host's", () => {
    // ProfileOverview supplies "UTC" before mount and the viewer's display zone
    // after, so an ignored third argument would silently reintroduce the
    // hydration mismatch the isMounted branch exists to prevent.
    expect(formatLastUpdated(ISO_A, true, "UTC")).toBe(UTC_A);
    expect(formatLastUpdated(ISO_A, true, "Asia/Seoul")).toBe(
      "7/11/2026, 12:30:00 AM",
    );
    // Pre-mount stays UTC no matter what zone is handed in.
    expect(formatLastUpdated(ISO_A, false, "Asia/Seoul")).toBe(UTC_A);
  });

  it("derives the label from the current timestamp with no stale carry-over", () => {
    // Regression guard for the previous implementation, which stashed the
    // formatted string in useState and refreshed it from a useEffect keyed on
    // `lastUpdated`. On the render where the prop changed, that lagging state
    // still held the PREVIOUS timestamp's formatted value. Because the value is
    // now derived purely from the current argument, a change is reflected
    // immediately — regardless of any prior call.
    expect(formatLastUpdated(ISO_A, true)).toBe(LOCAL_A);
    // Simulate the prop changing to a new value: the output must be B's value,
    // never a stale A.
    expect(formatLastUpdated(ISO_B, true)).toBe(LOCAL_B);
    // And back again — still tracks the current argument exactly.
    expect(formatLastUpdated(ISO_A, true)).toBe(LOCAL_A);
  });
});

// 2026-01-01 00:30 UTC: still December in the Americas, already January in Asia.
const JOINED_BOUNDARY = "2026-01-01T00:30:00.000Z";

describe("formatJoined", () => {
  it("returns null when there is no usable timestamp", () => {
    expect(formatJoined(undefined)).toBeNull();
    expect(formatJoined(null)).toBeNull();
    expect(formatJoined("")).toBeNull();
    expect(formatJoined("not-a-date")).toBeNull();
  });

  it("formats in UTC when the caller pins UTC, matching the server render", () => {
    // ProfileOverview passes "UTC" until it has mounted, so this is the string
    // the SSR markup carries; anything else here is a hydration mismatch.
    expect(formatJoined(JOINED_BOUNDARY, "UTC")).toBe("Jan 2026");
  });

  it("renders the joined instant in the zone it is given", () => {
    // "Joined" is an absolute instant, not a calendar bucket, so two viewers
    // straddling a UTC boundary reading different months is the intended
    // behavior of the display-timezone preference — not a bug to pin to UTC.
    expect(formatJoined(JOINED_BOUNDARY, "Asia/Seoul")).toBe("Jan 2026");
    expect(formatJoined(JOINED_BOUNDARY, "America/Los_Angeles")).toBe(
      "Dec 2025",
    );
  });

  it("falls back to the host zone when no zone is given", () => {
    // TZ is pinned to America/New_York at the top of this file.
    expect(formatJoined(JOINED_BOUNDARY)).toBe("Dec 2025");
  });
});
