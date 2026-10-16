export const PROFILE_PERIODS = ["all", "month", "week", "this-month", "last-month"] as const;
export type ProfilePeriod = (typeof PROFILE_PERIODS)[number];

export const PROFILE_PERIOD_LABELS: Record<ProfilePeriod, string> = {
  all: "Lifetime",
  month: "30d",
  week: "7d",
  "this-month": "This month",
  "last-month": "Last month",
};

export const GROUP_PERIODS = ["all", "month", "last-month", "week", "last-week"] as const;
export type GroupPeriod = (typeof GROUP_PERIODS)[number];

export function getCalendarDateRange(
  period: Exclude<GroupPeriod, "all">,
  now: Date,
  timeZone = "UTC",
  weekStartsOn = 1,
): { start: string; end: string } {
  const parts = new Intl.DateTimeFormat("en-US", {
    timeZone, year: "numeric", month: "2-digit", day: "2-digit",
  }).formatToParts(now);
  const part = (type: Intl.DateTimeFormatPartTypes) => Number(parts.find((p) => p.type === type)!.value);
  const today = new Date(Date.UTC(part("year"), part("month") - 1, part("day")));
  let start = new Date(today);
  let end = new Date(today);

  if (period === "month") {
    start.setUTCDate(1);
  } else if (period === "last-month") {
    end.setUTCDate(0);
    start = new Date(end);
    start.setUTCDate(1);
  } else {
    start.setUTCDate(start.getUTCDate() - (start.getUTCDay() - weekStartsOn + 7) % 7);
    if (period === "last-week") {
      end = new Date(start);
      end.setUTCDate(end.getUTCDate() - 1);
      start.setUTCDate(start.getUTCDate() - 7);
    }
  }

  // Submitted dates are day buckets, not instants to reproject into this zone.
  return { start: start.toISOString().slice(0, 10), end: end.toISOString().slice(0, 10) };
}
