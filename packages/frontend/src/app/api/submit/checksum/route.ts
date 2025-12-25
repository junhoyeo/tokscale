import { NextResponse } from "next/server";
import { db, apiTokens, users, submissions, dailyBreakdown } from "@/lib/db";
import { eq } from "drizzle-orm";
import { hashSourceBreakdown, type SourceBreakdownData } from "@/lib/db/helpers";

export type ChecksumResponse = Record<string, Record<string, string>>;

export async function GET(request: Request) {
  try {
    const authHeader = request.headers.get("Authorization");
    if (!authHeader?.startsWith("Bearer ")) {
      return NextResponse.json(
        { error: "Missing or invalid Authorization header" },
        { status: 401 }
      );
    }

    const token = authHeader.slice(7);

    const [tokenRecord] = await db
      .select({
        userId: apiTokens.userId,
        expiresAt: apiTokens.expiresAt,
      })
      .from(apiTokens)
      .innerJoin(users, eq(apiTokens.userId, users.id))
      .where(eq(apiTokens.token, token))
      .limit(1);

    if (!tokenRecord) {
      return NextResponse.json({ error: "Invalid API token" }, { status: 401 });
    }

    if (tokenRecord.expiresAt && tokenRecord.expiresAt < new Date()) {
      return NextResponse.json({ error: "API token has expired" }, { status: 401 });
    }

    const [existingSubmission] = await db
      .select({ id: submissions.id })
      .from(submissions)
      .where(eq(submissions.userId, tokenRecord.userId))
      .limit(1);

    if (!existingSubmission) {
      return NextResponse.json({ checksums: {} });
    }

    const days = await db
      .select({
        date: dailyBreakdown.date,
        sourceBreakdown: dailyBreakdown.sourceBreakdown,
      })
      .from(dailyBreakdown)
      .where(eq(dailyBreakdown.submissionId, existingSubmission.id));

    const checksums: ChecksumResponse = {};

    for (const day of days) {
      if (day.sourceBreakdown) {
        checksums[day.date] = {};
        for (const [sourceName, sourceData] of Object.entries(day.sourceBreakdown)) {
          checksums[day.date][sourceName] = hashSourceBreakdown(sourceData as SourceBreakdownData);
        }
      }
    }

    return NextResponse.json({ checksums });
  } catch (error) {
    console.error("Checksum error:", error);
    return NextResponse.json(
      { error: "Internal server error" },
      { status: 500 }
    );
  }
}
