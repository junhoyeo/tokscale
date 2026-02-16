import { NextRequest, NextResponse } from "next/server";
import { getUserRank, type Period, type SortBy } from "@/lib/leaderboard/getLeaderboard";

export const revalidate = 60;

const VALID_PERIODS: Period[] = ["all", "month", "week"];
const VALID_SORT_BY: SortBy[] = ["tokens", "cost"];

export async function GET(
  request: NextRequest,
  { params }: { params: Promise<{ username: string }> }
) {
  try {
    const { username } = await params;

    if (!username || !/^[a-zA-Z0-9-]{1,39}$/.test(username)) {
      return NextResponse.json(
        { error: "Invalid username format" },
        { status: 400 }
      );
    }

    const { searchParams } = new URL(request.url);
    const periodParam = searchParams.get("period") || "all";
    const period: Period = VALID_PERIODS.includes(periodParam as Period)
      ? (periodParam as Period)
      : "all";

    const sortByParam = searchParams.get("sortBy") || "tokens";
    const sortBy: SortBy = VALID_SORT_BY.includes(sortByParam as SortBy)
      ? (sortByParam as SortBy)
      : "tokens";

    const userRank = await getUserRank(username, period, sortBy);

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
