import { NextResponse } from "next/server";
import { getLeaderboardData } from "@/lib/leaderboard/getLeaderboard";
import type { Period, SortBy } from "@/lib/leaderboard/types";

export const revalidate = 60;

const VALID_PERIODS: Period[] = ["all", "month", "last-month", "week", "custom"];
const VALID_SORT_BY: SortBy[] = ["tokens", "cost"];

function parseIntSafe(value: string | null, defaultValue: number): number {
  if (!value) return defaultValue;
  const parsed = Number(value);
  return Number.isFinite(parsed) ? Math.floor(parsed) : defaultValue;
}

const DATE_REGEX = /^\d{4}-\d{2}-\d{2}$/;

function isValidDateString(value: string | null): value is string {
  if (!value || !DATE_REGEX.test(value)) return false;
  const [year, month, day] = value.split("-").map(Number);
  const date = new Date(year, month - 1, day);
  return date.getFullYear() === year && date.getMonth() === month - 1 && date.getDate() === day;
}

export async function GET(request: Request) {
  try {
    const { searchParams } = new URL(request.url);

    let periodParam = searchParams.get("period") || "all";
    let period: Period = VALID_PERIODS.includes(periodParam as Period)
      ? (periodParam as Period)
      : "all";

    const sortByParam = searchParams.get("sortBy") || "tokens";
    const sortBy: SortBy = VALID_SORT_BY.includes(sortByParam as SortBy)
      ? (sortByParam as SortBy)
      : "tokens";

    const page = Math.max(1, parseIntSafe(searchParams.get("page"), 1));
    const limit = Math.min(100, Math.max(1, parseIntSafe(searchParams.get("limit"), 50)));

    const search = (searchParams.get("search") || "").trim();

    const fromParam = searchParams.get("from");
    const toParam = searchParams.get("to");

    let customFrom: string | undefined;
    let customTo: string | undefined;

    if (period === "custom") {
      if (isValidDateString(fromParam) && isValidDateString(toParam)) {
        customFrom = fromParam;
        customTo = toParam;
      } else {
        period = "all";
      }
    }

    const data = await getLeaderboardData(period, page, limit, sortBy, search, customFrom, customTo);

    return NextResponse.json(data);
  } catch (error) {
    console.error("Leaderboard error:", error);
    return NextResponse.json(
      { error: "Failed to fetch leaderboard" },
      { status: 500 }
    );
  }
}
