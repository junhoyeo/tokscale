import { NextRequest, NextResponse } from "next/server";
import { getUserRank } from "@/lib/leaderboard/getLeaderboard";
import type { Period, SortBy } from "@/lib/leaderboard/types";
import { isValidGitHubUsername } from "@/lib/validation/username";

export const revalidate = 60;

const VALID_PERIODS: Period[] = ["all", "month", "last-month", "week", "custom"];
const VALID_SORT_BY: SortBy[] = ["tokens", "cost", "time"];

const DATE_REGEX = /^\d{4}-\d{2}-\d{2}$/;

function isValidDateString(value: string | null): value is string {
  if (!value || !DATE_REGEX.test(value)) return false;
  const [year, month, day] = value.split("-").map(Number);
  const date = new Date(year, month - 1, day);
  return date.getFullYear() === year && date.getMonth() === month - 1 && date.getDate() === day;
}

export async function GET(
  request: NextRequest,
  { params }: { params: Promise<{ username: string }> }
) {
  try {
    const { username } = await params;

    if (!username || !isValidGitHubUsername(username)) {
      return NextResponse.json(
        { error: "Invalid username format" },
        { status: 400 }
      );
    }

    const { searchParams } = new URL(request.url);
    const periodParam = searchParams.get("period") || "all";
    let period: Period = VALID_PERIODS.includes(periodParam as Period)
      ? (periodParam as Period)
      : "all";

    const sortByParam = searchParams.get("sortBy") || "tokens";
    const sortBy: SortBy = VALID_SORT_BY.includes(sortByParam as SortBy)
      ? (sortByParam as SortBy)
      : "tokens";

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

    const userRank = await getUserRank(username, period, sortBy, customFrom, customTo);

    if (!userRank) {
      return NextResponse.json({ error: "User not found or has no submissions" }, { status: 404 });
    }

    return NextResponse.json(userRank);
  } catch (error) {
    console.error("Error fetching user rank:", error);
    return NextResponse.json(
      { error: "Internal server error" },
      { status: 500 }
    );
  }
}
