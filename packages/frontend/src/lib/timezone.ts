/**
 * Display-timezone helpers for the web dashboard.
 *
 * Scope is deliberately narrow: the preference resolved here renders *absolute
 * instants* — "Updated", "Joined" — in a zone the viewer chooses. It must never
 * touch contribution dates. Those are calendar-day buckets (`YYYY-MM-DD`) that
 * the CLI already resolved in the submitting machine's local timezone, so
 * re-projecting them through a viewer's zone would move a day's usage onto a
 * day its owner never worked. See #960 for the scan-timezone side of that split.
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
