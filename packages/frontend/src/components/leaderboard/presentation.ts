import type { Period } from "@/lib/leaderboard/types";

const MONTH_NAMES = [
  "Jan",
  "Feb",
  "Mar",
  "Apr",
  "May",
  "Jun",
  "Jul",
  "Aug",
  "Sep",
  "Oct",
  "Nov",
  "Dec",
] as const;

interface CalendarDate {
  year: number;
  month: number;
  day: number;
}

function parseCalendarDate(value?: string): CalendarDate | null {
  if (!value) return null;

  const match = /^(\d{4})-(\d{2})-(\d{2})$/.exec(value);
  if (!match) return null;

  const year = Number(match[1]);
  const month = Number(match[2]);
  const day = Number(match[3]);
  const date = new Date(Date.UTC(year, month - 1, day));

  if (
    date.getUTCFullYear() !== year ||
    date.getUTCMonth() !== month - 1 ||
    date.getUTCDate() !== day
  ) {
    return null;
  }

  return { year, month, day };
}

export function getGroupMonogram(name: string): string {
  const words = name.trim().split(/\s+/u).filter(Boolean);
  if (words.length === 0) return "TG";

  const letters = words.length > 1
    ? [Array.from(words[0])[0], Array.from(words[1])[0]]
    : Array.from(words[0]).slice(0, 2);

  return letters.join("").toLocaleUpperCase("en-US");
}

export function formatGroupMemberCount(count: number): string {
  const safeCount = Number.isFinite(count) ? Math.max(0, Math.trunc(count)) : 0;
  return `${safeCount.toLocaleString("en-US")} ${safeCount === 1 ? "member" : "members"}`;
}

export function getLeaderboardPeriodLabel(
  period: Period,
  from?: string,
  to?: string,
): string {
  if (period === "all") return "All time";
  if (period === "last-month") return "Last month";
  if (period === "month") return "This month";
  if (period === "week") return "This week";

  const start = parseCalendarDate(from);
  const end = parseCalendarDate(to);
  if (!start || !end) return "Custom range";

  const startMonth = MONTH_NAMES[start.month - 1];
  const endMonth = MONTH_NAMES[end.month - 1];

  if (start.year === end.year && start.month === end.month) {
    return `${startMonth} ${start.day}–${end.day}, ${end.year}`;
  }

  if (start.year === end.year) {
    return `${startMonth} ${start.day}–${endMonth} ${end.day}, ${end.year}`;
  }

  return `${startMonth} ${start.day}, ${start.year}–${endMonth} ${end.day}, ${end.year}`;
}
