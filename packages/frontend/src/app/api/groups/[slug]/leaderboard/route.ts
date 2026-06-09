import { NextResponse } from "next/server";
import { getSessionFromRequest } from "@/lib/auth/requestSession";
import { getGroupLeaderboardData } from "@/lib/groups/getGroupLeaderboard";
import { getGroupMembership } from "@/lib/groups/permissions";
import { getGroupBySlug } from "@/lib/groups/queries";
import type { Period, SortBy } from "@/lib/leaderboard/types";

const VALID_PERIODS: Period[] = ["all", "month", "week"];
const VALID_SORT_BY: SortBy[] = ["tokens", "cost"];

function parseIntSafe(value: string | null, defaultValue: number): number {
  if (!value) return defaultValue;
  const parsed = Number(value);
  return Number.isFinite(parsed) ? Math.floor(parsed) : defaultValue;
}

export async function GET(
  request: Request,
  { params }: { params: Promise<{ slug: string }> }
) {
  try {
    const { slug } = await params;
    const group = await getGroupBySlug(slug);

    if (!group) {
      return NextResponse.json({ error: "Group not found" }, { status: 404 });
    }

    if (!group.isPublic) {
      const session = await getSessionFromRequest(request);
      if (!session) {
        return NextResponse.json({ error: "Group not found" }, { status: 404 });
      }

      const membership = await getGroupMembership(group.id, session.id);
      if (!membership) {
        return NextResponse.json({ error: "Group not found" }, { status: 404 });
      }
    }

    const { searchParams } = new URL(request.url);
    const periodParam = searchParams.get("period") || "all";
    const sortByParam = searchParams.get("sortBy") || "tokens";
    const period: Period = VALID_PERIODS.includes(periodParam as Period)
      ? (periodParam as Period)
      : "all";
    const sortBy: SortBy = VALID_SORT_BY.includes(sortByParam as SortBy)
      ? (sortByParam as SortBy)
      : "tokens";
    const page = Math.max(1, parseIntSafe(searchParams.get("page"), 1));
    const limit = Math.min(100, Math.max(1, parseIntSafe(searchParams.get("limit"), 50)));
    const search = (searchParams.get("search") || "").trim();

    const data = await getGroupLeaderboardData(
      group.id,
      period,
      page,
      limit,
      sortBy,
      search
    );

    return NextResponse.json(data);
  } catch (error) {
    console.error("Group leaderboard error:", error);
    return NextResponse.json(
      { error: "Failed to fetch group leaderboard" },
      { status: 500 }
    );
  }
}
