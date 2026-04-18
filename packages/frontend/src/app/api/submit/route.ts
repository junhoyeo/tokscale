import { NextResponse } from "next/server";
import { revalidateTag } from "next/cache";
import { db, apiTokens, submissionReviews } from "@/lib/db";
import { and, eq, sql } from "drizzle-orm";
import {
  validateSubmission,
  generateSubmissionHash,
} from "@/lib/validation/submission";
import { authenticatePersonalToken } from "@/lib/auth/personalTokens";
import {
  assessSubmissionTrust,
  subsetSubmissionByDates,
  SUBMISSION_TRUST_STATE,
} from "@/lib/validation/submissionTrust";
import { getBearerToken } from "../../../lib/auth/bearerToken";
import { normalizeUsernameCacheKey, revalidateUsernamePaths } from "@/lib/db/usernameLookup";
import { revalidateUserGroupLeaderboards } from "@/lib/groups/cache";
import {
  applyTrustedSubmission,
  getSubmitDevice,
} from "@/lib/submissionPersistence";
import { revalidateLeaderboardPublicSurfacePaths } from "../../../lib/leaderboard/publicSurfaceRevalidation";
import { readBoundedRequestBody } from "@/lib/http/requestBody";

const MAX_PENDING_SUBMISSION_REVIEWS = 20;
const MAX_SUBMISSION_BODY_BYTES = 5 * 1024 * 1024;
const MAX_MCP_SERVERS = 100;
const MAX_MCP_SERVER_NAME_LENGTH = 128;
const CONTROL_CHARACTER_PATTERN = /[\u0000-\u001f\u007f-\u009f]/;

class PendingReviewLimitError extends Error {}


function normalizeSubmissionData(data: unknown): void {
  if (!data || typeof data !== "object") return;
  const obj = data as Record<string, unknown>;
  if (!Array.isArray(obj.contributions)) return;

  for (const contribution of obj.contributions) {
    if (!contribution || typeof contribution !== "object") continue;
    const day = contribution as Record<string, unknown>;
    // Handle both legacy "sources" and new "clients" formats
    const items = Array.isArray(day.sources)
      ? day.sources
      : Array.isArray(day.clients)
      ? day.clients
      : null;
    if (!items) continue;
    for (const entry of items) {
      if (!entry || typeof entry !== "object") continue;
      const s = entry as Record<string, unknown>;
      if (s.modelId == null || typeof s.modelId !== "string") {
        s.modelId = "unknown";
      } else {
        const trimmed = s.modelId.trim();
        s.modelId = trimmed === "" ? "unknown" : trimmed;
      }
    }
  }
}


/**
 * POST /api/submit
 * Submit token usage data from CLI
 * 
 * IMPLEMENTS CLIENT-LEVEL MERGE:
 * - Only updates clients present in submission
 * - Preserves data for clients NOT in submission
 * - Recalculates totals from dailyBreakdown
 *
 * Headers:
 *   Authorization: Bearer <api_token>
 *
 * Body: TokenContributionData JSON
 */
