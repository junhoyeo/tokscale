import { NextResponse } from "next/server";
import { db, apiTokens, users, submissions, dailyBreakdown } from "@/lib/db";
import { eq } from "drizzle-orm";
import {
  validateSubmission,
  extractMetrics,
  generateSubmissionHash,
  type SubmissionData,
} from "@/lib/validation/submission";

/**
 * POST /api/submit
 * Submit token usage data from CLI
 *
 * Headers:
 *   Authorization: Bearer <api_token>
 *
 * Body: TokenContributionData JSON
 */
export async function POST(request: Request) {
  try {
    // Step 1: Authenticate via API token
    const authHeader = request.headers.get("Authorization");
    if (!authHeader?.startsWith("Bearer ")) {
      return NextResponse.json(
        { error: "Missing or invalid Authorization header" },
        { status: 401 }
      );
    }

    const token = authHeader.slice(7);

    // Find the API token and associated user
    const [tokenRecord] = await db
      .select({
        tokenId: apiTokens.id,
        userId: apiTokens.userId,
        username: users.username,
        expiresAt: apiTokens.expiresAt,
      })
      .from(apiTokens)
      .innerJoin(users, eq(apiTokens.userId, users.id))
      .where(eq(apiTokens.token, token))
      .limit(1);

    if (!tokenRecord) {
      return NextResponse.json({ error: "Invalid API token" }, { status: 401 });
    }

    // Check if token is expired
    if (tokenRecord.expiresAt && tokenRecord.expiresAt < new Date()) {
      return NextResponse.json({ error: "API token has expired" }, { status: 401 });
    }

    // Update last used timestamp
    await db
      .update(apiTokens)
      .set({ lastUsedAt: new Date() })
      .where(eq(apiTokens.id, tokenRecord.tokenId));

    // Step 2: Parse and validate submission data
    let data: SubmissionData;
    try {
      data = await request.json();
    } catch {
      return NextResponse.json(
        { error: "Invalid JSON body" },
        { status: 400 }
      );
    }

    const validation = validateSubmission(data);

    if (!validation.valid) {
      return NextResponse.json(
        {
          error: "Validation failed",
          details: validation.errors,
        },
        { status: 400 }
      );
    }

    // Step 3: Extract metrics and create submission
    const metrics = extractMetrics(data);
    const submissionHash = generateSubmissionHash(data);

    // Delete all previous submissions for this user (replace mode)
    await db.delete(submissions).where(eq(submissions.userId, tokenRecord.userId));

    // Step 4: Insert submission
    const [newSubmission] = await db
      .insert(submissions)
      .values({
        userId: tokenRecord.userId,
        totalTokens: metrics.totalTokens,
        totalCost: metrics.totalCost.toFixed(4),
        inputTokens: metrics.inputTokens,
        outputTokens: metrics.outputTokens,
        cacheCreationTokens: metrics.cacheCreationTokens,
        cacheReadTokens: metrics.cacheReadTokens,
        dateStart: metrics.dateStart,
        dateEnd: metrics.dateEnd,
        sourcesUsed: metrics.sourcesUsed,
        modelsUsed: metrics.modelsUsed,
        status: "verified",
        cliVersion: data.meta.version,
        submissionHash,
      })
      .returning({ id: submissions.id });

    // Step 5: Insert daily breakdown (batch insert)
    if (data.contributions.length > 0) {
      const breakdownRecords = data.contributions.map((day) => ({
        submissionId: newSubmission.id,
        date: day.date,
        tokens: day.totals.tokens,
        cost: day.totals.cost.toFixed(4),
        inputTokens: day.tokenBreakdown.input,
        outputTokens: day.tokenBreakdown.output,
        sourceBreakdown: Object.fromEntries(
          day.sources.map((s) => [
            s.source,
            {
              tokens: s.tokens.input + s.tokens.output,
              cost: s.cost,
              modelId: s.modelId,
              input: s.tokens.input,
              output: s.tokens.output,
              cacheRead: s.tokens.cacheRead,
              cacheWrite: s.tokens.cacheWrite,
              messages: s.messages,
            },
          ])
        ),
        modelBreakdown: Object.fromEntries(
          day.sources.map((s) => [s.modelId, s.tokens.input + s.tokens.output])
        ),
      }));

      await db.insert(dailyBreakdown).values(breakdownRecords);
    }

    // Step 6: Return success response
    return NextResponse.json({
      success: true,
      submissionId: newSubmission.id,
      username: tokenRecord.username,
      metrics: {
        totalTokens: metrics.totalTokens,
        totalCost: metrics.totalCost,
        dateRange: {
          start: metrics.dateStart,
          end: metrics.dateEnd,
        },
        activeDays: data.summary.activeDays,
        sources: metrics.sourcesUsed,
      },
      warnings: validation.warnings.length > 0 ? validation.warnings : undefined,
    });
  } catch (error) {
    console.error("Submit error:", error);
    return NextResponse.json(
      { error: "Internal server error" },
      { status: 500 }
    );
  }
}
