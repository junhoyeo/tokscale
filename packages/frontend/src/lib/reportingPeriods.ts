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

export interface ReportDateRange {
  start: string;
  end: string;
}

/** Rolling profile periods and UTC calendar periods, evaluated at one anchor. */
export function getProfileDateRange(period: ProfilePeriod, anchor: Date): ReportDateRange | null {
  if (period === "all") return null;
  if (period === "this-month" || period === "last-month") {
    return getCalendarDateRange(period === "this-month" ? "month" : period, anchor);
  }
  const end = new Date(Date.UTC(anchor.getUTCFullYear(), anchor.getUTCMonth(), anchor.getUTCDate()));
  const start = new Date(end);
  start.setUTCDate(start.getUTCDate() - (period === "week" ? 6 : 29));
  return { start: start.toISOString().slice(0, 10), end: end.toISOString().slice(0, 10) };
}

export function describeRankingWindow(range: ReportDateRange | null | undefined): string {
  return range ? `Rank uses UTC dates ${range.start} through ${range.end}. Personal totals may include newer local dates.` : "All-time rank";
}

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

/** Do not let a finite-period HTTP response survive its UTC day boundary. */
export function finitePeriodCacheControl(startedAt: number, now = Date.now()): string {
  const started = new Date(startedAt);
  const midnight = Date.UTC(started.getUTCFullYear(), started.getUTCMonth(), started.getUTCDate() + 1);
  const remaining = Math.floor((midnight - now) / 1000);
  if (remaining <= 0) return "no-store";
  const fresh = Math.min(60, remaining);
  const stale = Math.min(300, remaining - fresh);
  return `public, max-age=0, s-maxage=${fresh}, stale-while-revalidate=${stale}`;
}