export async function POST(request: Request) {
  try {
    // ========================================
    // STEP 1: Authentication
    // ========================================
    const token = getBearerToken(request.headers.get("Authorization"));
    if (!token) {
      return NextResponse.json(
        { error: "Missing or invalid Authorization header" },
        { status: 401 }
      );
    }

    const authResult = await authenticatePersonalToken(token, {
      touchLastUsedAt: false,
    });

    if (authResult.status === "invalid") {
      return NextResponse.json({ error: "Invalid API token" }, { status: 401 });
    }

    if (authResult.status === "expired") {
      return NextResponse.json({ error: "API token has expired" }, { status: 401 });
    }

    const tokenRecord = authResult;

    // ========================================
    // STEP 2: Parse and Validate
    // ========================================
    const declaredBodyBytes = Number(request.headers.get("Content-Length"));
    if (
      Number.isFinite(declaredBodyBytes) &&
      declaredBodyBytes > MAX_SUBMISSION_BODY_BYTES
    ) {
      return NextResponse.json(
        { error: "Submission body is too large" },
        { status: 413 }
      );
    }

    let rawData: unknown;
    try {
      const rawBody = await readBoundedRequestBody(
        request,
        MAX_SUBMISSION_BODY_BYTES
      );
      if (rawBody == null) {
        return NextResponse.json(
          { error: "Submission body is too large" },
          { status: 413 }
        );
      }
      rawData = JSON.parse(rawBody);
    } catch {
      return NextResponse.json({ error: "Invalid JSON body" }, { status: 400 });
    }

    normalizeSubmissionData(rawData);

    let mcpServers: string[] | null = null;
    if (
      rawData != null &&
      typeof rawData === "object" &&
      Array.isArray((rawData as Record<string, unknown>).mcpServers)
    ) {
      const rawServerNames = (
        (rawData as Record<string, unknown>).mcpServers as unknown[]
      );
      const serverNames = rawServerNames.filter(
        (server): server is string =>
          typeof server === "string" && server.length > 0
      );
      if (
        rawServerNames.length > MAX_MCP_SERVERS ||
        serverNames.some(
          (server) =>
            server.length > MAX_MCP_SERVER_NAME_LENGTH ||
            CONTROL_CHARACTER_PATTERN.test(server)
        )
      ) {
        return NextResponse.json(
          { error: "Invalid MCP server metadata" },
          { status: 400 }
        );
      }
      mcpServers = Array.from(new Set(serverNames));
    }

    const validation = validateSubmission(rawData);

    if (!validation.valid || !validation.data) {
      return NextResponse.json(
        {
          error: "Validation failed",
          details: validation.errors,
          trustState: SUBMISSION_TRUST_STATE.REJECTED,
        },
        { status: 400 }
      );
    }

    const validatedData = validation.data;
    const warnings = [...validation.warnings];

    if (validatedData.contributions.length === 0) {
      return NextResponse.json(
        {
          error: "No contribution data to submit",
          trustState: SUBMISSION_TRUST_STATE.REJECTED,
        },
        { status: 400 }
      );
    }

    const trustAssessment = assessSubmissionTrust(validatedData);
    warnings.push(...trustAssessment.warnings);
    if (trustAssessment.trustState === SUBMISSION_TRUST_STATE.REJECTED) {
      return NextResponse.json(
        {
          error: "Submission rejected by trust policy",
          details: trustAssessment.errors,
          trustState: trustAssessment.trustState,
          errorCodes: trustAssessment.rejectionReasonCodes,
        },
        { status: 400 }
      );
    }

    const submitDevice = getSubmitDevice(validatedData);
    const reviewDates = new Set(trustAssessment.reviewDates);
    const trustedDates = new Set(
      validatedData.contributions
        .filter((day) => !reviewDates.has(day.date))
        .map((day) => day.date)
    );
    const reviewData = subsetSubmissionByDates(validatedData, reviewDates);
    const trustedData = reviewData
      ? subsetSubmissionByDates(validatedData, trustedDates)
      : validatedData;
    const data = trustedData ?? validatedData;


    // ========================================
    // STEP 3: DATABASE OPERATIONS IN TRANSACTION
    // ========================================
    const result = await db.transaction(async (tx) => {
      await tx
        .update(apiTokens)
        .set({ lastUsedAt: new Date() })
        .where(eq(apiTokens.id, tokenRecord.tokenId));

      let reviewId: string | null = null;
      if (reviewData) {
        const reviewPayload = {
          ...(reviewData as unknown as Record<string, unknown>),
          ...(mcpServers && mcpServers.length > 0 ? { mcpServers } : {}),
        };
        const reviewHash = generateSubmissionHash(reviewData);
        await tx.execute(
          sql`SELECT pg_advisory_xact_lock(hashtextextended(${tokenRecord.userId}::text, 0))`
        );
        const [pendingReviewStats] = await tx
          .select({
            pendingCount: sql<number>`count(*)::int`,
            matchingHashCount: sql<number>`count(*) FILTER (
              WHERE ${submissionReviews.submissionHash} = ${reviewHash}
            )::int`,
          })
          .from(submissionReviews)
          .where(
            and(
              eq(submissionReviews.userId, tokenRecord.userId),
              eq(
                submissionReviews.trustState,
                SUBMISSION_TRUST_STATE.REVIEW_REQUIRED
              )
            )
          );
        if (
          Number(pendingReviewStats.pendingCount) >=
            MAX_PENDING_SUBMISSION_REVIEWS &&
          Number(pendingReviewStats.matchingHashCount) === 0
        ) {
          throw new PendingReviewLimitError();
        }
        const [review] = await tx
          .insert(submissionReviews)
          .values({
            userId: tokenRecord.userId,
            submissionHash: reviewHash,
            trustState: SUBMISSION_TRUST_STATE.REVIEW_REQUIRED,
            competitiveWriteApplied: trustedData !== null,
            reasonCodes: trustAssessment.reasonCodes,
            payload: reviewPayload,
            totalTokens: reviewData.summary.totalTokens,
            totalCost: reviewData.summary.totalCost.toFixed(4),
            activeDays: reviewData.summary.activeDays,
            dateStart: reviewData.meta.dateRange.start,
            dateEnd: reviewData.meta.dateRange.end,
            sourcesUsed: reviewData.summary.clients,
            modelsUsed: reviewData.summary.models,
            cliVersion: reviewData.meta.version,
            schemaVersion: submitDevice.schemaVersion,
          })
          .onConflictDoUpdate({
            target: [
              submissionReviews.userId,
              submissionReviews.submissionHash,
            ],
            targetWhere: sql`${submissionReviews.trustState} = 'review_required'`,
            set: {
              competitiveWriteApplied: sql`${submissionReviews.competitiveWriteApplied} OR ${trustedData !== null}`,
              reasonCodes: trustAssessment.reasonCodes,
              payload: reviewPayload,
              totalTokens: reviewData.summary.totalTokens,
              totalCost: reviewData.summary.totalCost.toFixed(4),
              activeDays: reviewData.summary.activeDays,
              dateStart: reviewData.meta.dateRange.start,
              dateEnd: reviewData.meta.dateRange.end,
              sourcesUsed: reviewData.summary.clients,
              modelsUsed: reviewData.summary.models,
              cliVersion: reviewData.meta.version,
              schemaVersion: submitDevice.schemaVersion,
              updatedAt: new Date(),
            },
          })
          .returning({ id: submissionReviews.id });
        reviewId = review.id;
      }

      if (!trustedData) {
        return {
          trustState: SUBMISSION_TRUST_STATE.REVIEW_REQUIRED,
          reviewId,
          submissionId: null,
          isNewSubmission: false,
          competitiveWriteApplied: false,
          metrics: undefined,
          reviewMetrics: {
            totalTokens: reviewData!.summary.totalTokens,
            totalCost: reviewData!.summary.totalCost,
            dateRange: reviewData!.meta.dateRange,
            activeDays: reviewData!.summary.activeDays,
            clients: reviewData!.summary.clients,
          },
        };
      }

      const trustedResult = await applyTrustedSubmission(tx, {
        userId: tokenRecord.userId,
        data,
        mcpServers,
      });
      warnings.push(...trustedResult.warnings);

      return {
        trustState: trustAssessment.trustState,
        reviewId,
        competitiveWriteApplied: true,
        submissionId: trustedResult.submissionId,
        isNewSubmission: trustedResult.isNewSubmission,
        reviewMetrics: reviewData
          ? {
              totalTokens: reviewData.summary.totalTokens,
              totalCost: reviewData.summary.totalCost,
              dateRange: reviewData.meta.dateRange,
              activeDays: reviewData.summary.activeDays,
              clients: reviewData.summary.clients,
            }
          : undefined,
        metrics: trustedResult.metrics,
      };
    });

    if (result.competitiveWriteApplied) {
      const usernameCacheKey = normalizeUsernameCacheKey(tokenRecord.username);
      try {
        revalidateTag("leaderboard", "max");
        revalidateTag(`user:${usernameCacheKey}`, "max");
        revalidateTag("user-rank", "max");
        revalidateTag(`user-rank:${usernameCacheKey}`, "max");
        revalidateLeaderboardPublicSurfacePaths();
      } catch (e) {
        console.error("Public cache invalidation failed:", e);
      }

      try {
        await revalidateUserGroupLeaderboards(tokenRecord.userId);
      } catch (e) {
        console.error("Group leaderboard cache invalidation failed:", e);
      }

      try {
        revalidateUsernamePaths(tokenRecord.username);
      } catch (e) {
        console.error("Username path revalidation failed:", e);
      }
    }

    return NextResponse.json({
      success: true,
      trustState: result.trustState,
      submissionId: result.submissionId,
      reviewId: result.reviewId,
      username: tokenRecord.username,
      metrics: result.metrics,
      reviewMetrics: result.reviewMetrics,
      mode: result.competitiveWriteApplied
        ? result.isNewSubmission
          ? "create"
          : "merge"
        : "review",
      reasonCodes:
        trustAssessment.reasonCodes.length > 0
          ? trustAssessment.reasonCodes
          : undefined,
      competitiveWriteApplied: result.competitiveWriteApplied,
      warnings: warnings.length > 0 ? warnings : undefined,
    });
  } catch (error) {
    if (error instanceof PendingReviewLimitError) {
      return NextResponse.json(
        {
          error: "Pending submission review limit reached",
          trustState: SUBMISSION_TRUST_STATE.REVIEW_REQUIRED,
        },
        { status: 429 }
      );
    }
    console.error("Submit error:", error);
    return NextResponse.json(
      { error: "Internal server error" },
      { status: 500 }
    );
  }
}
