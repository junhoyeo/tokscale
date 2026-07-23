/**
 * Display-timezone helpers for the web dashboard.
 *
 * Contribution dates are calendar-day buckets (`YYYY-MM-DD`) submitted in the
 * user's local timezone, while the server computes chart ranges against UTC
 * "today". These helpers resolve the viewer's display timezone preference and
 * anchor calendar dates to it so freshly submitted local-today data renders
 * immediately. Display-only: the leaderboard stays UTC-based.
 */

/** A modest list of common IANA zones for the settings select, by region. */
export const COMMON_TIMEZONE_GROUPS: ReadonlyArray<{
  region: string;
  zones: readonly string[];
}> = [
  {
    region: "Americas",
    zones: [
      "America/New_York",
      "America/Chicago",
      "America/Denver",
      "America/Los_Angeles",
      "America/Anchorage",
      "Pacific/Honolulu",
      "America/Toronto",
      "America/Vancouver",
      "America/Mexico_City",
      "America/Sao_Paulo",
      "America/Argentina/Buenos_Aires",
    ],
  },
  {
    region: "Europe",
    zones: [
      "Europe/London",
      "Europe/Dublin",
      "Europe/Paris",
      "Europe/Berlin",
      "Europe/Madrid",
      "Europe/Rome",
      "Europe/Amsterdam",
      "Europe/Stockholm",
      "Europe/Warsaw",
      "Europe/Athens",
      "Europe/Helsinki",
      "Europe/Istanbul",
      "Europe/Moscow",
    ],
  },
  {
    region: "Africa",
    zones: [
      "Africa/Cairo",
      "Africa/Lagos",
      "Africa/Nairobi",
      "Africa/Johannesburg",
    ],
  },
  {
    region: "Asia",
    zones: [
      "Asia/Dubai",
      "Asia/Karachi",
      "Asia/Kolkata",
      "Asia/Dhaka",
      "Asia/Bangkok",
      "Asia/Jakarta",
      "Asia/Shanghai",
      "Asia/Singapore",
      "Asia/Hong_Kong",
      "Asia/Taipei",
      "Asia/Seoul",
      "Asia/Tokyo",
    ],
  },
  {
    region: "Oceania",
    zones: [
      "Australia/Perth",
      "Australia/Melbourne",
      "Australia/Sydney",
      "Pacific/Auckland",
      "Pacific/Fiji",
    ],
  },
  {
    region: "Other",
    zones: ["UTC", "Atlantic/Reykjavik", "Atlantic/Azores"],
  },
];

export function isValidTimeZone(value: string): boolean {
  try {
    new Intl.DateTimeFormat("en-US", { timeZone: value });
    return true;
  } catch {
    return false;
  }
}

export function getBrowserTimeZone(): string {
  try {
    const timeZone = Intl.DateTimeFormat().resolvedOptions().timeZone;
    return timeZone && isValidTimeZone(timeZone) ? timeZone : "UTC";
  } catch {
    return "UTC";
  }
}

/** Resolve a stored preference (`"auto"` or an IANA zone) to a usable zone. */
export function resolveEffectiveTimeZone(preference: string): string {
  return preference !== "auto" && isValidTimeZone(preference)
    ? preference
    : getBrowserTimeZone();
}

const dayPartsFormatters = new Map<string, Intl.DateTimeFormat>();

function getDayPartsFormatter(timeZone: string): Intl.DateTimeFormat {
  let formatter = dayPartsFormatters.get(timeZone);
  if (!formatter) {
    formatter = new Intl.DateTimeFormat("en-US", {
      timeZone,
      year: "numeric",
      month: "2-digit",
      day: "2-digit",
    });
    dayPartsFormatters.set(timeZone, formatter);
  }
  return formatter;
}

/** "Today" as `YYYY-MM-DD` in the given timezone (never `toISOString`, which is UTC). */
export function getTodayInTimeZone(
  timeZone: string,
  now: Date = new Date(),
): string {
  const parts = getDayPartsFormatter(timeZone).formatToParts(now);
  const part = (type: string) =>
    parts.find((candidate) => candidate.type === type)?.value ?? "";
  return `${part("year")}-${part("month")}-${part("day")}`;
}

/**
 * Extend a range end to "today" in the effective timezone when the local day
 * is ahead of the (UTC-bucketed) end, so same-day contributions are not
 * clipped from profile charts. Never shortens the range.
 */
export function extendDateRangeEndToToday(
  rangeEnd: string | null | undefined,
  timeZone: string,
  now: Date = new Date(),
): string | null {
  if (!rangeEnd) return null;
  const today = getTodayInTimeZone(timeZone, now);
  return today > rangeEnd ? today : rangeEnd;
}

const offsetPartsFormatters = new Map<string, Intl.DateTimeFormat>();

function getOffsetPartsFormatter(timeZone: string): Intl.DateTimeFormat {
  let formatter = offsetPartsFormatters.get(timeZone);
  if (!formatter) {
    formatter = new Intl.DateTimeFormat("en-US", {
      timeZone,
      hourCycle: "h23",
      year: "numeric",
      month: "2-digit",
      day: "2-digit",
      hour: "2-digit",
      minute: "2-digit",
      second: "2-digit",
    });
    offsetPartsFormatters.set(timeZone, formatter);
  }
  return formatter;
}

function getTimeZoneOffsetMs(timeZone: string, utcMs: number): number {
  const parts = getOffsetPartsFormatter(timeZone).formatToParts(
    new Date(utcMs),
  );
  const part = (type: string) =>
    Number(parts.find((candidate) => candidate.type === type)?.value ?? 0);
  const asUtcMs = Date.UTC(
    part("year"),
    part("month") - 1,
    part("day"),
    part("hour"),
    part("minute"),
    part("second"),
  );
  return asUtcMs - utcMs;
}

/**
 * Shift a UTC-midnight calendar timestamp to the instant of that same
 * calendar midnight in `timeZone`, so timezone-aware formatters label the
 * intended date instead of the previous/next UTC day.
 */
export function calendarInstantInTimeZone(
  utcMidnightMs: number,
  timeZone: string,
): number {
  if (timeZone === "UTC") return utcMidnightMs;
  const offset = getTimeZoneOffsetMs(timeZone, utcMidnightMs);
  let timestamp = utcMidnightMs - offset;
  // Refine once: the offset at UTC midnight can differ from the offset at
  // local midnight around DST transitions.
  const refined = getTimeZoneOffsetMs(timeZone, timestamp);
  if (refined !== offset) timestamp = utcMidnightMs - refined;
  return timestamp;
}
