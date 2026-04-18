import { NextResponse } from "next/server";
import { authenticateSubmissionReviewOperator } from "../../../../../lib/submissionReviewAuth";
import { revalidateSubmissionPublicCaches } from "../../../../../lib/leaderboard/publicCacheInvalidation";
import {
  adjudicateSubmissionReview,
  getSubmissionReviewArtifact,
  REVIEW_ARTIFACT_RESULT_KIND,
} from "../../../../../lib/submissionReviews";
import { SUBMISSION_TRUST_STATE } from "../../../../../lib/validation/submissionTrust";
import { readBoundedRequestBody } from "@/lib/http/requestBody";

interface RouteParams {
  params: Promise<{ reviewId: string }>;
}

interface AdjudicationBody {
  trustState?: string;
  reviewNote?: string;
}

const MAX_ADJUDICATION_BODY_BYTES = 16 * 1024;
const UUID_PATTERN =
  /^[0-9a-f]{8}-[0-9a-f]{4}-[1-8][0-9a-f]{3}-[89ab][0-9a-f]{3}-[0-9a-f]{12}$/i;


function isAdjudicationTrustState(
  value: string | undefined
): value is
  | typeof SUBMISSION_TRUST_STATE.TRUSTED
  | typeof SUBMISSION_TRUST_STATE.REJECTED {
  return (
    value === SUBMISSION_TRUST_STATE.TRUSTED ||
    value === SUBMISSION_TRUST_STATE.REJECTED
  );
}

export async function GET(request: Request, { params }: RouteParams) {
  try {
    const operator = authenticateSubmissionReviewOperator(request);
    if (!operator) {
      return NextResponse.json({ error: "Not authenticated" }, { status: 401 });
    }

    const { reviewId } = await params;
    if (!UUID_PATTERN.test(reviewId)) {
      return NextResponse.json({ error: "Invalid review ID" }, { status: 400 });
    }
    const artifact = await getSubmissionReviewArtifact(reviewId);

    if (!artifact) {
      return NextResponse.json({ error: "Submission review not found" }, { status: 404 });
    }

    return NextResponse.json({ review: artifact });
  } catch (error) {
    console.error("Submission review detail error:", error);
    return NextResponse.json(
      { error: "Failed to load submission review" },
      { status: 500 }
    );
  }
}

export async function PATCH(request: Request, { params }: RouteParams) {
  try {
    const operator = authenticateSubmissionReviewOperator(request);
    if (!operator) {
      return NextResponse.json({ error: "Not authenticated" }, { status: 401 });
    }

    const { reviewId } = await params;
    if (!UUID_PATTERN.test(reviewId)) {
      return NextResponse.json({ error: "Invalid review ID" }, { status: 400 });
    }

    const declaredBodyBytes = Number(request.headers.get("Content-Length"));
    if (
      Number.isFinite(declaredBodyBytes) &&
      declaredBodyBytes > MAX_ADJUDICATION_BODY_BYTES
    ) {
      return NextResponse.json(
        { error: "Adjudication body is too large" },
        { status: 413 }
      );
    }

    let body: AdjudicationBody;
    try {
      const rawBody = await readBoundedRequestBody(
        request,
        MAX_ADJUDICATION_BODY_BYTES
      );
      if (rawBody == null) {
        return NextResponse.json(
          { error: "Adjudication body is too large" },
          { status: 413 }
        );
      }
      const parsedBody: unknown = JSON.parse(rawBody);
      if (
        parsedBody === null ||
        typeof parsedBody !== "object" ||
        Array.isArray(parsedBody)
      ) {
        return NextResponse.json(
          { error: "Adjudication body must be a JSON object" },
          { status: 400 }
        );
      }
      body = parsedBody as AdjudicationBody;
    } catch {
      return NextResponse.json({ error: "Invalid JSON body" }, { status: 400 });
    }

    if (!isAdjudicationTrustState(body.trustState)) {
      return NextResponse.json(
        {
          error: "trustState must be 'trusted' or 'rejected'",
        },
        { status: 400 }
      );
    }

    if (
      body.reviewNote !== undefined &&
      (typeof body.reviewNote !== "string" || body.reviewNote.length > 2_000)
    ) {
      return NextResponse.json(
        { error: "reviewNote must be a string of at most 2000 characters" },
        { status: 400 }
      );
    }

    const result = await adjudicateSubmissionReview({
      reviewId,
      trustState: body.trustState,
      reviewedByUsername: operator.username,
      reviewNote: body.reviewNote,
    });

    if (result.kind === REVIEW_ARTIFACT_RESULT_KIND.NOT_FOUND) {
      return NextResponse.json(
        { error: "Submission review not found" },
        { status: 404 }
      );
    }

    if (result.kind === REVIEW_ARTIFACT_RESULT_KIND.CONFLICT) {
      return NextResponse.json(
        {
          error: "Submission review has already been adjudicated",
          trustState: result.currentTrustState,
        },
        { status: 409 }
      );
    }

    if (result.competitiveWriteApplied) {
      try {
        await revalidateSubmissionPublicCaches(
          result.artifact.user.id,
          result.artifact.user.username,
          result.affectedCompetitiveUsernames
        );
      } catch (cacheError) {
        console.error(
          "Review adjudication cache invalidation failed:",
          cacheError
        );
      }
    }

    return NextResponse.json({
      success: true,
      review: result.artifact,
      competitiveWriteApplied: result.competitiveWriteApplied,
    });
  } catch (error) {
    console.error("Submission review adjudication error:", error);
    return NextResponse.json(
      { error: "Failed to adjudicate submission review" },
      { status: 500 }
    );
  }
}
