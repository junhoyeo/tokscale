import { NextResponse } from "next/server";
import { getLeaderboardData } from "@/lib/leaderboard/getLeaderboard";
import type { Period, SortBy } from "@/lib/leaderboard/types";

export const revalidate = 60;

const VALID_PERIODS: Period[] = ["all", "month", "week"];
const VALID_SORT_BY: SortBy[] = ["tokens", "cost"];

function parseIntSafe(value: string | null, defaultValue: number): number {
  if (!value) return defaultValue;
  const parsed = Number(value);
  return Number.isFinite(parsed) ? Math.floor(parsed) : defaultValue;
}

export async function GET(request: Request) {
  try {
    const { searchParams } = new URL(request.url);

    const periodParam = searchParams.get("period") || "all";
    const period: Period = VALID_PERIODS.includes(periodParam as Period)
      ? (periodParam as Period)
      : "all";

    const sortByParam = searchParams.get("sortBy") || "tokens";
    const sortBy: SortBy = VALID_SORT_BY.includes(sortByParam as SortBy)
      ? (sortByParam as SortBy)
      : "tokens";

    const page = Math.max(1, parseIntSafe(searchParams.get("page"), 1));
    const limit = Math.min(100, Math.max(1, parseIntSafe(searchParams.get("limit"), 50)));

    const data = await getLeaderboardData(period, page, limit, sortBy);

    return NextResponse.json(data);
  } catch (error) {
    console.error("Leaderboard error:", error);
    return NextResponse.json(
      { error: "Failed to fetch leaderboard" },
      { status: 500 }
    );
  }
}
